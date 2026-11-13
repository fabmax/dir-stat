//! UI-side tree model: applies scanner batches incrementally.
//!
//! This is what the iced app keeps in its state. Applying a listing is O(depth) (size propagation
//! to the ancestors) + O(subdirs); completion propagation is amortised O(1) per directory.
//! Sorting is *not* maintained incrementally: the view sorts the children of the (few) expanded
//! nodes when it renders (or caches the order per node with a dirty flag).

use crate::scanner::{DirId, DirListing, FileEntry, ScanError, Skip, Usage, ROOT_ID};
use std::ffi::OsStr;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeState {
    /// Announced by the parent, listing not received yet.
    Pending,
    /// Listed, but some descendants are still pending.
    Scanning,
    /// This directory and all its descendants are complete: its size is final.
    Done,
    /// Shown but not scanned (mount point / other filesystem).
    Skipped(Skip),
}

#[derive(Debug)]
pub struct Node {
    pub parent: DirId, // u32::MAX for the root
    pub name: Box<OsStr>,
    pub state: NodeState,
    pub error: Option<ScanError>,
    /// Entries of this directory that could not be stat'ed.
    pub entry_errors: u32,
    /// Direct content only: files + the directory inode.
    pub own: Usage,
    /// Own + all descendants received so far (final once `state == Done`).
    pub total: Usage,
    /// Number of directories in this subtree (including itself) received so far.
    pub total_dirs: u64,
    pub first_child: DirId,
    pub child_count: u32,
    /// Children whose subtree is not complete yet.
    unfinished: u32,
    pub large_files: Vec<FileEntry>,
}

impl Node {
    fn vacant() -> Self {
        Node {
            parent: u32::MAX,
            name: OsStr::new("").into(),
            state: NodeState::Pending,
            error: None,
            entry_errors: 0,
            own: Usage::default(),
            total: Usage::default(),
            total_dirs: 0,
            first_child: 0,
            child_count: 0,
            unfinished: 0,
            large_files: Vec::new(),
        }
    }
}

pub struct TreeModel {
    pub root_path: PathBuf,
    /// Arena indexed by `DirId`. Ids are allocated in blocks, so a block announced by a listing
    /// that arrives early can leave short-lived vacant slots for a block announced by a listing
    /// still in flight; they are filled when that listing arrives (parents always come first).
    pub nodes: Vec<Node>,
    pub listed: u64,
}

impl TreeModel {
    pub fn new(root_path: PathBuf) -> Self {
        let mut root = Node::vacant();
        root.name = root_path.as_os_str().into();
        TreeModel { root_path, nodes: vec![root], listed: 0 }
    }

    pub fn root(&self) -> &Node {
        &self.nodes[ROOT_ID as usize]
    }

    pub fn apply(&mut self, l: DirListing) {
        let id = l.id as usize;
        let n = l.subdirs.len() as u32;
        let end = (l.first_child + n) as usize;
        if self.nodes.len() < end {
            self.nodes.resize_with(end, Node::vacant);
        }
        // 1) announce children (names come from the parent's listing)
        let mut unfinished = 0;
        for (i, sd) in l.subdirs.into_iter().enumerate() {
            let c = &mut self.nodes[l.first_child as usize + i];
            c.parent = l.id;
            c.name = sd.name;
            match sd.skip {
                Some(skip) => c.state = NodeState::Skipped(skip),
                None => unfinished += 1,
            }
        }
        // 2) fill in this node
        let mut own = l.files;
        own.alloc += l.dir_alloc;
        {
            let node = &mut self.nodes[id];
            debug_assert_eq!(node.state, NodeState::Pending, "listing delivered twice");
            node.own = own;
            node.error = l.error;
            node.entry_errors = l.entry_errors;
            node.first_child = l.first_child;
            node.child_count = n;
            node.unfinished = unfinished;
            node.large_files = l.large_files;
            node.state = NodeState::Scanning;
        }
        self.listed += 1;
        // 3) propagate sizes to all ancestors (O(depth))
        let mut cur = l.id;
        loop {
            let node = &mut self.nodes[cur as usize];
            node.total.add(&own);
            node.total_dirs += 1;
            if cur == ROOT_ID {
                break;
            }
            cur = node.parent;
        }
        // 4) completion: a node is done when it is listed and all its children are done
        if unfinished == 0 {
            self.complete(l.id);
        }
    }

    fn complete(&mut self, mut id: DirId) {
        loop {
            self.nodes[id as usize].state = NodeState::Done;
            if id == ROOT_ID {
                return;
            }
            let parent = self.nodes[id as usize].parent;
            let p = &mut self.nodes[parent as usize];
            p.unfinished -= 1;
            if p.unfinished > 0 || p.state != NodeState::Scanning {
                return;
            }
            id = parent;
        }
    }

    /// Children of `id` sorted by allocated size, largest first (what the view does on expand).
    pub fn children_by_size(&self, id: DirId) -> Vec<DirId> {
        let n = &self.nodes[id as usize];
        let mut v: Vec<DirId> = (n.first_child..n.first_child + n.child_count).collect();
        v.sort_unstable_by_key(|&c| std::cmp::Reverse(self.nodes[c as usize].total.alloc));
        v
    }

    /// Rebuild the absolute path of a node (for "Reveal in Finder", "Move to Trash", ...).
    pub fn path_of(&self, mut id: DirId) -> PathBuf {
        let mut parts = Vec::new();
        while id != ROOT_ID {
            parts.push(&*self.nodes[id as usize].name);
            id = self.nodes[id as usize].parent;
        }
        let mut p = self.root_path.clone();
        p.extend(parts.into_iter().rev());
        p
    }

    /// Consistency check used by the benchmark after a complete (non-cancelled) scan.
    pub fn check_complete(&self) -> Result<(), String> {
        for (i, n) in self.nodes.iter().enumerate() {
            match n.state {
                NodeState::Done | NodeState::Skipped(_) => {}
                s => return Err(format!("node {i} ({:?}) is {s:?} after the scan finished", self.path_of(i as u32))),
            }
            if n.unfinished != 0 {
                return Err(format!("node {i} has {} unfinished children", n.unfinished));
            }
        }
        Ok(())
    }
}
