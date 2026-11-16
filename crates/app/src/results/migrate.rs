//! Carries the view state across a rescan splice, a tree replacement or a compaction
//! (design §4.6). Everything `NodeId`-keyed is captured by path from the scan root and
//! resolved again afterwards; a path that no longer resolves falls back to its nearest
//! surviving ancestor.

use std::ffi::OsStr;

use dirstat_core::git::GitStatus;
use dirstat_core::tree::{CandidateRef, NodeFlags, NodeId, Tree};

use super::Results;
use super::candidates::Candidates;
use super::rows::{ROW_H, RowKey};
use super::tree_view::Place;

/// Names below the scan root.
type Rel = Vec<Box<OsStr>>;

#[derive(Clone, Debug, PartialEq)]
enum SavedKey {
    Dir(Rel),
    /// A large file, re-found by name.
    File(Rel, Box<OsStr>),
}

#[derive(Debug)]
struct SavedPlace {
    zoom: Rel,
    selection: Option<SavedKey>,
    offset_y: f32,
}

#[derive(Debug)]
pub struct SavedView {
    zoom: Rel,
    selection: Option<SavedKey>,
    /// Where the selection was on screen, so it stays there.
    selection_y: Option<f32>,
    offset_y: f32,
    back: Vec<SavedPlace>,
    forward: Vec<SavedPlace>,
    expanded: Vec<Rel>,
    /// Git statuses of candidates outside the target; those inside are checked again.
    git: Vec<(SavedKey, GitStatus)>,
}

fn rel(tree: &Tree, id: NodeId) -> Rel {
    tree.relative_path(id, NodeId::ROOT).unwrap_or_default()
}

fn save_key(tree: &Tree, key: RowKey) -> SavedKey {
    match key {
        RowKey::File(dir, index) => match tree.node(dir).large_files().get(index as usize) {
            Some(file) => SavedKey::File(rel(tree, dir), file.name.clone()),
            None => SavedKey::Dir(rel(tree, dir)),
        },
        // Aggregates and small files are listed again on demand: keep their folder.
        other => SavedKey::Dir(rel(tree, other.dir())),
    }
}

/// The node at `rel`, or its nearest surviving ancestor; `true` if it resolved fully.
fn resolve(tree: &Tree, rel: &[Box<OsStr>]) -> (NodeId, bool) {
    let mut cur = NodeId::ROOT;
    for name in rel {
        match tree.find_relative(cur, std::slice::from_ref(name)) {
            Some(child) => cur = child,
            None => return (cur, false),
        }
    }
    (cur, true)
}

fn file_index(tree: &Tree, dir: NodeId, name: &OsStr) -> Option<u32> {
    tree.node(dir)
        .large_files()
        .iter()
        .position(|f| &*f.name == name)
        .map(|i| i as u32)
}

fn resolve_key(tree: &Tree, key: &SavedKey) -> RowKey {
    match key {
        SavedKey::Dir(rel) => RowKey::Dir(resolve(tree, rel).0),
        SavedKey::File(rel, name) => match resolve(tree, rel) {
            (dir, true) => file_index(tree, dir, name)
                .map_or(RowKey::Dir(dir), |index| RowKey::File(dir, index)),
            (dir, false) => RowKey::Dir(dir),
        },
    }
}

fn resolve_candidate(tree: &Tree, key: &SavedKey) -> Option<CandidateRef> {
    match key {
        SavedKey::Dir(rel) => match resolve(tree, rel) {
            (id, true) => Some(CandidateRef::Dir(id)),
            _ => None,
        },
        SavedKey::File(rel, name) => match resolve(tree, rel) {
            (dir, true) => file_index(tree, dir, name).map(|i| CandidateRef::File(dir, i)),
            _ => None,
        },
    }
}

fn save_candidate(tree: &Tree, candidate: CandidateRef) -> SavedKey {
    match candidate {
        CandidateRef::Dir(id) => SavedKey::Dir(rel(tree, id)),
        CandidateRef::File(dir, index) => save_key(tree, RowKey::File(dir, index)),
    }
}

impl SavedView {
    /// Before the swap; `target` is the rescanned folder.
    pub fn capture(results: &Results, target: NodeId) -> SavedView {
        let tree = &results.tree;
        let view = &results.view;
        let save_place = |p: &Place| SavedPlace {
            zoom: rel(tree, p.zoom),
            selection: p.selection.map(|k| save_key(tree, k)),
            offset_y: p.offset_y,
        };
        let selection_y = view.selection.and_then(|s| {
            let index = view.rows.iter().position(|r| r.key == s)?;
            let y = index as f32 * ROW_H - view.viewport.offset_y;
            (0.0..view.viewport.height).contains(&y).then_some(y)
        });
        let expanded = (0..tree.len())
            .map(NodeId)
            .filter(|&id| {
                tree.get(id)
                    .is_some_and(|n| n.has_flag(NodeFlags::EXPANDED))
            })
            .map(|id| rel(tree, id))
            .collect();
        let git = results
            .candidates
            .statuses()
            .filter(|(c, _)| tree.get(c.dir()).is_some() && !tree.is_within(c.dir(), target))
            .map(|(c, status)| (save_candidate(tree, c), status))
            .collect();
        SavedView {
            zoom: rel(tree, view.zoom),
            selection: view.selection.map(|k| save_key(tree, k)),
            selection_y,
            offset_y: view.viewport.offset_y,
            back: view.history.back.iter().map(save_place).collect(),
            forward: view.history.forward.iter().map(save_place).collect(),
            expanded,
            git,
        }
    }

    /// After the swap. `replaced`: every id changed (root rescan or compaction), so nothing
    /// id-keyed survives as is. Returns the offset to scroll to.
    pub fn restore(self, results: &mut Results, replaced: bool, now: std::time::Instant) -> f32 {
        let tree = &mut results.tree;
        for rel in &self.expanded {
            if let (id, true) = resolve(tree, rel) {
                tree.set_flag(id, NodeFlags::EXPANDED, true);
            }
        }
        let tree = &results.tree;
        let view = &mut results.view;
        let restore_place = |p: &SavedPlace| Place {
            zoom: resolve(tree, &p.zoom).0,
            selection: p.selection.as_ref().map(|k| resolve_key(tree, k)),
            offset_y: p.offset_y,
        };
        view.zoom = resolve(tree, &self.zoom).0;
        view.selection = self.selection.as_ref().map(|k| resolve_key(tree, k));
        view.history.back = self.back.iter().map(restore_place).collect();
        view.history.forward = self.forward.iter().map(restore_place).collect();
        view.flash = None;
        if replaced {
            view.open_aggregates.clear();
        } else {
            view.open_aggregates.retain(|&id, _| tree.get(id).is_some());
        }

        let known = self
            .git
            .iter()
            .filter_map(|(key, status)| Some((resolve_candidate(tree, key)?, *status)));
        // A job in flight still reports for the same ids after a plain splice; after a
        // replacement its results are dropped (the screen's scan id changes).
        let running = !replaced && results.candidates.is_running();
        results.candidates = Candidates::rebuilt(tree, known, running);
        if replaced {
            results.cleanup.keep_trashed.clear();
        }
        results.cleanup.cursor = None;
        view.rebuild_after_swap(tree, self.selection_y, self.offset_y, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::results::scan_task::{ActiveScan, ScanId};
    use dirstat_core::tree::testing::{TreeBuilder, dir, large};

    fn results(builder: &TreeBuilder) -> Results {
        let mut r = Results::new("/r".into(), "r".into(), None, ActiveScan::inert(ScanId(1)));
        r.tree = builder.build();
        r.active = None;
        r.phase = super::super::Phase::Complete;
        r.view.set_height(400.0);
        r
    }

    fn find(r: &Results, p: &str) -> NodeId {
        r.tree.find_path(std::path::Path::new(p)).unwrap()
    }

    fn before() -> TreeBuilder {
        TreeBuilder::new(
            "/r",
            dir("r")
                .child(
                    dir("a")
                        .child(dir("keep").child(dir("deep")))
                        .child(dir("gone"))
                        .large(large("big.iso", 50_000_000)),
                )
                .child(dir("b").child(dir("x"))),
        )
    }

    /// `a` after the rescan: `gone` was deleted, `new` appeared.
    fn a_after() -> Tree {
        TreeBuilder::new(
            "/r/a",
            dir("a")
                .child(dir("keep").child(dir("deep")))
                .child(dir("new"))
                .large(large("big.iso", 50_000_000)),
        )
        .build()
    }

    #[test]
    fn expansion_selection_zoom_and_history_survive_a_splice() {
        let mut r = results(&before());
        let now = std::time::Instant::now();
        let (a, keep, b) = (find(&r, "/r/a"), find(&r, "/r/a/keep"), find(&r, "/r/b"));
        r.tree.set_flag(keep, NodeFlags::EXPANDED, true);
        r.tree.set_flag(b, NodeFlags::EXPANDED, true);
        r.view.zoom_to(&r.tree, a, Some(RowKey::Dir(keep)), now);

        let saved = SavedView::capture(&r, a);
        r.tree.splice(a, a_after());
        saved.restore(&mut r, false, now);

        let (keep, deep) = (find(&r, "/r/a/keep"), find(&r, "/r/a/keep/deep"));
        assert_eq!(r.view.zoom, a, "the target keeps its id");
        assert_eq!(
            r.view.selection,
            Some(RowKey::Dir(keep)),
            "re-resolved by path"
        );
        assert!(r.tree.node(keep).has_flag(NodeFlags::EXPANDED));
        assert!(
            r.tree.node(b).has_flag(NodeFlags::EXPANDED),
            "outside: untouched"
        );
        assert!(r.view.rows.iter().any(|row| row.key == RowKey::Dir(deep)));
        assert!(r.view.history.can_go_back());
        assert_eq!(r.view.history.back[0].zoom, NodeId::ROOT);
    }

    #[test]
    fn vanished_keys_fall_back_to_the_nearest_ancestor_and_files_by_name() {
        let mut r = results(&before());
        let now = std::time::Instant::now();
        let (a, gone) = (find(&r, "/r/a"), find(&r, "/r/a/gone"));
        r.tree.set_flag(a, NodeFlags::EXPANDED, true);
        r.view.selection = Some(RowKey::Dir(gone));
        let saved = SavedView::capture(&r, a);
        r.tree.splice(a, a_after());
        saved.restore(&mut r, false, now);
        assert_eq!(r.view.selection, Some(RowKey::Dir(a)));

        // A large file: same name, new index position is looked up.
        r.view.selection = Some(RowKey::File(a, 0));
        let saved = SavedView::capture(&r, a);
        r.tree.splice(a, a_after());
        saved.restore(&mut r, false, now);
        assert!(matches!(r.view.selection, Some(RowKey::File(dir, 0)) if dir == a));
    }

    #[test]
    fn a_replacement_maps_everything_by_path() {
        let mut r = results(&before());
        let now = std::time::Instant::now();
        let x = find(&r, "/r/b/x");
        let b = find(&r, "/r/b");
        r.tree.set_flag(b, NodeFlags::EXPANDED, true);
        r.view.selection = Some(RowKey::Dir(x));
        let saved = SavedView::capture(&r, NodeId::ROOT);
        // Children in another order: every id differs.
        r.tree = TreeBuilder::new(
            "/r",
            dir("r")
                .child(dir("b").child(dir("x")))
                .child(dir("a").child(dir("keep"))),
        )
        .build();
        saved.restore(&mut r, true, now);
        let (b, x) = (find(&r, "/r/b"), find(&r, "/r/b/x"));
        assert!(r.tree.node(b).has_flag(NodeFlags::EXPANDED));
        assert_eq!(r.view.selection, Some(RowKey::Dir(x)));
        assert!(r.view.rows.iter().any(|row| row.key == RowKey::Dir(x)));
    }

    #[test]
    fn git_statuses_outside_the_target_are_kept() {
        use dirstat_core::catalog::{GitState, Verdict};
        let target = Verdict::Target {
            rule: 0,
            by_default: false,
        };
        let builder = TreeBuilder::new(
            "/r",
            dir("r").child(
                dir("repo")
                    .markers(dirstat_core::catalog::m::GIT)
                    .child(dir("in").child(dir("target").verdict(target)))
                    .child(dir("out").child(dir("target").verdict(target))),
            ),
        );
        let mut r = results(&builder);
        r.candidates.collect(&r.tree);
        let job = r.candidates.next_job().unwrap();
        r.candidates.finished(
            job.items
                .iter()
                .map(|&(c, _)| (c, GitStatus::Clean))
                .collect(),
        );
        let inside = find(&r, "/r/repo/in");
        let saved = SavedView::capture(&r, inside);
        r.tree.splice(
            inside,
            TreeBuilder::new("/r/repo/in", dir("in").child(dir("target").verdict(target))).build(),
        );
        saved.restore(&mut r, false, std::time::Instant::now());
        let state = |r: &Results, p: &str| r.candidates.state(CandidateRef::Dir(find(r, p)));
        assert_eq!(
            state(&r, "/r/repo/out/target"),
            GitState::Known(GitStatus::Clean)
        );
        assert_eq!(
            state(&r, "/r/repo/in/target"),
            GitState::Pending,
            "inside the target: checked again"
        );
    }
}
