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
    app_with(Box::new(MemoryPrefs(Mutex::new(Prefs::default()))))
}

fn app_with(prefs_store: Box<dyn crate::prefs::PrefsStore>) -> App {
    let env = Env {
        engine: engine(),
        prefs_store,
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

        // A multi-match group: one dialog for all of its folders.
        let _ = app.update(Message::Key(KeyAction::PaneCleanup));
        assert_eq!(results(&app).focus, Pane::Cleanup);
        if let Screen::Results(r) = &mut app.screen {
            r.cleanup.cursor = Some(Line::Group(cargo));
        }
        let _ = app.update(Message::Key(KeyAction::Trash));
        let Some(Modal::ConfirmBatch { batch, .. }) = app.modal() else {
            panic!("{:?}", app.modal())
        };
        assert_eq!(batch.plans.len(), 2);
        let _ = app.update(Message::Key(KeyAction::Escape));

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

mod rescan_flow {
    use super::*;
    use crate::keys::KeyAction;
    use crate::results::rows::RowKey;
    use crate::results::scan_task::{ActiveScan, ScanId};
    use crate::results::trash::Modal;
    use dirstat_core::tree::NodeFlags;

    fn before() -> TreeBuilder {
        TreeBuilder::new(
            "/r",
            dir("r")
                .child(dir("a").small_files(2, 10_000_000).child(dir("deep")))
                .child(dir("b").small_files(1, 5_000_000)),
        )
    }

    /// `a` after files were added: 30 MB.
    fn a_after() -> TreeBuilder {
        TreeBuilder::new(
            "/r/a",
            dir("a")
                .small_files(3, 30_000_000)
                .child(dir("deep"))
                .child(dir("new")),
        )
    }

    fn find(app: &App, p: &str) -> NodeId {
        results(app)
            .tree
            .find_path(std::path::Path::new(p))
            .unwrap()
    }

    fn total(app: &App, id: NodeId) -> u64 {
        results(app).tree.node(id).total().alloc
    }

    /// Starts a rescan with an inert scan behind it and returns its id.
    fn begin(app: &mut App, target: NodeId) -> ScanId {
        let id = ScanId(900 + target.0 as u64);
        let Screen::Results(r) = &mut app.screen else {
            panic!()
        };
        assert_eq!(r.rescan_refusal(target), None);
        r.begin_rescan(target, ActiveScan::inert(id));
        id
    }

    fn feed(app: &mut App, id: ScanId, sub: &TreeBuilder, finish: bool) {
        let _ = app.update(Message::Scan(id, Update::Started(sub.root_info())));
        let _ = app.update(Message::Scan(id, batch(sub.listings())));
        if finish {
            let _ = app.update(Message::Scan(id, Update::Finished(Summary::default())));
        }
    }

    #[test]
    fn old_numbers_stay_until_the_swap_then_everything_moves_at_once() {
        let mut app = app();
        completed(&mut app, &before());
        let a = find(&app, "/r/a");
        let root_before = total(&app, NodeId::ROOT);
        let a_before = total(&app, a);
        let _ = app.update(Message::RowPressed(RowKey::Dir(a)));
        let _ = app.update(Message::ToggleExpand(RowKey::Dir(a)));

        let id = begin(&mut app, a);
        assert!(results(&app).tree.node(a).has_flag(NodeFlags::RESCANNING));
        feed(&mut app, id, &a_after(), false);
        assert_eq!(total(&app, a), a_before, "no dip-then-grow");
        assert_eq!(total(&app, NodeId::ROOT), root_before);
        assert_eq!(
            results(&app).phase,
            Phase::Complete,
            "a rescan keeps the phase"
        );

        let _ = app.update(Message::Scan(id, Update::Finished(Summary::default())));
        let r = results(&app);
        assert!(r.rescan.is_none());
        assert!(!r.tree.node(a).has_flag(NodeFlags::RESCANNING));
        let a_new = total(&app, a);
        assert!(a_new > a_before + 19_000_000, "{a_new}");
        assert_eq!(total(&app, NodeId::ROOT), root_before - a_before + a_new);
        assert_eq!(r.view.selection, Some(RowKey::Dir(a)), "selection survives");
        assert!(
            r.tree.node(a).has_flag(NodeFlags::EXPANDED),
            "expansion survives"
        );
        let new = find(&app, "/r/a/new");
        assert!(
            results(&app)
                .view
                .rows
                .iter()
                .any(|row| row.key == RowKey::Dir(new)),
            "the new folder shows at once"
        );
    }

    #[test]
    fn stop_and_failure_leave_the_tree_unchanged() {
        let mut app = app();
        completed(&mut app, &before());
        let a = find(&app, "/r/a");
        let a_before = total(&app, a);

        let id = begin(&mut app, a);
        feed(&mut app, id, &a_after(), false);
        let _ = app.update(Message::Key(KeyAction::Stop));
        let r = results(&app);
        assert!(r.rescan.is_none());
        assert_eq!(r.phase, Phase::Complete);
        assert!(!r.tree.node(a).has_flag(NodeFlags::RESCANNING));
        assert_eq!(total(&app, a), a_before);
        // A late Finished of the stopped rescan changes nothing.
        let _ = app.update(Message::Scan(id, Update::Finished(Summary::default())));
        assert_eq!(total(&app, a), a_before);

        let id = begin(&mut app, a);
        let _ = app.update(Message::Scan(
            id,
            Update::Failed(StartError {
                kind: ErrorKind::NotFound,
                message: "gone".into(),
            }),
        ));
        assert_eq!(
            app.toast_text(),
            Some("Folder no longer exists — rescan the parent")
        );
        assert!(!results(&app).tree.node(a).has_flag(NodeFlags::RESCANNING));
        assert!(results(&app).can_rescan());
    }

    #[test]
    fn a_root_rescan_replaces_the_tree_and_keeps_the_view() {
        let mut app = app();
        completed(&mut app, &before());
        let b = find(&app, "/r/b");
        let _ = app.update(Message::RowPressed(RowKey::Dir(b)));
        let id = begin(&mut app, NodeId::ROOT);
        let after = TreeBuilder::new(
            "/r",
            dir("r")
                .child(dir("b").small_files(1, 5_000_000))
                .child(dir("c").small_files(1, 1_000_000)),
        );
        feed(&mut app, id, &after, true);
        let r = results(&app);
        assert!(r.tree.find_path(std::path::Path::new("/r/a")).is_none());
        assert_eq!(r.view.selection, Some(RowKey::Dir(find(&app, "/r/b"))));
        assert_eq!(r.scan_id, id, "late results of the old tree are dropped");
        assert_eq!(r.phase, Phase::Complete);
    }

    #[test]
    fn rescanning_the_last_unfinished_folder_completes_a_stopped_scan() {
        let mut app = app();
        let builder = before();
        let scan = app.start_simulated("/r");
        let _ = app.update(Message::Scan(scan, Update::Started(builder.root_info())));
        let mut listings = builder.listings();
        // Root and `b` listed; `a` (and below) never.
        listings.retain(|l| l.id == NodeId::ROOT || l.id == NodeId(2));
        let _ = app.update(Message::Scan(scan, batch(listings)));
        let _ = app.update(Message::Key(KeyAction::Stop));
        assert_eq!(results(&app).phase, Phase::Stopped);

        let a = find(&app, "/r/a");
        let id = begin(&mut app, a);
        feed(&mut app, id, &a_after(), true);
        let r = results(&app);
        assert!(r.tree.node(NodeId::ROOT).is_done());
        assert_eq!(r.phase, Phase::Complete);
    }

    #[test]
    fn refusals_while_scanning_and_with_trash_pending() {
        let mut app = app();
        let builder = before();
        let scan = app.start_simulated("/r");
        let _ = app.update(Message::Scan(scan, Update::Started(builder.root_info())));
        let _ = app.update(Message::Key(KeyAction::Rescan));
        assert_eq!(app.toast_text(), Some("Available when the scan ends."));
        let _ = app.update(Message::Scan(scan, batch(builder.listings())));
        let _ = app.update(Message::Scan(scan, Update::Finished(Summary::default())));

        // A trash job inside the target blocks its rescan, not a sibling's.
        let deep = find(&app, "/r/a/deep");
        let _ = app.update(Message::RowPressed(RowKey::Dir(deep)));
        let _ = app.update(Message::RequestTrash);
        let _ = app.update(Message::ConfirmTrash);
        assert_eq!(results(&app).trash_jobs.len(), 1);
        let a = find(&app, "/r/a");
        assert_eq!(
            results(&app).rescan_refusal(a),
            Some("Available when the move to the Trash finishes.")
        );
        let b = find(&app, "/r/b");
        let id = begin(&mut app, b);
        feed(&mut app, id, &TreeBuilder::new("/r/b", dir("b")), true);

        // The trash job finishes after the sibling's splice: totals stay consistent.
        let root = total(&app, NodeId::ROOT);
        let deep_size = total(&app, deep);
        let generation = results(&app).tree_gen;
        let path = results(&app).tree.path_of(deep);
        let _ = app.update(Message::TrashFinished(
            dirstat_core::trash::TrashJobId(1),
            generation,
            vec![dirstat_core::trash::TrashOutcome::Trashed(path)],
        ));
        assert_eq!(total(&app, NodeId::ROOT), root - deep_size);
        assert!(results(&app).tree.node(deep).has_flag(NodeFlags::TRASHED));
    }

    #[test]
    fn new_scan_while_scanning_asks_first() {
        let mut app = app();
        let builder = before();
        let scan = app.start_simulated("/r");
        let _ = app.update(Message::Scan(scan, Update::Started(builder.root_info())));
        let _ = app.update(Message::Key(KeyAction::NewScan));
        assert!(matches!(app.modal(), Some(Modal::StopScan { .. })));
        let _ = app.update(Message::Key(KeyAction::Enter));
        assert!(app.modal().is_none(), "Return keeps scanning");
        assert!(matches!(app.screen, Screen::Results(_)));

        let _ = app.update(Message::Key(KeyAction::NewScan));
        let _ = app.update(Message::Key(KeyAction::Tab));
        let _ = app.update(Message::Key(KeyAction::Space));
        assert!(
            matches!(app.screen, Screen::Start(_)),
            "stopped and started over"
        );
    }
}

mod polish {
    use super::*;
    use crate::keys::KeyAction;
    use crate::prefs::PrefsStore;
    use crate::results::order::Column;
    use crate::results::rows::RowKey;
    use crate::results::selection_bar::contents;
    use dirstat_core::volumes::ScanEstimate;

    /// A prefs file that outlives the app, like the JSON file on disk.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Prefs>>);

    impl PrefsStore for Shared {
        fn load(&self) -> Prefs {
            self.0.lock().unwrap().clone()
        }
        fn save(&self, prefs: &Prefs) {
            *self.0.lock().unwrap() = prefs.clone();
        }
    }

    fn with_estimate(app: &mut App, used: u64) {
        if let Screen::Results(r) = &mut app.screen {
            r.estimate = Some(ScanEstimate {
                container_used: Some(used),
                ..ScanEstimate::default()
            });
        }
    }

    #[test]
    fn unaccounted_space_shows_last_at_the_root_only_when_big_enough() {
        let builder = sample();
        // The gap is 5 GB of 6 GB used.
        let mut app = app();
        let id = app.start_simulated("/r");
        with_estimate(&mut app, 6_000_000_000);
        let _ = app.update(Message::Scan(id, Update::Started(builder.root_info())));
        let _ = app.update(Message::Scan(id, batch(builder.listings())));
        assert!(
            results(&app).view.unaccounted.is_none(),
            "only after completion"
        );
        let _ = app.update(Message::Scan(id, Update::Finished(Summary::default())));
        let r = results(&app);
        assert_eq!(
            r.view.rows.last().map(|row| row.key),
            Some(RowKey::Unaccounted)
        );
        let gap = r.view.unaccounted.unwrap();
        assert!(gap > 5_900_000_000, "{gap}");
        let bar = contents(r, RowKey::Unaccounted, None, 0).unwrap();
        assert_eq!(bar.path, "Unaccounted space");
        assert!(bar.actions.is_empty());

        // Zoomed in, the row is gone.
        let a = r.tree.find_path(std::path::Path::new("/r/a")).unwrap();
        let _ = app.update(Message::ZoomTo(a));
        assert!(
            !results(&app)
                .view
                .rows
                .iter()
                .any(|row| row.key == RowKey::Unaccounted)
        );

        // A small gap (under 1 GB) shows nothing; neither does a negative one.
        for used in [500_000_000, 1] {
            let mut app = super::app();
            let id = app.start_simulated("/r");
            with_estimate(&mut app, used);
            let _ = app.update(Message::Scan(id, Update::Started(builder.root_info())));
            let _ = app.update(Message::Scan(id, batch(builder.listings())));
            let _ = app.update(Message::Scan(id, Update::Finished(Summary::default())));
            assert!(results(&app).view.unaccounted.is_none());
        }
    }

    #[test]
    fn recents_side_pane_and_sort_survive_a_restart() {
        let store = Shared::default();
        let mut first = app_with(Box::new(store.clone()));
        let builder = sample();
        let id = first.start_simulated("/r");
        let _ = first.update(Message::Scan(id, Update::Started(builder.root_info())));
        let _ = first.update(Message::Scan(id, batch(builder.listings())));
        let _ = first.update(Message::Scan(id, Update::Finished(Summary::default())));
        let _ = first.update(Message::Key(KeyAction::TogglePane));
        let _ = first.update(Message::SortBy(Column::Name));
        drop(first);

        let second = app_with(Box::new(store.clone()));
        let Screen::Start(start) = &second.screen else {
            panic!()
        };
        assert_eq!(start.recents.len(), 1);
        assert_eq!(start.recents[0].path, std::path::Path::new("/r"));
        assert_eq!(start.recents[0].when, "just now");
        assert!(second.prefs().hide_side_pane);
        assert_eq!(second.prefs().sort.column, Column::Name);

        let mut second = second;
        let _ = second.update(Message::RemoveRecent("/r".into()));
        let Screen::Start(start) = &second.screen else {
            panic!()
        };
        assert!(start.recents.is_empty());
        assert!(store.load().recents.is_empty(), "saved at once");
    }

    #[test]
    fn folder_scans_count_finished_subfolders() {
        let mut app = app();
        let builder = sample();
        let id = app.start_simulated("/r");
        let _ = app.update(Message::Scan(id, Update::Started(builder.root_info())));
        let mut listings = builder.listings();
        // Root and `b` only: `a` (with `deep`) is still open.
        listings.retain(|l| l.id == NodeId::ROOT || l.id == NodeId(2));
        let _ = app.update(Message::Scan(id, batch(listings)));
        let lines = crate::results::activity::lines(results(&app), "r");
        assert_eq!(lines.line1, "Scanning r · 1 of 2 folders done");
    }
}

#[test]
fn with_full_disk_access_privacy_errors_raise_no_bar() {
    let builder = TreeBuilder::new(
        "/r",
        dir("r").child(dir("mail").error(dirstat_core::scan::ErrorClass::PrivacyBlocked)),
    );
    for has_fda in [false, true] {
        let mut app = app();
        let id = app.start_simulated("/r");
        if let Screen::Results(r) = &mut app.screen {
            r.has_fda = has_fda;
        }
        let _ = app.update(Message::Scan(id, Update::Started(builder.root_info())));
        let _ = app.update(Message::Scan(id, batch(builder.listings())));
        let r = results(&app);
        assert_eq!(r.fda_bar, cfg!(target_os = "macos") && !has_fda);
        assert_eq!(
            r.offer_privacy_settings(),
            cfg!(target_os = "macos") && !has_fda
        );
    }
}

mod map_flow {
    use super::*;
    use crate::keys::KeyAction;
    use crate::results::Pane;
    use crate::results::rows::RowKey;
    use crate::results::treemap::BlockKind;
    use crate::results::treemap_view::selected_block;
    use dirstat_core::tree::NodeFlags;
    use dirstat_core::tree::testing::large;

    fn builder() -> TreeBuilder {
        TreeBuilder::new(
            "/r",
            dir("r")
                .child(
                    dir("a")
                        .child(dir("deep").small_files(1, 40_000_000))
                        .large(large("big.iso", 30_000_000)),
                )
                .child(dir("b").small_files(1, 20_000_000)),
        )
    }

    fn find(app: &App, p: &str) -> NodeId {
        results(app)
            .tree
            .find_path(std::path::Path::new(p))
            .unwrap()
    }

    #[test]
    fn map_tab_clicks_select_and_double_clicks_zoom() {
        let mut app = app();
        app.window_size = iced::Size::new(1400.0, 900.0);
        completed(&mut app, &builder());
        let _ = app.update(Message::Key(KeyAction::PaneMap));
        assert!(app.prefs().map_tab, "⌘2 shows the map and remembers it");
        let _ = app.update(Message::Key(KeyAction::Tab));
        assert_eq!(results(&app).focus, Pane::Tree, "the map takes no focus");

        // A click on a nested folder selects it and opens its ancestors in the tree.
        let deep = find(&app, "/r/a/deep");
        let _ = app.update(Message::MapPressed(RowKey::Dir(deep)));
        let r = results(&app);
        assert_eq!(r.view.selection, Some(RowKey::Dir(deep)));
        assert!(r.view.rows.iter().any(|row| row.key == RowKey::Dir(deep)));

        // A file block opens its folder and selects the file row.
        let a = find(&app, "/r/a");
        let _ = app.update(Message::MapPressed(RowKey::File(a, 0)));
        assert_eq!(results(&app).view.selection, Some(RowKey::File(a, 0)));
        assert!(results(&app).tree.node(a).has_flag(NodeFlags::EXPANDED));

        // Double-click: zoom into the folder; on a file, into its folder.
        let _ = app.update(Message::MapActivated(RowKey::Dir(deep)));
        assert_eq!(results(&app).view.zoom, deep);
        let _ = app.update(Message::Key(KeyAction::Back));
        let _ = app.update(Message::ZoomTo(NodeId::ROOT));
        let _ = app.update(Message::MapActivated(RowKey::File(a, 0)));
        assert_eq!(results(&app).view.zoom, a);
        assert_eq!(results(&app).view.selection, Some(RowKey::File(a, 0)));

        // ⌘1 goes back to Cleanup.
        let _ = app.update(Message::Key(KeyAction::PaneCleanup));
        assert!(!app.prefs().map_tab);
        assert_eq!(results(&app).focus, Pane::Cleanup);
    }

    #[test]
    fn selection_and_hover_sync_both_ways() {
        let mut app = app();
        completed(&mut app, &builder());
        let _ = app.update(Message::Tick(std::time::Instant::now()));
        let b = find(&app, "/r/b");
        let _ = app.update(Message::RowPressed(RowKey::Dir(b)));
        let r = results(&app);
        let layout = r
            .map
            .layout_for(&r.tree, r.view.zoom, iced::Size::new(400.0, 600.0));
        let index = selected_block(r, &layout).expect("the tree selection is outlined");
        assert_eq!(layout.blocks[index].kind, BlockKind::Dir(b));

        // A deep selection outlines its nearest drawn ancestor.
        let deep = find(&app, "/r/a/deep");
        let _ = app.update(Message::RowPressed(RowKey::Dir(deep)));
        let small = results(&app).map.layout_for(
            &results(&app).tree,
            NodeId::ROOT,
            iced::Size::new(90.0, 60.0),
        );
        let index = selected_block(results(&app), &small).unwrap();
        assert!(matches!(small.blocks[index].kind, BlockKind::Dir(_)));

        let _ = app.update(Message::MapHover(Some(RowKey::Dir(b))));
        assert_eq!(results(&app).map.hover, Some(RowKey::Dir(b)));
        let _ = app.update(Message::PointerInMap(false));
        assert_eq!(
            results(&app).map.hover,
            None,
            "leaving the map clears the highlight"
        );
    }

    #[test]
    fn no_relayout_under_the_pointer_while_scanning() {
        let mut app = app();
        let builder = builder();
        let id = app.start_simulated("/r");
        let _ = app.update(Message::Scan(id, Update::Started(builder.root_info())));
        let t0 = std::time::Instant::now();
        let _ = app.update(Message::Tick(t0));
        let before = results(&app).map.clock.version;
        let _ = app.update(Message::PointerInMap(true));
        let _ = app.update(Message::Scan(id, batch(builder.listings())));
        let _ = app.update(Message::Tick(t0 + std::time::Duration::from_secs(2)));
        assert_eq!(
            results(&app).map.clock.version,
            before,
            "pointer over the map"
        );
        let _ = app.update(Message::PointerInMap(false));
        let _ = app.update(Message::Tick(t0 + std::time::Duration::from_secs(3)));
        assert_eq!(results(&app).map.clock.version, before + 1);
    }
}

mod batch_flow {
    use super::*;
    use crate::keys::KeyAction;
    use crate::results::cleanup::GroupKey;
    use crate::results::trash::{Focus, Modal};
    use dirstat_core::catalog::{Section, Verdict};
    use dirstat_core::trash::{TrashError, TrashJobId, TrashOutcome};
    use dirstat_core::tree::NodeFlags;

    fn target(id: &str) -> Verdict {
        Verdict::Target {
            rule: engine().rule_by_id(id).unwrap(),
            by_default: false,
        }
    }

    /// Three cargo targets (Safe) next to a big folder, two node_modules (Review first).
    fn scanned(app: &mut App) {
        let proj = |name: &str, mb: u64| {
            dir(name)
                .child(
                    dir("target")
                        .verdict(target("cargo-target"))
                        .small_files(1, mb * 1_000_000),
                )
                .child(
                    dir("node_modules")
                        .verdict(target("node-modules"))
                        .small_files(1, 20_000_000),
                )
        };
        let builder = TreeBuilder::new(
            "/r",
            dir("r")
                .child(dir("photos").small_files(1, 2_000_000_000))
                .child(proj("a", 50))
                .child(proj("b", 40))
                .child(
                    dir("c").child(
                        dir("target")
                            .verdict(target("cargo-target"))
                            .small_files(1, 30_000_000),
                    ),
                ),
        );
        completed(app, &builder);
    }

    fn group(app: &App, section: Section, rule: &str) -> GroupKey {
        let rule = engine().rule_by_id(rule).unwrap();
        results(app)
            .cleanup
            .model
            .sections
            .iter()
            .find(|s| s.section == section)
            .and_then(|s| s.groups.iter().chain(&s.small).find(|g| g.rule == rule))
            .map(|g| g.key())
            .unwrap_or_else(|| panic!("{:?}", results(app).cleanup.model))
    }

    fn finish(app: &mut App, id: u64, ok: bool) {
        let r = results(app);
        let pending = &r.trash_jobs[&TrashJobId(id)];
        let path = r.tree.path_of(pending.target.dir());
        let outcome = if ok {
            TrashOutcome::Trashed(path)
        } else {
            TrashOutcome::Failed(path, TrashError::Changed)
        };
        let generation = r.tree_gen;
        let _ = app.update(Message::TrashFinished(
            TrashJobId(id),
            generation,
            vec![outcome],
        ));
    }

    #[test]
    fn safe_group_moves_together_with_one_dialog_and_one_toast() {
        let mut app = app();
        scanned(&mut app);
        let cargo = group(&app, Section::SafeToDelete, "cargo-target");
        let root_before = results(&app).tree.node(NodeId::ROOT).total().alloc;
        let _ = app.update(Message::TrashGroup(cargo));
        let Some(Modal::ConfirmBatch { batch, focus }) = app.modal() else {
            panic!("{:?}", app.modal())
        };
        assert_eq!(batch.plans.len(), 3);
        assert!(batch.skipped.is_empty());
        assert_eq!(batch.title, "Move 3 folders to the Trash?");
        assert!(
            batch.subtitle.starts_with("Cargo target"),
            "{}",
            batch.subtitle
        );
        assert!(
            batch.default_is_trash,
            "clean Safe matches, a small share of the scan"
        );
        assert_eq!(*focus, Focus::Trash);
        assert!(
            results(&app).trash_jobs.is_empty(),
            "nothing moves before confirming"
        );

        let _ = app.update(Message::Key(KeyAction::Enter));
        assert!(app.modal().is_none());
        let ids: Vec<u64> = {
            let mut ids: Vec<u64> = results(&app).trash_jobs.keys().map(|k| k.0).collect();
            ids.sort();
            ids
        };
        assert_eq!(ids.len(), 3, "one job per folder");
        for id in &ids {
            let dir = results(&app).trash_jobs[&TrashJobId(*id)].target.dir();
            assert!(
                results(&app)
                    .tree
                    .node(dir)
                    .has_flag(NodeFlags::TRASH_PENDING)
            );
        }
        finish(&mut app, ids[0], true);
        finish(&mut app, ids[1], true);
        assert!(app.toast_text().is_none(), "no toast until the last one");
        finish(&mut app, ids[2], true);
        let toast = app.toast_text().unwrap();
        assert!(
            toast.starts_with("Moved 3 folders to the Trash. Empty the Trash to free"),
            "{toast}"
        );
        assert_eq!(
            results(&app).tree.node(NodeId::ROOT).total().alloc,
            root_before - 120_000_000 - 3 * 4096,
            "every folder left its ancestors"
        );
    }

    #[test]
    fn a_failure_in_a_batch_gives_a_sticky_summary() {
        let mut app = app();
        scanned(&mut app);
        let cargo = group(&app, Section::SafeToDelete, "cargo-target");
        let _ = app.update(Message::TrashGroup(cargo));
        let _ = app.update(Message::ConfirmBatch);
        let mut ids: Vec<u64> = results(&app).trash_jobs.keys().map(|k| k.0).collect();
        ids.sort();
        finish(&mut app, ids[0], true);
        finish(&mut app, ids[1], false);
        finish(&mut app, ids[2], true);
        let toast = app.toast_text().unwrap();
        assert!(
            toast.starts_with("Moved 2 folders to the Trash; 1 didn't move."),
            "{toast}"
        );
        assert!(results(&app).trash_batches.is_empty());
    }

    #[test]
    fn review_groups_default_to_cancel_and_unfinished_folders_are_skipped() {
        let mut app = app();
        scanned(&mut app);
        let modules = group(&app, Section::ReviewFirst, "node-modules");
        let _ = app.update(Message::TrashGroup(modules));
        let Some(Modal::ConfirmBatch { batch, focus }) = app.modal() else {
            panic!("{:?}", app.modal())
        };
        assert!(!batch.default_is_trash);
        assert_eq!(*focus, Focus::Cancel);
        let _ = app.update(Message::Key(KeyAction::Enter));
        assert!(
            app.modal().is_none() && results(&app).trash_jobs.is_empty(),
            "Return cancelled"
        );

        // A folder that is already being moved stays out of the batch, with its reason.
        let cargo = group(&app, Section::SafeToDelete, "cargo-target");
        let first = results(&app).cleanup.group(cargo).unwrap().items[0].candidate;
        if let Screen::Results(r) = &mut app.screen {
            r.tree.set_flag(first.dir(), NodeFlags::TRASH_PENDING, true);
        }
        let _ = app.update(Message::TrashGroup(cargo));
        let Some(Modal::ConfirmBatch { batch, .. }) = app.modal() else {
            panic!("{:?}", app.modal())
        };
        assert_eq!((batch.plans.len(), batch.skipped.len()), (2, 1));
    }
}
