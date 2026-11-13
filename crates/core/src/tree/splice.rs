//! Changes after the scan: trash accounting (design §7.4), the rescan splice and compaction
//! (design §4.6). All subtraction saturates.

use std::ffi::OsStr;
use std::sync::Arc;

use super::{Arena, ErrorRecord, FileFlags, Node, NodeFlags, NodeState, Totals, Tree};
use crate::scan::NodeId;
use crate::trash::ContentsOutcome;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpliceEffect {
    /// Nodes of the old subtree that became tombstones.
    pub removed: u32,
    /// The target wasn't done before (the main scan was stopped) and is now.
    pub became_done: bool,
}

impl Tree {
    // ---- trash accounting ----

    /// The folder was moved to the Trash: its totals leave every ancestor. The row stays.
    pub fn mark_trashed(&mut self, id: NodeId) {
        if self.accounting_blocked(id) {
            return;
        }
        let node = &mut self.nodes[id];
        node.flags.set(NodeFlags::TRASHED, true);
        node.flags.set(NodeFlags::TRASH_PENDING, false);
        let mut gone = node.contribution();
        gone.items += 1;
        self.subtract_from_ancestors(id, &gone);
    }

    /// The large file `index` of `dir` was moved to the Trash.
    pub fn mark_file_trashed(&mut self, dir: NodeId, index: u32) {
        if self.accounting_blocked(dir) {
            return;
        }
        let Some(file) = self.nodes[dir].large_files.get_mut(index as usize) else {
            return;
        };
        if file.flags.contains(FileFlags::TRASHED) {
            return;
        }
        file.flags.set(FileFlags::TRASHED, true);
        file.flags.set(FileFlags::TRASH_PENDING, false);
        let (alloc, apparent, linked) = (file.alloc, file.apparent, file.nlink > 1);
        self.subtract_files(dir, alloc, apparent, 1, linked);
    }

    /// A small file of `dir` (from an on-demand listing) was moved to the Trash.
    pub fn subtract_small_file(&mut self, dir: NodeId, alloc: u64, apparent: u64, linked: bool) {
        if !self.accounting_blocked(dir) {
            self.subtract_files(dir, alloc, apparent, 1, linked);
        }
    }

    /// The contents of `id` were moved to the Trash: trashed subfolders by their tree totals,
    /// files by the measured amounts.
    pub fn subtract_contents(&mut self, id: NodeId, outcome: &ContentsOutcome) {
        if self.accounting_blocked(id) {
            return;
        }
        for name in &outcome.trashed_dirs {
            if let Some(child) = self.children(id).find(|&c| self.nodes[c].name() == &**name) {
                self.mark_trashed(child);
            }
        }
        self.subtract_files(
            id,
            outcome.files_alloc,
            outcome.files_apparent,
            outcome.files_count,
            false,
        );
    }

    /// Removed nodes, and nodes at or below a trashed folder, are already accounted for.
    fn accounting_blocked(&self, id: NodeId) -> bool {
        let Some(node) = self.get(id) else {
            return true;
        };
        node.has_flag(NodeFlags::TRASHED)
            || self
                .ancestors(id)
                .any(|a| self.nodes[a].has_flag(NodeFlags::TRASHED))
    }

    /// Files directly in `dir` are gone: `dir` and all its ancestors shrink.
    fn subtract_files(&mut self, dir: NodeId, alloc: u64, apparent: u64, count: u64, linked: bool) {
        let files = &mut self.nodes[dir].files;
        files.alloc = files.alloc.saturating_sub(alloc);
        files.apparent = files.apparent.saturating_sub(apparent);
        files.count = files.count.saturating_sub(count);
        let gone = Totals {
            alloc,
            apparent,
            linked_alloc: if linked { alloc } else { 0 },
            items: count,
            ..Totals::default()
        };
        self.nodes[dir].total.saturating_sub(&gone);
        self.subtract_from_ancestors(dir, &gone);
    }

    fn subtract_from_ancestors(&mut self, id: NodeId, gone: &Totals) {
        let mut cur = self.nodes[id].parent();
        while let Some(a) = cur {
            self.nodes[a].total.saturating_sub(gone);
            cur = self.nodes[a].parent();
        }
    }

    fn add_to_ancestors(&mut self, id: NodeId, added: &Totals) {
        let mut cur = self.nodes[id].parent();
        while let Some(a) = cur {
            self.nodes[a].total.add(added);
            cur = self.nodes[a].parent();
        }
    }

    // ---- rescan ----

    /// Replaces the subtree at `at` with `sub`, the finished, complete scan of the same
    /// folder. The rescan of the scan root replaces the whole tree instead.
    pub fn splice(&mut self, at: NodeId, sub: Tree) -> SpliceEffect {
        assert!(at != NodeId::ROOT, "a rescan of the root replaces the tree");
        assert!(
            self.get(at).is_some(),
            "splice target {at:?} is not in the tree"
        );
        debug_assert!(
            sub.is_started() && sub.node(NodeId::ROOT).is_done(),
            "sub-tree incomplete"
        );
        let was_done = self.nodes[at].is_done();

        // 1. The old subtree leaves the ancestors.
        let old = self.nodes[at].contribution();
        self.subtract_from_ancestors(at, &old);

        // 7, 8 (first half). Drop what lies inside the target while parent links still work.
        let mut candidates = std::mem::take(&mut self.candidates);
        candidates.retain(|c| !self.is_within(c.dir(), at));
        self.candidates = candidates;
        let mut errors = std::mem::take(&mut self.errors);
        errors.retain(|e| !self.is_within(e.dir, at));
        self.errors = errors;
        let mut links = std::mem::take(&mut self.links);
        Arc::make_mut(&mut links).retain(|_, owner| !self.is_within(*owner, at));

        // 2. Tombstone the old descendants. Their slots are not reused.
        let mut removed = 0;
        let mut listed_gone = u64::from(self.nodes[at].is_listed());
        let mut stack: Vec<NodeId> = self.children(at).collect();
        while let Some(id) = stack.pop() {
            stack.extend(self.children(id));
            let node = &mut self.nodes[id];
            listed_gone += u64::from(node.is_listed());
            node.state = NodeState::Removed;
            node.name = OsStr::new("").into();
            node.large_files = Box::new([]);
            removed += 1;
        }
        self.removed += removed;

        // 3, 4. Move the sub-tree's nodes in: sub-root 0 becomes `at`, the rest are appended.
        let offset = self.nodes.len() - 1;
        let map = |id: NodeId| match id {
            NodeId::ROOT => at,
            NodeId::NONE => NodeId::NONE,
            NodeId(n) => NodeId(n + offset),
        };
        let Tree {
            nodes: sub_nodes,
            candidates: sub_candidates,
            errors: sub_errors,
            links: sub_links,
            listed: sub_listed,
            saw_privacy_error,
            ..
        } = sub;
        for (index, mut node) in sub_nodes.into_nodes().enumerate() {
            node.first_child = map(node.first_child);
            if index == 0 {
                let target = &mut self.nodes[at];
                node.parent = target.parent;
                node.name = std::mem::take(&mut target.name);
                node.flags = target.flags;
                *target = node;
            } else {
                node.parent = map(node.parent);
                self.nodes.push(node);
            }
        }
        self.listed = self.listed - listed_gone + sub_listed;
        self.saw_privacy_error |= saw_privacy_error;

        // 5. The new subtree joins the ancestors; newest_mtime may have gone down.
        let new = self.nodes[at].contribution();
        self.add_to_ancestors(at, &new);
        let ancestors: Vec<NodeId> = self.ancestors(at).collect();
        for a in ancestors {
            let newest = self
                .children(a)
                .map(|c| self.nodes[c].newest_mtime)
                .fold(self.nodes[a].own_mtime, Ord::max);
            self.nodes[a].newest_mtime = newest;
        }

        // 6. Completion: a stopped scan's last unfinished folder may now be complete.
        let became_done = !was_done && self.nodes[at].is_done();
        if became_done {
            self.propagate_done(at);
        }

        // 7, 8 (second half). Add the sub-tree's candidates, errors and link owners.
        self.candidates
            .extend(sub_candidates.into_iter().map(|c| c.map(map)));
        for record in sub_errors {
            self.push_error(ErrorRecord {
                dir: map(record.dir),
                ..record
            });
        }
        Arc::make_mut(&mut links).extend(sub_links.iter().map(|(&key, &owner)| (key, map(owner))));
        self.links = links;

        SpliceEffect {
            removed,
            became_done,
        }
    }

    /// Tombstones are more than a quarter of the arena and more than 100k nodes.
    pub fn needs_compaction(&self) -> bool {
        self.removed > 100_000 && self.removed > self.nodes.len() / 4
    }

    /// A copy without tombstones. Each node's children get a fresh consecutive block, so ids
    /// change; candidates, errors and link owners are remapped.
    pub fn compacted(&self) -> Tree {
        let mut nodes = Arena::default();
        let mut map = vec![NodeId::NONE; self.nodes.len() as usize];
        if self.is_started() {
            nodes.push(self.nodes[NodeId::ROOT].clone());
            map[0] = NodeId::ROOT;
            let mut stack = vec![(NodeId::ROOT, NodeId::ROOT)];
            while let Some((old, new)) = stack.pop() {
                let first = NodeId(nodes.len());
                for child in self.children(old) {
                    debug_assert_ne!(self.nodes[child].state, NodeState::Removed);
                    let mut node: Node = self.nodes[child].clone();
                    node.parent = new;
                    let id = nodes.push(node);
                    map[child.index()] = id;
                    stack.push((child, id));
                }
                nodes[new].first_child = if self.nodes[old].child_count > 0 {
                    first
                } else {
                    NodeId::NONE
                };
            }
        }
        let map = |id: NodeId| map[id.index()];
        Tree {
            root_path: self.root_path.clone(),
            root_dev: self.root_dev,
            root_fs_type: self.root_fs_type.clone(),
            root_ctx: self.root_ctx.clone(),
            nodes,
            removed: 0,
            candidates: self.candidates.iter().map(|c| c.map(map)).collect(),
            errors: self
                .errors
                .iter()
                .map(|e| ErrorRecord {
                    dir: map(e.dir),
                    ..e.clone()
                })
                .collect(),
            links: Arc::new(
                self.links
                    .iter()
                    .map(|(&key, &owner)| (key, map(owner)))
                    .collect(),
            ),
            listed: self.listed,
            stopped: self.stopped,
            saw_privacy_error: self.saw_privacy_error,
        }
    }
}
