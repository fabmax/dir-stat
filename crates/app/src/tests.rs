//! App-level tests without rendering (design §10.3): scans are simulated by feeding
//! `Message::Scan` updates by hand; returned tasks are ignored.

use std::io::ErrorKind;
use std::sync::{Arc, Mutex, OnceLock};

use dirstat_core::catalog::{Engine, Resolver};
use dirstat_core::scan::{Batch, Progress, StartError, Summary, Update};
use dirstat_core::tree::NodeId;
use dirstat_core::tree::testing::{TreeBuilder, dir};

use crate::Cli;
use crate::app::{App, Env, Message, Screen};
use crate::prefs::{MemoryPrefs, Prefs};
use crate::results::{Phase, Results};

fn engine() -> Arc<Engine> {
    static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();
    ENGINE
        .get_or_init(|| Arc::new(Engine::new(&Resolver::from_env())))
        .clone()
}

fn app() -> App {
    let env = Env {
        engine: engine(),
        prefs_store: Box::new(MemoryPrefs(Mutex::new(Prefs::default()))),
        cards: Vec::new(),
        mounts: Vec::new(),
        fda: None,
        system_name: "Test Disk".into(),
        home: None,
        terminal: None,
        guards: std::sync::Arc::new(dirstat_core::safety::Guards::for_home(
            "/nonexistent-home".into(),
            Default::default(),
        )),
        trasher: std::sync::Arc::new(dirstat_core::trash::RecordingTrasher::into_dir(
            std::env::temp_dir(),
        )),
        git: None,
    };
    App::new(env, Cli::default()).0
}

fn sample() -> TreeBuilder {
    TreeBuilder::new(
        "/r",
        dir("r")
            .small_files(2, 8000)
            .child(dir("a").child(dir("deep")))
            .child(dir("b")),
    )
}

fn batch(listings: Vec<dirstat_core::scan::DirListing>) -> Update {
    Update::Batch(Batch {
        listings,
        progress: Progress::default(),
    })
}

fn results(app: &App) -> &Results {
    match &app.screen {
        Screen::Results(r) => r,
        Screen::Start(_) => panic!("expected the Results screen"),
    }
}

#[test]
fn stop_is_final_even_if_finished_never_arrives() {
    let mut app = app();
    let builder = sample();
    let id = app.start_simulated("/r");
    let _ = app.update(Message::Scan(id, Update::Started(builder.root_info())));
    let mut listings = builder.listings();
    let late = listings.split_off(2);
    let _ = app.update(Message::Scan(id, batch(listings)));
    assert_eq!(results(&app).phase, Phase::Scanning);

    let _ = app.update(Message::Stop);
    let r = results(&app);
    assert_eq!(r.phase, Phase::Stopped);
    assert!(r.tree.is_stopped());
    assert!(r.tree.is_lower_bound(NodeId::ROOT), "partial sizes read ≥");
    assert!(
        !r.is_scanning() && r.can_rescan(),
        "Rescan works without Finished"
    );
    assert!(r.draining.is_some());

    // Late batches of the draining scan still add data; its Finished doesn't change the phase.
    let _ = app.update(Message::Scan(id, batch(late)));
    assert!(results(&app).tree.node(NodeId::ROOT).is_done());
    let _ = app.update(Message::Scan(
        id,
        Update::Finished(Summary {
            cancelled: true,
            ..Summary::default()
        }),
    ));
    let r = results(&app);
    assert_eq!(r.phase, Phase::Stopped);
    assert!(r.draining.is_none() && r.summary.is_some());
}

#[test]
fn stop_while_opening_returns_to_the_start_screen() {
    let mut app = app();
    app.start_simulated("/r");
    let _ = app.update(Message::Stop);
    assert!(matches!(app.screen, Screen::Start(_)));
}

#[test]
fn a_failed_full_scan_shows_the_banner() {
    let mut app = app();
    let id = app.start_simulated("/nope");
    let error = StartError {
        kind: ErrorKind::NotFound,
        message: "No such file or directory (os error 2)".into(),
    };
    let _ = app.update(Message::Scan(id, Update::Failed(error)));
    let Screen::Start(start) = &app.screen else {
        panic!("expected the start screen")
    };
    assert_eq!(
        start.banner.as_deref(),
        Some("Can't scan “/nope”: No such folder.")
    );
}

#[test]
fn updates_of_a_replaced_scan_are_ignored() {
    let mut app = app();
    let builder = sample();
    let old = app.start_simulated("/r");
    let new = app.start_simulated("/r");
    assert_ne!(old, new);
    let _ = app.update(Message::Scan(new, Update::Started(builder.root_info())));
    let _ = app.update(Message::Scan(old, batch(builder.listings())));
    let r = results(&app);
    assert_eq!(r.tree.listed(), 0, "the old scan's batch was dropped");
    let _ = app.update(Message::Scan(old, Update::Finished(Summary::default())));
    assert_eq!(results(&app).phase, Phase::Scanning);
}

#[test]
fn a_complete_scan_and_its_recent_entry() {
    let mut app = app();
    let builder = sample();
    let id = app.start_simulated("/r");
    let _ = app.update(Message::Scan(id, Update::Started(builder.root_info())));
    let _ = app.update(Message::Scan(id, batch(builder.listings())));
    let _ = app.update(Message::Scan(id, Update::Finished(Summary::default())));
    let r = results(&app);
    assert_eq!(r.phase, Phase::Complete);
    assert!(r.can_rescan());
    assert_eq!(
        app.prefs()
            .recents
            .first()
            .map(|(p, _)| p.to_str().unwrap()),
        Some("/r")
    );
}

#[test]
fn escape_stops_a_scan() {
    let mut app = app();
    let builder = sample();
    let id = app.start_simulated("/r");
    let _ = app.update(Message::Scan(id, Update::Started(builder.root_info())));
    let _ = app.update(Message::Key(crate::keys::KeyAction::Escape));
    assert_eq!(results(&app).phase, Phase::Stopped);
}

fn completed(app: &mut App, builder: &TreeBuilder) -> crate::results::scan_task::ScanId {
    let id = app.start_simulated("/r");
    let _ = app.update(Message::Scan(id, Update::Started(builder.root_info())));
    let _ = app.update(Message::Scan(id, batch(builder.listings())));
    let _ = app.update(Message::Scan(id, Update::Finished(Summary::default())));
    id
}

#[test]
fn zoom_in_out_and_back_through_the_app() {
    use crate::keys::KeyAction::*;
    use crate::results::rows::RowKey;
    let mut app = app();
    let builder = TreeBuilder::new(
        "/r",
        dir("r")
            .child(dir("big").small_files(1, 900_000).child(dir("inner")))
            .child(dir("b")),
    );
    completed(&mut app, &builder);
    let big = RowKey::Dir(NodeId(1));
    let _ = app.update(Message::Key(Down));
    assert_eq!(results(&app).view.selection, Some(big));
    let _ = app.update(Message::Key(Enter));
    assert_eq!(results(&app).view.zoom, NodeId(1));
    assert_eq!(app.title(), "big — dir-stat");
    let _ = app.update(Message::Key(Back));
    let r = results(&app);
    assert_eq!(
        (r.view.zoom, r.view.selection),
        (NodeId::ROOT, Some(big)),
        "Back restores the selection"
    );
    let _ = app.update(Message::Key(Forward));
    let _ = app.update(Message::Key(ZoomOut));
    assert_eq!(
        results(&app).view.selection,
        Some(big),
        "zooming out selects the folder we came from"
    );
    let _ = app.update(Message::ZoomTo(NodeId(1)));
    let _ = app.update(Message::ZoomTo(NodeId::ROOT));
    assert_eq!(
        results(&app).view.selection,
        Some(big),
        "a breadcrumb click too"
    );
}

#[test]
fn copy_path_toasts_and_stale_listings_are_ignored() {
    use crate::results::rows::{RowKey, SmallFiles};
    let mut app = app();
    let builder = TreeBuilder::new("/r", dir("r").child(dir("a").small_files(5, 5000)));
    let id = completed(&mut app, &builder);
    let _ = app.update(Message::RowPressed(RowKey::Dir(NodeId(1))));
    let _ = app.update(Message::CopySelection);
    assert!(app.toast_text().is_some_and(|t| t == "Copied path"));

    let _ = app.update(Message::ToggleExpand(RowKey::Dir(NodeId(1))));
    let _ = app.update(Message::ToggleExpand(RowKey::Aggregate(NodeId(1))));
    let stale = crate::results::scan_task::ScanId(id.0 + 100);
    let _ = app.update(Message::SmallFilesListed(stale, NodeId(1), Ok(Vec::new())));
    let small = &results(&app).view.open_aggregates[&NodeId(1)].small;
    assert_eq!(
        small,
        &SmallFiles::Loading,
        "a listing for another scan is dropped"
    );
    let _ = app.update(Message::SmallFilesListed(id, NodeId(1), Err("gone".into())));
    let small = &results(&app).view.open_aggregates[&NodeId(1)].small;
    assert_eq!(small, &SmallFiles::Failed("gone".into()));
}

#[test]
fn errors_popover_and_full_disk_access_bar() {
    use dirstat_core::scan::ErrorClass;
    let mut app = app();
    let builder = TreeBuilder::new(
        "/r",
        dir("r")
            .child(dir("mail").error(ErrorClass::PrivacyBlocked))
            .child(dir("safari").error(ErrorClass::PrivacyBlocked))
            .child(dir("ok")),
    );
    let id = app.start_simulated("/r");
    let _ = app.update(Message::Scan(id, Update::Started(builder.root_info())));
    let _ = app.update(Message::Scan(id, batch(builder.listings())));
    assert_eq!(
        results(&app).fda_bar,
        cfg!(target_os = "macos"),
        "the first EPERM raises the bar"
    );

    // Stop: the popover still lists the errors.
    let _ = app.update(Message::Stop);
    let _ = app.update(Message::ToggleErrorsPopover);
    assert!(results(&app).errors_popover);
    assert_eq!(
        crate::results::errors::headline(&results(&app).tree).as_deref(),
        Some("2 folders couldn't be read")
    );
    let _ = app.update(Message::CopyErrorList);
    assert_eq!(app.toast_text(), Some("Copied the list"));
    let _ = app.update(Message::Key(crate::keys::KeyAction::Escape));
    assert!(
        !results(&app).errors_popover,
        "Esc closes the popover first"
    );

    // "Not Now" holds for the session, also for the next scan.
    let _ = app.update(Message::DismissFdaBar);
    let id = app.start_simulated("/r");
    let _ = app.update(Message::Scan(id, Update::Started(builder.root_info())));
    let _ = app.update(Message::Scan(id, batch(builder.listings())));
    assert!(!app.fda_bar_visible());
}

mod trash_flow {
    use super::*;
    use crate::keys::KeyAction;
    use crate::results::rows::RowKey;
    use crate::results::selection_bar::{Action, contents};
    use crate::results::trash::{Focus, Modal};
    use dirstat_core::catalog::Verdict;
    use dirstat_core::scan::Skip;
    use dirstat_core::trash::{TrashError, TrashJobId, TrashOutcome};
    use dirstat_core::tree::NodeFlags;

    /// root: big (100 MB), cache (a Low candidate, 2 MB), mnt-holder (a mount inside).
    fn scanned(app: &mut App) -> (NodeId, NodeId, NodeId) {
        let low = Verdict::Target {
            rule: engine().rule_by_id("node-modules").unwrap(),
            by_default: false,
        };
        let builder = TreeBuilder::new(
            "/r",
            dir("r")
                .child(dir("big").small_files(1, 100_000_000))
                .child(dir("node_modules").verdict(low).small_files(5, 2_000_000))
                .child(dir("holder").child(dir("mnt").skip(Skip::MountPoint))),
        );
        completed(app, &builder);
        let tree = &results(app).tree;
        let find = |p: &str| tree.find_path(std::path::Path::new(p)).unwrap();
        (find("/r/big"), find("/r/node_modules"), find("/r/holder"))
    }

    fn select(app: &mut App, id: NodeId) {
        let _ = app.update(Message::RowPressed(RowKey::Dir(id)));
    }

    #[test]
    fn nothing_moves_before_confirm_and_return_cancels_risky_items() {
        let mut app = app();
        let (_, cache, _) = scanned(&mut app);
        select(&mut app, cache);
        let _ = app.update(Message::Key(KeyAction::Trash));
        let Some(Modal::Confirm { plan, focus }) = app.modal() else {
            panic!("{:?}", app.modal())
        };
        assert!(
            !plan.default_is_trash,
            "Low is Review first: Cancel is the default"
        );
        assert_eq!(*focus, Focus::Cancel);
        assert!(
            results(&app).trash_jobs.is_empty(),
            "no job before confirming"
        );

        let _ = app.update(Message::Key(KeyAction::Enter));
        assert!(app.modal().is_none(), "Return cancelled");
        assert!(results(&app).trash_jobs.is_empty());
        assert!(
            !results(&app)
                .tree
                .node(cache)
                .has_flag(NodeFlags::TRASH_PENDING)
        );

        // Tab to the destructive button, Space presses it.
        let _ = app.update(Message::Key(KeyAction::Trash));
        let _ = app.update(Message::Key(KeyAction::Tab));
        let _ = app.update(Message::Key(KeyAction::Space));
        assert!(app.modal().is_none());
        assert_eq!(results(&app).trash_jobs.len(), 1);
        assert!(
            results(&app)
                .tree
                .node(cache)
                .has_flag(NodeFlags::TRASH_PENDING),
            "Moving to Trash…"
        );
    }

    #[test]
    fn success_accounts_at_once_and_the_row_leaves_at_the_next_reorder() {
        let mut app = app();
        let (_, cache, _) = scanned(&mut app);
        let root_before = results(&app).tree.node(NodeId::ROOT).total().alloc;
        let cache_size = results(&app).tree.node(cache).total().alloc;
        select(&mut app, cache);
        let _ = app.update(Message::RequestTrash);
        let _ = app.update(Message::ConfirmTrash);
        let generation = results(&app).tree_gen;
        let path = results(&app).tree.path_of(cache);
        let _ = app.update(Message::TrashFinished(
            TrashJobId(1),
            generation,
            vec![TrashOutcome::Trashed(path)],
        ));

        let r = results(&app);
        assert_eq!(
            r.tree.node(NodeId::ROOT).total().alloc,
            root_before - cache_size,
            "ancestors drop at once"
        );
        assert!(r.tree.node(cache).has_flag(NodeFlags::TRASHED));
        assert!(
            r.view.rows.iter().any(|row| row.key == RowKey::Dir(cache)),
            "the struck-through row stays"
        );
        assert!(app.toast_text().is_some_and(|t| {
            t.starts_with("Moved “node_modules” to the Trash. Empty the Trash to free")
        }));

        // The scan is complete; the post-trash re-order still runs on the tick.
        let later = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let _ = app.update(Message::Tick(later));
        assert!(
            !results(&app)
                .view
                .rows
                .iter()
                .any(|row| row.key == RowKey::Dir(cache)),
            "gone after the re-order"
        );
    }

    #[test]
    fn failure_restores_the_row_with_a_sticky_toast() {
        let mut app = app();
        let (_, cache, _) = scanned(&mut app);
        select(&mut app, cache);
        let _ = app.update(Message::RequestTrash);
        let _ = app.update(Message::ConfirmTrash);
        let generation = results(&app).tree_gen;
        let path = results(&app).tree.path_of(cache);
        let error = TrashError::Os("Operation not permitted".into());
        let _ = app.update(Message::TrashFinished(
            TrashJobId(1),
            generation,
            vec![TrashOutcome::Failed(path, error)],
        ));
        let node = results(&app).tree.node(cache);
        assert!(!node.has_flag(NodeFlags::TRASH_PENDING) && !node.has_flag(NodeFlags::TRASHED));
        assert_eq!(
            app.toast_text(),
            Some("Couldn't move “node_modules” to the Trash: Operation not permitted")
        );
        // Sticky: still there after the info-toast timeout.
        let _ = app.update(Message::Tick(
            std::time::Instant::now() + std::time::Duration::from_secs(10),
        ));
        assert!(app.toast_text().is_some());
    }

    #[test]
    fn a_result_for_an_old_tree_changes_nothing() {
        let mut app = app();
        let (_, cache, _) = scanned(&mut app);
        select(&mut app, cache);
        let _ = app.update(Message::RequestTrash);
        let _ = app.update(Message::ConfirmTrash);
        let before = results(&app).tree.node(NodeId::ROOT).total().alloc;
        let path = results(&app).tree.path_of(cache);
        let stale = results(&app).tree_gen + 7;
        let _ = app.update(Message::TrashFinished(
            TrashJobId(1),
            stale,
            vec![TrashOutcome::Trashed(path)],
        ));
        assert_eq!(results(&app).tree.node(NodeId::ROOT).total().alloc, before);
        assert!(!results(&app).tree.node(cache).has_flag(NodeFlags::TRASHED));
    }

    #[test]
    fn the_trash_button_is_disabled_for_unfinished_folders_and_other_volumes() {
        let mut app = app();
        let (big, _, holder) = scanned(&mut app);
        let trash_action = |app: &App, id| {
            contents(results(app), RowKey::Dir(id), None, 0)
                .unwrap()
                .actions
                .into_iter()
                .find(|a| matches!(a, Action::Trash(_)))
        };
        assert_eq!(trash_action(&app, big), Some(Action::Trash(Ok(()))));
        assert_eq!(
            trash_action(&app, holder),
            Some(Action::Trash(Err("This folder contains another volume.")))
        );
        let _ = app.update(Message::RowPressed(RowKey::Dir(holder)));
        let _ = app.update(Message::RequestTrash);
        assert!(
            matches!(app.modal(), Some(Modal::Refuse { message, .. }) if message.contains("another volume"))
        );

        // While scanning, an unfinished folder can't be trashed.
        let builder = TreeBuilder::new("/r", dir("r").child(dir("x").child(dir("y"))));
        let id = app.start_simulated("/r");
        let _ = app.update(Message::Scan(id, Update::Started(builder.root_info())));
        let _ = app.update(Message::Scan(
            id,
            batch(builder.listings().into_iter().take(2).collect()),
        ));
        assert_eq!(
            trash_action(&app, NodeId(1)),
            Some(Action::Trash(Err(
                "Available when this folder is fully scanned."
            )))
        );
    }
}

mod cleanup_flow {
    use super::*;
    use crate::keys::KeyAction;
    use crate::results::Pane;
    use crate::results::cleanup_pane::Line;
    use crate::results::rows::RowKey;
    use crate::results::trash::Modal;
    use dirstat_core::catalog::{Section, Verdict};
    use dirstat_core::tree::CandidateRef;

    fn target(id: &str) -> Verdict {
        Verdict::Target {
            rule: engine().rule_by_id(id).unwrap(),
            by_default: false,
        }
    }

    /// Two Safe cargo targets (500 MB, 300 MB), one Low node_modules (40 MB), a tool-only
    /// Trash folder (20 MB), and a deep Safe target to reveal.
    fn scanned(app: &mut App) {
        let proj = |name: &str, mb: u64| {
            dir(name).child(
                dir("target")
                    .verdict(target("cargo-target"))
                    .small_files(1, mb * 1_000_000),
            )
        };
        let builder = TreeBuilder::new(
            "/r",
            dir("r")
                .child(proj("a", 500))
                .child(proj("b", 300))
                .child(
                    dir("web").child(
                        dir("node_modules")
                            .verdict(target("node-modules"))
                            .small_files(1, 40_000_000),
                    ),
                )
                .child(
                    dir(".Trash")
                        .verdict(target("mac-trash"))
                        .small_files(1, 20_000_000),
                ),
        );
        completed(app, &builder);
    }

    fn group(app: &App, section: Section, rule: &str) -> crate::results::cleanup::Group {
        let rule = engine().rule_by_id(rule).unwrap();
        results(app)
            .cleanup
            .model
            .sections
            .iter()
            .find(|s| s.section == section)
            .and_then(|s| s.groups.iter().find(|g| g.rule == rule))
            .cloned()
            .unwrap_or_else(|| panic!("{:?}", results(app).cleanup.model))
    }

    #[test]
    fn sections_and_totals_after_the_scan() {
        let mut app = app();
        scanned(&mut app);
        let m = &results(&app).cleanup.model;
        assert!(!m.scanning);
        assert!(
            (800_000_000..800_100_000).contains(&m.safe_total),
            "{}",
            m.safe_total
        );
        assert!(
            (60_000_000..60_100_000).contains(&m.review_total),
            "{}",
            m.review_total
        );
        let cargo = group(&app, Section::SafeToDelete, "cargo-target");
        assert_eq!(cargo.items.len(), 2);
        let modules = group(&app, Section::ReviewFirst, "node-modules");
        assert!(modules.redownloadable);
        assert!(
            group(&app, Section::ReviewFirst, "mac-trash")
                .command
                .is_some()
        );
    }

    #[test]
    fn trash_from_the_pane_toasts_for_groups_and_tool_only_rules() {
        let mut app = app();
        scanned(&mut app);
        let cargo = group(&app, Section::SafeToDelete, "cargo-target").key();
        let trash = group(&app, Section::ReviewFirst, "mac-trash").key();

        // A multi-match group: no dialog, a toast.
        let _ = app.update(Message::Key(KeyAction::PaneCleanup));
        assert_eq!(results(&app).focus, Pane::Cleanup);
        if let Screen::Results(r) = &mut app.screen {
            r.cleanup.cursor = Some(Line::Group(cargo));
        }
        let _ = app.update(Message::Key(KeyAction::Trash));
        assert!(app.modal().is_none());
        assert_eq!(
            app.toast_text(),
            Some("Select one folder in the group to move it to the Trash.")
        );

        // A tool-only rule: the command instead.
        if let Screen::Results(r) = &mut app.screen {
            r.cleanup.cursor = Some(Line::Group(trash));
        }
        let _ = app.update(Message::Key(KeyAction::Trash));
        assert!(app.modal().is_none());
        assert!(
            app.toast_text()
                .is_some_and(|t| t.starts_with("Use the command instead: ")),
            "{:?}",
            app.toast_text()
        );

        // One item of the group: the normal dialog.
        let _ = app.update(Message::CleanupGroupPressed(cargo));
        let item = results(&app)
            .cleanup
            .lines()
            .into_iter()
            .find(|l| matches!(l, Line::Item(..)));
        if let Screen::Results(r) = &mut app.screen {
            r.cleanup.cursor = item;
        }
        let _ = app.update(Message::Key(KeyAction::Trash));
        assert!(
            matches!(app.modal(), Some(Modal::Confirm { .. })),
            "{:?}",
            app.modal()
        );
    }

    #[test]
    fn clicking_an_item_reveals_selects_and_flashes_it() {
        let mut app = app();
        scanned(&mut app);
        let cargo = group(&app, Section::SafeToDelete, "cargo-target");
        let candidate = cargo.items[1].candidate;
        let CandidateRef::Dir(id) = candidate else {
            panic!()
        };
        let _ = app.update(Message::CleanupItemPressed(cargo.key(), candidate));
        let r = results(&app);
        assert_eq!(r.view.selection, Some(RowKey::Dir(id)));
        assert!(
            r.view.rows.iter().any(|row| row.key == RowKey::Dir(id)),
            "parent expanded"
        );
        assert!(r.view.flash.is_some_and(|(k, _)| k == RowKey::Dir(id)));
        assert_eq!(r.focus, Pane::Cleanup, "a click keeps the pane focus");

        // Return on a focused item reveals it and hands focus to the tree.
        let _ = app.update(Message::Key(KeyAction::Enter));
        assert_eq!(results(&app).focus, Pane::Tree);
    }

    #[test]
    fn a_single_match_group_reveals_and_tab_switches_panes() {
        let mut app = app();
        app.window_size = iced::Size::new(1400.0, 900.0);
        scanned(&mut app);
        let modules = group(&app, Section::ReviewFirst, "node-modules");
        let _ = app.update(Message::CleanupGroupPressed(modules.key()));
        let CandidateRef::Dir(id) = modules.items[0].candidate else {
            panic!()
        };
        assert_eq!(results(&app).view.selection, Some(RowKey::Dir(id)));
        assert!(!results(&app).cleanup.expanded.contains(&modules.key()));

        let _ = app.update(Message::Key(KeyAction::Tab));
        assert_eq!(results(&app).focus, Pane::Tree);
        let _ = app.update(Message::Key(KeyAction::Tab));
        assert_eq!(results(&app).focus, Pane::Cleanup);
        let _ = app.update(Message::Key(KeyAction::TogglePane));
        assert_eq!(
            results(&app).focus,
            Pane::Tree,
            "hiding the pane returns focus"
        );
    }
}
