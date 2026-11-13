//! `plan` (design §7.1–7.3): eligibility, refusals, warnings and the default button.

use std::sync::OnceLock;
use std::time::SystemTime;

use dirstat_core::catalog::{Engine, GitState, Resolver, Verdict};
use dirstat_core::git::GitStatus;
use dirstat_core::safety::Guards;
use dirstat_core::scan::{ErrorClass, Skip};
use dirstat_core::tree::testing::{Dir, TreeBuilder, dir, large};
use dirstat_core::tree::{NodeFlags, NodeId};

use super::*;
use crate::results::scan_task::{ActiveScan, ScanId};

fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| Engine::new(&Resolver::from_env()))
}

fn target(id: &str) -> Verdict {
    Verdict::Target {
        rule: engine().rule_by_id(id).unwrap(),
        by_default: false,
    }
}

/// `/r/proj/<child>` next to a big sibling, so nothing is over 25% of the scan.
fn results_with(child: Dir) -> Results {
    let tree = TreeBuilder::new(
        "/r",
        dir("r")
            .child(dir("big").small_files(1, 100_000_000))
            .child(dir("proj").child(child)),
    )
    .build();
    let mut results = Results::new("/r".into(), "r".into(), None, ActiveScan::inert(ScanId(1)));
    results.tree = tree;
    results
}

fn guards() -> Guards {
    Guards::for_home("/nonexistent-home".into(), Default::default())
}

fn plan_for(results: &Results, rel: &str, git: GitState) -> Result<TrashPlan, Refused> {
    let id = results
        .tree
        .find_path(&std::path::Path::new("/r").join(rel))
        .unwrap();
    plan(
        results,
        engine(),
        &guards(),
        TrashTarget::Dir(id),
        |_| git,
        SystemTime::now(),
    )
}

#[test]
fn the_default_button_is_trash_only_for_clean_safe_matches() {
    let safe = results_with(
        dir("target")
            .verdict(target("cargo-target"))
            .small_files(10, 50_000),
    );
    let p = plan_for(&safe, "proj/target", GitState::NotInRepo).unwrap();
    assert!(p.default_is_trash, "{:?}", p.warnings);
    assert_eq!(
        p.reason.as_ref().map(|r| r.0),
        Some(Some(Section::SafeToDelete))
    );
    assert!(
        plan_for(&safe, "proj/target", GitState::Known(GitStatus::Clean))
            .unwrap()
            .default_is_trash
    );
    // Pending git, not ignored, tracked: Cancel.
    let pending = plan_for(&safe, "proj/target", GitState::Pending).unwrap();
    assert!(!pending.default_is_trash);
    assert_eq!(pending.reason, Some((None, "Checking with git…".into())));
    let not_ignored =
        plan_for(&safe, "proj/target", GitState::Known(GitStatus::NotIgnored)).unwrap();
    assert!(
        !not_ignored.default_is_trash
            && not_ignored
                .warnings
                .iter()
                .any(|w| w.contains("Not ignored by git"))
    );
    let tracked = plan_for(&safe, "proj/target", GitState::Known(GitStatus::Tracked)).unwrap();
    assert!(!tracked.default_is_trash);
    assert_eq!(
        tracked.reason, None,
        "no catalog reason for git-tracked folders"
    );

    // Low (node_modules): Review first, with the re-download wording.
    let low = results_with(
        dir("node_modules")
            .verdict(target("node-modules"))
            .small_files(10, 50_000),
    );
    let p = plan_for(&low, "proj/node_modules", GitState::NotInRepo).unwrap();
    assert!(!p.default_is_trash);
    assert_eq!(
        p.reason.as_ref().map(|r| r.0),
        Some(Some(Section::ReviewFirst))
    );
    assert_eq!(
        p.warnings[0], "Restored only by re-downloading or rebuilding.",
        "the reason has its own line"
    );

    // Not a candidate at all.
    let plain = results_with(dir("notes").small_files(3, 5000));
    let p = plan_for(&plain, "proj/notes", GitState::NotInRepo).unwrap();
    assert!(!p.default_is_trash && p.reason.is_none());
}

#[test]
fn warning_lines_and_their_triggers() {
    let risky = results_with(
        dir("target")
            .verdict(target("cargo-target"))
            .cloud_files(2)
            .linked_file(40_000)
            .small_files(1, 10_000)
            .child(dir("locked").error(ErrorClass::PermissionDenied)),
    );
    let p = plan_for(&risky, "proj/target", GitState::NotInRepo).unwrap();
    let has = |s: &str| p.warnings.iter().any(|w| w.contains(s));
    assert!(has("stored only in iCloud"));
    assert!(has("couldn't be read"));
    assert!(has("may free less than shown"));
    assert!(!p.default_is_trash, "any warning makes Cancel the default");

    // More than 25% of the scan.
    let tree = TreeBuilder::new(
        "/r",
        dir("r")
            .child(dir("huge").small_files(1, 9_000_000))
            .child(dir("small")),
    )
    .build();
    let mut big = Results::new("/r".into(), "r".into(), None, ActiveScan::inert(ScanId(1)));
    big.tree = tree;
    let p = plan_for(&big, "huge", GitState::NotInRepo).unwrap();
    assert!(
        p.warnings
            .iter()
            .any(|w| w.starts_with("This is ") && w.ends_with("of everything scanned."))
    );
}

#[test]
fn refusals() {
    let r = results_with(
        dir("data")
            .child(dir("mnt").skip(Skip::MountPoint))
            .small_files(1, 10),
    );
    let refused = plan_for(&r, "proj/data", GitState::NotInRepo).unwrap_err();
    assert!(
        refused.message.contains("contains another volume"),
        "{refused:?}"
    );
    // The scan root itself.
    let root = plan(
        &r,
        engine(),
        &guards(),
        TrashTarget::Dir(NodeId::ROOT),
        |_| GitState::NotInRepo,
        SystemTime::now(),
    )
    .unwrap_err();
    assert!(root.message.contains("won't move"));

    // Not fully scanned yet.
    let builder = TreeBuilder::new("/r", dir("r").child(dir("x").child(dir("y"))));
    let mut scanning = Results::new("/r".into(), "r".into(), None, ActiveScan::inert(ScanId(1)));
    scanning.tree = builder.started();
    for l in builder.listings().into_iter().take(2) {
        scanning.tree.apply(l);
    }
    let p = plan(
        &scanning,
        engine(),
        &guards(),
        TrashTarget::Dir(NodeId(1)),
        |_| GitState::NotInRepo,
        SystemTime::now(),
    )
    .unwrap_err();
    assert_eq!(p.message, "Available when this folder is fully scanned.");

    // A trashed or pending ancestor.
    let mut r = results_with(dir("a").child(dir("b").small_files(1, 10)));
    let a = r.tree.find_path(std::path::Path::new("/r/proj/a")).unwrap();
    r.tree.set_flag(a, NodeFlags::TRASH_PENDING, true);
    assert!(plan_for(&r, "proj/a/b", GitState::NotInRepo).is_err());
}

#[test]
fn contents_mode_lists_the_scanned_entries() {
    let r = results_with(
        dir("npm")
            .verdict(target("npm-cache"))
            .child(dir("_cacache"))
            .large(large("big.tgz", 3_000_000)),
    );
    let p = plan_for(&r, "proj/npm", GitState::NotInRepo).unwrap();
    let Mode::Contents { entries, .. } = &p.items[0].mode else {
        panic!("{:?}", p.items[0].mode)
    };
    let names: Vec<_> = entries
        .iter()
        .map(|e| e.name.to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["_cacache", "big.tgz"]);
    assert!(p.title.starts_with("Move the contents of"));
}
