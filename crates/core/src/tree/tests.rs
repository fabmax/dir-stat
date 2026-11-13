use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::testing::{Dir, TreeBuilder, dir, large};
use super::*;
use crate::scan::{ErrorClass, FileMatch, Skip, Summary};
use crate::trash::ContentsOutcome;

const MIB: u64 = 1 << 20;
const ROOT: &str = "/scan/r";

fn target(rule: u16) -> Verdict {
    Verdict::Target {
        rule,
        by_default: false,
    }
}

fn dir_a() -> Dir {
    dir("a")
        .small_files(5, 20_000)
        .mtime(18_000)
        .child(dir("a1").linked_file(8192).large(FileEntry {
            matched: Some(FileMatch::Target(7)),
            ..large("model.bin", 2 * MIB)
        }))
        .child(dir("a2").error(ErrorClass::PermissionDenied))
}

fn sample_with(a: Dir) -> TreeBuilder {
    TreeBuilder::new(
        ROOT,
        dir("r")
            .file("big.iso", 3 * MIB)
            .small_files(10, 40_000)
            .mtime(6_000)
            .child(a)
            .child(
                dir("b")
                    .verdict(target(3))
                    .small_files(100, 1_000_000)
                    .mtime(12_000)
                    .child(dir("b1").small_files(1, 4096)),
            )
            .child(dir("mnt").skip(Skip::MountPoint))
            .child(dir("cloud").skip(Skip::CloudOnly))
            .child(
                dir("c")
                    .cloud_files(2)
                    .entry_error("x", ErrorClass::PrivacyBlocked)
                    .child(dir("p").verdict(Verdict::Protected(9))),
            ),
    )
}

fn sample() -> TreeBuilder {
    sample_with(dir_a())
}

fn id(tree: &Tree, rel: &str) -> NodeId {
    let path = if rel.is_empty() {
        PathBuf::from(ROOT)
    } else {
        Path::new(ROOT).join(rel)
    };
    tree.find_path(&path)
        .unwrap_or_else(|| panic!("{rel} not in tree"))
}

/// Every live node by path, with its totals and state.
fn snapshot(tree: &Tree) -> HashMap<PathBuf, (Totals, NodeState, i64)> {
    let mut out = HashMap::new();
    let mut stack = vec![NodeId::ROOT];
    while let Some(n) = stack.pop() {
        let node = tree.node(n);
        out.insert(
            tree.path_of(n),
            (*node.total(), node.state(), node.newest_mtime()),
        );
        stack.extend(tree.children(n));
    }
    out
}

/// Listings reordered pseudo-randomly, keeping every parent before its children.
fn shuffled_parent_first(listings: Vec<DirListing>, mut seed: u64) -> Vec<DirListing> {
    let mut parent_of = HashMap::new();
    for l in &listings {
        for i in 0..l.subdirs.len() as u32 {
            parent_of.insert(NodeId(l.first_child.0 + i), l.id);
        }
    }
    let mut pending = listings;
    let mut out: Vec<DirListing> = Vec::new();
    while !pending.is_empty() {
        let ready: Vec<usize> = (0..pending.len())
            .filter(|&i| {
                parent_of
                    .get(&pending[i].id)
                    .is_none_or(|p| out.iter().any(|o| o.id == *p))
            })
            .collect();
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        out.push(pending.swap_remove(ready[(seed >> 33) as usize % ready.len()]));
    }
    out
}

#[test]
fn node_stays_small() {
    assert!(
        std::mem::size_of::<Node>() <= 176,
        "Node is {} bytes",
        std::mem::size_of::<Node>()
    );
}

#[test]
fn mtimes_are_kept_to_the_minute() {
    let tree = TreeBuilder::new(
        ROOT,
        dir("r").mtime(1_773_508_979).child(dir("old").mtime(-61)),
    )
    .build();
    assert_eq!(tree.node(NodeId::ROOT).newest_mtime(), 1_773_508_920);
    assert_eq!(
        tree.node(NodeId(1)).newest_mtime(),
        -120,
        "floors toward the past"
    );
}

#[test]
fn totals_equal_the_sum_of_listings() {
    let builder = sample();
    let listings = builder.listings();
    let tree = builder.build();
    let root = tree.node(NodeId::ROOT).total();
    assert_eq!(
        root.alloc,
        listings
            .iter()
            .map(|l| l.files.alloc + l.dir_alloc)
            .sum::<u64>()
    );
    assert_eq!(
        root.apparent,
        listings.iter().map(|l| l.files.apparent).sum::<u64>()
    );
    assert_eq!(
        root.items,
        listings
            .iter()
            .map(|l| l.files.count + l.subdirs.len() as u64)
            .sum::<u64>()
    );

    let a = tree.node(id(&tree, "a")).total();
    assert_eq!(a.alloc, 4096 + 20_000 + (4096 + 8192 + 2 * MIB) + 4096);
    assert_eq!(a.items, 5 + 2 + 2);
    assert_eq!(tree.listed(), listings.len() as u64);
}

#[test]
fn out_of_order_id_blocks_give_the_same_tree() {
    let builder = sample();
    let expected = snapshot(&builder.build());
    for seed in 0..20 {
        let order = shuffled_parent_first(builder.listings(), seed);
        let mut tree = builder.started();
        for listing in order {
            tree.apply(listing);
        }
        assert_eq!(snapshot(&tree), expected, "seed {seed}");
        assert!(tree.node(NodeId::ROOT).is_done());
    }
}

#[test]
fn root_is_done_exactly_when_everything_is_listed() {
    let builder = sample();
    let listings = builder.listings();
    let count = listings.len();
    let mut tree = builder.started();
    for (i, listing) in listings.into_iter().enumerate() {
        tree.apply(listing);
        assert_eq!(
            tree.node(NodeId::ROOT).is_done(),
            i + 1 == count,
            "after listing {i}"
        );
    }
    for (_, (_, state, _)) in snapshot(&tree) {
        assert!(
            matches!(state, NodeState::Done | NodeState::Skipped(_)),
            "{state:?}"
        );
    }
}

#[test]
fn skipped_children_never_block_completion() {
    let tree = TreeBuilder::new(
        ROOT,
        dir("r")
            .child(dir("m").skip(Skip::MountPoint))
            .child(dir("e").skip(Skip::Excluded)),
    )
    .build();
    assert!(tree.node(NodeId::ROOT).is_done());
    assert_eq!(tree.node(NodeId::ROOT).total().items, 2);
    assert_eq!(
        tree.node(id(&tree, "m")).state(),
        NodeState::Skipped(Skip::MountPoint)
    );
}

#[test]
fn counters_propagate_strictly_inside() {
    let tree = sample().build();
    let total = |rel| *tree.node(id(&tree, rel)).total();

    assert_eq!(total("").error_dirs, 1);
    assert_eq!(total("a").error_dirs, 1);
    assert_eq!(total("a/a2").error_dirs, 0, "its own error isn't inside it");

    assert_eq!(total("").boundaries, 1);
    assert_eq!(total("a").boundaries, 0);

    assert_eq!(
        total("").cloud_items,
        3,
        "one cloud-only dir and two dataless files"
    );
    assert_eq!(total("c").cloud_items, 2);

    assert_eq!(total("").linked_alloc, 8192);
    assert_eq!(total("a/a1").linked_alloc, 8192);
    assert_eq!(total("b").linked_alloc, 0);

    let mtime = |rel| tree.node(id(&tree, rel)).newest_mtime();
    assert_eq!(mtime(""), 18_000);
    assert_eq!(mtime("a"), 18_000);
    assert_eq!(mtime("b"), 12_000);
    assert_eq!(mtime("b/b1"), i64::MIN);
}

#[test]
fn candidates_errors_and_effects() {
    let builder = sample();
    let mut tree = builder.started();
    let mut privacy_effects = 0;
    let mut root_listed = 0;
    for listing in builder.listings() {
        let effect = tree.apply(listing);
        privacy_effects += u32::from(effect.first_privacy_error);
        root_listed += u32::from(effect.root_listed);
    }
    assert_eq!((privacy_effects, root_listed), (1, 1));

    let (b, a1, p) = (id(&tree, "b"), id(&tree, "a/a1"), id(&tree, "c/p"));
    assert!(tree.candidates().contains(&CandidateRef::Dir(b)));
    assert!(tree.candidates().contains(&CandidateRef::File(a1, 0)));
    assert!(
        !tree.candidates().iter().any(|c| c.dir() == p),
        "Protect is never a candidate"
    );
    assert_eq!(tree.candidates().len(), 2);

    let errors: Vec<_> = tree
        .errors()
        .iter()
        .map(|e| (tree.path_of(e.dir), e.entry.clone(), e.error.class))
        .collect();
    assert_eq!(
        errors,
        [
            (
                Path::new(ROOT).join("c"),
                Some(OsStr::new("x").into()),
                ErrorClass::PrivacyBlocked
            ),
            (
                Path::new(ROOT).join("a/a2"),
                None,
                ErrorClass::PermissionDenied
            ),
        ]
    );
}

#[test]
fn listing_an_expanded_node_touches_rows() {
    let builder = sample();
    let mut listings = builder.listings().into_iter();
    let mut tree = builder.started();
    tree.apply(listings.next().unwrap());
    let a = id(&tree, "a");
    tree.set_flag(a, NodeFlags::EXPANDED, true);
    let effect = tree.apply(listings.find(|l| l.id == a).unwrap());
    assert!(effect.touched_expanded);
    assert!(!effect.root_listed);
}

#[test]
fn vanished_directories_are_flagged_not_errors() {
    let tree = TreeBuilder::new(ROOT, dir("r").child(dir("v").vanished())).build();
    assert!(tree.node(id(&tree, "v")).has_flag(NodeFlags::VANISHED));
    assert!(tree.errors().is_empty());
}

#[test]
fn stop_turns_unfinished_sizes_into_lower_bounds() {
    let builder = sample();
    let mut tree = builder.started();
    for listing in builder.listings().into_iter().take(2) {
        tree.apply(listing);
    }
    let (a, mnt) = (id(&tree, "a"), id(&tree, "mnt"));
    assert!(!tree.is_lower_bound(NodeId::ROOT));
    tree.mark_stopped();
    assert!(tree.is_stopped());
    assert!(tree.is_lower_bound(NodeId::ROOT));
    assert!(tree.is_lower_bound(a));
    assert!(
        !tree.is_lower_bound(mnt),
        "skipped rows have no size to bound"
    );
}

#[test]
fn trashing_a_folder_leaves_the_ancestors() {
    let mut tree = sample().build();
    let (a, a1) = (id(&tree, "a"), id(&tree, "a/a1"));
    let root_before = *tree.node(NodeId::ROOT).total();
    let a_total = *tree.node(a).total();

    tree.mark_trashed(a);
    let root = *tree.node(NodeId::ROOT).total();
    assert_eq!(root.alloc, root_before.alloc - a_total.alloc);
    assert_eq!(root.items, root_before.items - a_total.items - 1);
    assert_eq!(root.error_dirs, 0);
    assert_eq!(root.linked_alloc, 0);
    assert_eq!(
        *tree.node(a).total(),
        a_total,
        "the struck-through row keeps its numbers"
    );
    assert!(tree.node(a).has_flag(NodeFlags::TRASHED));

    // Already accounted: the folder again, anything inside it.
    tree.mark_trashed(a);
    tree.mark_trashed(a1);
    tree.mark_file_trashed(a1, 0);
    tree.subtract_small_file(a1, 8192, 8192, true);
    assert_eq!(*tree.node(NodeId::ROOT).total(), root);
}

#[test]
fn trashing_files() {
    let mut tree = sample().build();
    let (b, b1) = (id(&tree, "b"), id(&tree, "b/b1"));
    let root_before = *tree.node(NodeId::ROOT).total();

    tree.mark_file_trashed(NodeId::ROOT, 0);
    tree.mark_file_trashed(NodeId::ROOT, 0);
    tree.mark_file_trashed(NodeId::ROOT, 99);
    let root = *tree.node(NodeId::ROOT).total();
    assert_eq!(root.alloc, root_before.alloc - 3 * MIB);
    assert_eq!(root.items, root_before.items - 1);
    assert_eq!(tree.node(NodeId::ROOT).files().count, 11 - 1);
    assert!(
        tree.node(NodeId::ROOT).large_files()[0]
            .flags
            .contains(FileFlags::TRASHED)
    );

    let b_before = *tree.node(b).total();
    tree.subtract_small_file(b1, 4096, 4096, false);
    assert_eq!(
        tree.node(b1).total().alloc,
        4096,
        "only the folder's own blocks are left"
    );
    assert_eq!(tree.node(b).total().alloc, b_before.alloc - 4096);
    assert_eq!(tree.node(NodeId::ROOT).total().alloc, root.alloc - 4096);

    // Saturates instead of wrapping.
    tree.subtract_small_file(b1, u64::MAX, u64::MAX, true);
    assert_eq!(tree.node(b1).total().alloc, 0);
    assert_eq!(tree.node(NodeId::ROOT).total().alloc, 0);
    assert_eq!(tree.node(NodeId::ROOT).total().linked_alloc, 0);
}

#[test]
fn trashing_contents() {
    let mut tree = sample().build();
    let (b, b1) = (id(&tree, "b"), id(&tree, "b/b1"));
    let root_before = *tree.node(NodeId::ROOT).total();
    let b_before = *tree.node(b).total();
    let outcome = ContentsOutcome {
        trashed_dirs: vec![OsStr::new("b1").into()],
        files_alloc: 1_000_000,
        files_apparent: 1_000_000,
        files_count: 100,
        ..ContentsOutcome::default()
    };
    tree.subtract_contents(b, &outcome);
    assert!(tree.node(b1).has_flag(NodeFlags::TRASHED));
    let b_now = tree.node(b).total();
    assert_eq!(b_now.alloc, 4096);
    assert_eq!(b_now.items, 0);
    assert_eq!(tree.node(b).files().count, 0);
    let root = tree.node(NodeId::ROOT).total();
    assert_eq!(root.alloc, root_before.alloc - (b_before.alloc - 4096));
    assert_eq!(root.items, root_before.items - b_before.items);
}

fn new_a() -> Dir {
    dir("a").small_files(1, 50_000).mtime(9_000).child(
        dir("new")
            .verdict(target(4))
            .small_files(2, 8000)
            .entry_error("y", ErrorClass::Other),
    )
}

fn link_summary(tree: &Tree, links: &[((u64, u64), &str)]) -> Summary {
    let links = links
        .iter()
        .map(|&(key, rel)| (key, id(tree, rel)))
        .collect();
    Summary {
        links: Arc::new(links),
        ..Summary::default()
    }
}

#[test]
fn splice_replaces_a_subtree() {
    let mut tree = sample().build();
    tree.finish(&link_summary(
        &tree,
        &[((1, 10), "a/a1"), ((1, 11), "b/b1")],
    ));
    let (a, a1, a2) = (id(&tree, "a"), id(&tree, "a/a1"), id(&tree, "a/a2"));
    assert_eq!(tree.counted_elsewhere(a), [(1, 11)].into());
    assert_eq!(tree.counted_elsewhere(NodeId::ROOT), [].into());

    let sub_builder = TreeBuilder::new(Path::new(ROOT).join("a"), new_a());
    let mut sub = sub_builder.build();
    sub.finish(&Summary {
        links: Arc::new(
            [(
                (1, 12),
                sub.find_path(&Path::new(ROOT).join("a/new")).unwrap(),
            )]
            .into(),
        ),
        ..Summary::default()
    });
    tree.set_flag(a, NodeFlags::EXPANDED, true);
    let effect = tree.splice(a, sub);

    assert_eq!(
        effect,
        SpliceEffect {
            removed: 2,
            became_done: false
        }
    );
    assert_eq!(snapshot(&tree), snapshot(&sample_with(new_a()).build()));
    assert!(tree.node(a).has_flag(NodeFlags::EXPANDED), "flags survive");
    assert_eq!(tree.node(a).name(), "a");
    assert_eq!(tree.node(a1).state(), NodeState::Removed);
    assert!(tree.get(a1).is_none() && tree.get(a2).is_none());
    assert!(tree.find_path(&Path::new(ROOT).join("a/a1")).is_none());

    let new = id(&tree, "a/new");
    assert_eq!(tree.node(new).parent(), Some(a));
    assert_eq!(tree.path_of(new), Path::new(ROOT).join("a/new"));
    assert_eq!(tree.candidates().len(), 2);
    assert!(
        tree.candidates()
            .contains(&CandidateRef::Dir(id(&tree, "b")))
    );
    assert!(tree.candidates().contains(&CandidateRef::Dir(new)));
    let error_dirs: Vec<_> = tree.errors().iter().map(|e| tree.path_of(e.dir)).collect();
    assert_eq!(
        error_dirs,
        [Path::new(ROOT).join("c"), Path::new(ROOT).join("a/new")]
    );
    assert_eq!(
        tree.links(),
        &[((1, 11), id(&tree, "b/b1")), ((1, 12), new)].into()
    );
    assert_eq!(tree.listed(), sample_with(new_a()).listings().len() as u64);

    // Tombstones take no accounting.
    let root = *tree.node(NodeId::ROOT).total();
    tree.mark_trashed(a1);
    assert_eq!(*tree.node(NodeId::ROOT).total(), root);
}

#[test]
fn splice_into_a_stopped_scan_completes_the_ancestors() {
    let builder = TreeBuilder::new(
        ROOT,
        dir("r").child(dir("x").small_files(1, 100)).child(dir("y")),
    );
    let mut tree = builder.started();
    for listing in builder.listings().into_iter().filter(|l| l.id != NodeId(2)) {
        tree.apply(listing);
    }
    tree.mark_stopped();
    let y = id(&tree, "y");
    assert_eq!(tree.node(y).state(), NodeState::Pending);
    assert!(!tree.node(NodeId::ROOT).is_done());

    let sub = TreeBuilder::new(Path::new(ROOT).join("y"), dir("y").small_files(3, 9000)).build();
    let effect = tree.splice(y, sub);
    assert!(effect.became_done);
    assert!(tree.node(y).is_done());
    assert!(tree.node(NodeId::ROOT).is_done());
    assert!(!tree.is_lower_bound(NodeId::ROOT));
    assert_eq!(tree.node(NodeId::ROOT).total().alloc, 3 * 4096 + 100 + 9000);
}

#[test]
fn compaction_preserves_totals_paths_and_references() {
    let mut tree = sample().build();
    tree.finish(&link_summary(&tree, &[((1, 11), "b/b1")]));
    let a = id(&tree, "a");
    tree.splice(
        a,
        TreeBuilder::new(Path::new(ROOT).join("a"), new_a()).build(),
    );
    assert!(!tree.needs_compaction(), "two tombstones are not worth it");

    let compact = tree.compacted();
    assert_eq!(compact.len(), tree.len() - 2);
    assert_eq!(snapshot(&compact), snapshot(&tree));
    let paths =
        |t: &Tree| -> Vec<PathBuf> { t.candidates().iter().map(|c| t.path_of(c.dir())).collect() };
    assert_eq!(paths(&compact), paths(&tree));
    let error_paths =
        |t: &Tree| -> Vec<PathBuf> { t.errors().iter().map(|e| t.path_of(e.dir)).collect() };
    assert_eq!(error_paths(&compact), error_paths(&tree));
    assert_eq!(compact.links(), &[((1, 11), id(&compact, "b/b1"))].into());
    for n in (0..compact.len()).map(NodeId) {
        assert!(compact.get(n).is_some(), "no tombstones left");
    }
}

#[cfg(unix)]
#[test]
fn paths_round_trip_with_non_utf8_names() {
    use std::os::unix::ffi::OsStrExt;
    let odd = OsStr::from_bytes(b"f\xffo");
    let tree = TreeBuilder::new(ROOT, dir("r").child(dir(odd).child(dir("deep")))).build();
    let deep = NodeId(2);
    let path = tree.path_of(deep);
    assert_eq!(path.as_os_str().as_bytes(), b"/scan/r/f\xffo/deep");
    assert_eq!(tree.find_path(&path), Some(deep));
    let rel = tree.relative_path(deep, NodeId::ROOT).unwrap();
    assert_eq!(rel, [Box::<OsStr>::from(odd), OsStr::new("deep").into()]);
    assert_eq!(tree.find_relative(NodeId::ROOT, &rel), Some(deep));
    assert_eq!(tree.relative_path(NodeId::ROOT, deep), None);
    assert!(tree.is_within(deep, NodeId(1)) && !tree.is_within(NodeId(1), deep));
    assert_eq!(
        tree.ancestors(deep).collect::<Vec<_>>(),
        [NodeId(1), NodeId::ROOT]
    );
    assert_eq!(tree.node(NodeId::ROOT).name(), "r");
    assert_eq!(tree.path_of(NodeId::ROOT), Path::new(ROOT));
}

#[test]
fn root_name_of_a_volume_root() {
    let tree = TreeBuilder::new("/", dir("/")).build();
    assert_eq!(tree.node(NodeId::ROOT).name(), "/");
    assert_eq!(tree.find_path(Path::new("/")), Some(NodeId::ROOT));
}
