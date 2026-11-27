use std::collections::HashMap;
use std::path::PathBuf;

use super::*;
use crate::catalog::model::Os;
use crate::scan::FileEntry;
use crate::tree::testing::{Dir, TreeBuilder, dir, large};

const MIB: u64 = 1 << 20;
/// `dir()` gives every folder 4 kB of its own blocks.
const DIR: u64 = 4096;

fn engine() -> Engine {
    Engine::new(&Resolver {
        os: Os::LINUX,
        home: "/home/u".into(),
        vars: HashMap::new(),
    })
}

fn rule(e: &Engine, id: &str) -> RuleIdx {
    e.rule_by_id(id).unwrap_or_else(|| panic!("no rule {id}"))
}

fn target(rule: RuleIdx) -> Verdict {
    Verdict::Target {
        rule,
        by_default: false,
    }
}

fn places() -> Places {
    Places::with(vec![
        (PathBuf::from("/home/u/Documents"), Kind::Documents),
        (PathBuf::from("/opt"), Kind::Applications),
    ])
}

fn build(root: &str, d: Dir) -> Tree {
    TreeBuilder::new(root, d).build()
}

fn mb(n: u64) -> u64 {
    n * MIB
}

#[test]
fn every_byte_counts_once() {
    let e = engine();
    let tree = build(
        "/home/u",
        dir("u")
            .small_files(3, mb(1) / 2)
            .child(
                dir("Documents")
                    .small_files(10, mb(5))
                    .child(dir("thesis").small_files(4, mb(2))),
            )
            .child(dir("misc").small_files(2, mb(3))),
    );
    let b = compute(&tree, &e, &places(), NodeId::ROOT);
    assert_eq!(b.total(), tree.node(NodeId::ROOT).total().alloc);
    assert_eq!(b.get(Kind::Documents), mb(7) + 2 * DIR);
    assert_eq!(b.get(Kind::Other), mb(3) + mb(1) / 2 + 2 * DIR);
}

#[test]
fn a_cleanup_match_wins_over_its_location() {
    let e = engine();
    let nm = rule(&e, "node-modules");
    let tree = build(
        "/home/u",
        dir("u").child(
            dir("Documents").child(
                dir("app").small_files(1, mb(1)).child(
                    dir("node_modules")
                        .verdict(target(nm))
                        .small_files(50, mb(40)),
                ),
            ),
        ),
    );
    let b = compute(&tree, &e, &places(), NodeId::ROOT);
    assert_eq!(b.get(Kind::Developer), mb(40) + DIR);
    assert_eq!(b.get(Kind::Documents), mb(1) + 2 * DIR);
}

#[test]
fn protect_rules_and_names_give_a_kind_to_whole_subtrees() {
    let e = engine();
    let bundle = Verdict::Protected(rule(&e, "app-bundles"));
    let system = Verdict::Protected(rule(&e, "linux-system"));
    let tree = build(
        "/",
        dir("/")
            .child(
                dir("usr")
                    .verdict(system)
                    .child(dir("lib").small_files(9, mb(9))),
            )
            .child(
                dir("opt")
                    .verdict(system)
                    .child(dir("tool").small_files(1, mb(4))),
            )
            .child(
                dir("data")
                    .child(
                        dir("Viewer.app")
                            .verdict(bundle)
                            .child(dir("Contents").small_files(1, mb(6))),
                    )
                    .child(
                        dir("SteamLibrary")
                            .child(dir("steamapps").child(dir("common").small_files(1, mb(30)))),
                    ),
            ),
    );
    let b = compute(&tree, &e, &places(), NodeId::ROOT);
    assert_eq!(b.get(Kind::System), mb(9) + 2 * DIR);
    // `/opt` is a place: stronger than linux-system's protect rule.
    assert_eq!(b.get(Kind::Applications), mb(4) + mb(6) + 4 * DIR);
    assert_eq!(b.get(Kind::Games), mb(30) + 3 * DIR);
    assert_eq!(b.get(Kind::Other), 2 * DIR);
}

#[test]
fn zoom_inherits_from_ancestors_and_from_places_above_the_root() {
    let e = engine();
    let d = dir("Documents").child(dir("thesis").child(dir("figures").small_files(2, mb(2))));
    let tree = build("/home/u/Documents", d);
    let figures = tree
        .find_path(std::path::Path::new("/home/u/Documents/thesis/figures"))
        .unwrap();
    let b = compute(&tree, &e, &places(), figures);
    assert_eq!(b.get(Kind::Documents), mb(2) + DIR);
    assert_eq!(b.total(), b.get(Kind::Documents));

    let tree = build(
        "/home/u/Documents/thesis",
        dir("thesis").small_files(1, mb(1)),
    );
    let b = compute(&tree, &e, &places(), NodeId::ROOT);
    assert_eq!(b.get(Kind::Documents), mb(1) + DIR);
}

#[test]
fn matched_large_files_take_their_rule_kind() {
    let e = engine();
    let logs = rule(&e, "gradle-daemon");
    let tree = build(
        "/srv",
        dir("srv").small_files(1, mb(1)).large(FileEntry {
            matched: Some(FileMatch::Target(logs)),
            ..large("run.log", mb(8))
        }),
    );
    let b = compute(&tree, &e, &places(), NodeId::ROOT);
    assert_eq!(b.get(Kind::Logs), mb(8));
    assert_eq!(b.get(Kind::Other), mb(1) + DIR);
}

#[test]
fn trashed_folders_are_left_out() {
    let e = engine();
    let mut tree = build(
        "/srv",
        dir("srv")
            .child(dir("keep").small_files(1, mb(1)))
            .child(dir("gone").small_files(1, mb(5))),
    );
    let gone = tree.find_path(std::path::Path::new("/srv/gone")).unwrap();
    tree.mark_trashed(gone);
    let b = compute(&tree, &e, &places(), NodeId::ROOT);
    assert_eq!(b.total(), tree.node(NodeId::ROOT).total().alloc);
    assert_eq!(b.total(), mb(1) + 2 * DIR);
}

#[test]
fn places_resolve_from_the_environment() {
    let r = Resolver {
        os: Os::LINUX,
        home: "/home/u".into(),
        vars: HashMap::new(),
    };
    let p = Places::new(&r);
    let has = |path: &str, k: Kind| {
        p.anchors
            .iter()
            .any(|(a, kk)| a == &PathBuf::from(path) && *kk == k)
    };
    assert!(has("/home/u/Downloads", Kind::Documents));
    assert!(has("/home/u/.local/share/flatpak", Kind::Applications));
    assert!(has("/var/log", Kind::Logs));
    assert!(
        !p.anchors
            .iter()
            .any(|(a, _)| a == &PathBuf::from("/Applications"))
    );
}
