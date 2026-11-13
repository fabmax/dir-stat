//! The scan result the UI keeps: one node per directory, built incrementally from scanner
//! listings (design §3.2). Large files live inside their parent node; small files exist only
//! as totals.
//!
//! The tree is a deep module: node fields are private and read through accessors. Outside
//! code changes only UI flags (`set_flag`, `set_file_flag`); sizes, counts and structure
//! change only through `apply`, the trash accounting and `splice` (`splice.rs`).

mod arena;
mod splice;
#[cfg(any(test, feature = "test-util"))]
pub mod testing;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arena::Arena;

use crate::catalog::{RootContext, Verdict};
pub use crate::scan::NodeId;
use crate::scan::{
    DirListing, ErrorClass, FileEntry, FileMatch, LinkOwners, RootInfo, ScanError, Skip, Summary,
    Usage,
};

pub use splice::SpliceEffect;

/// At most this many error records are kept (the errors popover, design §9).
pub const MAX_ERROR_RECORDS: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeState {
    /// Announced by the parent; its listing hasn't arrived.
    Pending,
    /// Listed, but some descendants are still pending.
    Scanning,
    /// This directory and everything below it is complete: its size is final.
    Done,
    /// Shown but never scanned.
    Skipped(Skip),
    /// Tombstone of a node replaced by a rescan.
    Removed,
}

/// Sums over a subtree. Everything except `alloc` and `apparent` counts what is strictly
/// inside the node.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    /// Directory blocks plus file blocks of every listed directory (hard links once).
    pub alloc: u64,
    pub apparent: u64,
    /// Part of `alloc` in files with `nlink > 1`; trashing may not free it.
    pub linked_alloc: u64,
    /// Files and directories (the "Items" column).
    pub items: u64,
    /// Unreadable directories.
    pub error_dirs: u32,
    /// Mount points and other filesystems; a folder containing one is never trashed.
    pub boundaries: u32,
    /// Cloud-only directories and dataless files.
    pub cloud_items: u32,
}

impl Totals {
    fn add(&mut self, other: &Totals) {
        self.alloc += other.alloc;
        self.apparent += other.apparent;
        self.linked_alloc += other.linked_alloc;
        self.items += other.items;
        self.error_dirs += other.error_dirs;
        self.boundaries += other.boundaries;
        self.cloud_items += other.cloud_items;
    }

    fn saturating_sub(&mut self, other: &Totals) {
        self.alloc = self.alloc.saturating_sub(other.alloc);
        self.apparent = self.apparent.saturating_sub(other.apparent);
        self.linked_alloc = self
            .linked_alloc
            .saturating_sub(other.linked_alloc)
            .min(self.alloc);
        self.items = self.items.saturating_sub(other.items);
        self.error_dirs = self.error_dirs.saturating_sub(other.error_dirs);
        self.boundaries = self.boundaries.saturating_sub(other.boundaries);
        self.cloud_items = self.cloud_items.saturating_sub(other.cloud_items);
    }
}

macro_rules! flags {
    ($(#[$meta:meta])* $name:ident { $($(#[$fmeta:meta])* $flag:ident = $bit:expr),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub struct $name(u8);

        impl $name {
            $($(#[$fmeta])* pub const $flag: $name = $name($bit);)*

            pub const fn contains(self, other: $name) -> bool {
                self.0 & other.0 == other.0
            }

            fn set(&mut self, other: $name, on: bool) {
                if on {
                    self.0 |= other.0;
                } else {
                    self.0 &= !other.0;
                }
            }
        }
    };
}

flags!(
    /// UI state of a directory row.
    NodeFlags {
        EXPANDED = 1,
        TRASHED = 2,
        TRASH_PENDING = 4,
        RESCANNING = 8,
        /// Contents changed outside the scan (e.g. ~/.Trash after a move): "Rescan to update".
        OUTDATED = 16,
        /// The directory disappeared before it was read.
        VANISHED = 32,
    }
);

flags!(
    /// UI state of a large-file row.
    FileFlags {
        TRASHED = 1,
        TRASH_PENDING = 2,
    }
);

/// Tree-side copy of a `FileEntry`, with UI flags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LargeFile {
    pub name: Box<OsStr>,
    pub alloc: u64,
    pub apparent: u64,
    pub mtime: i64,
    pub ino: u64,
    pub nlink: u32,
    pub matched: Option<FileMatch>,
    pub flags: FileFlags,
}

impl From<FileEntry> for LargeFile {
    fn from(f: FileEntry) -> Self {
        LargeFile {
            name: f.name,
            alloc: f.alloc,
            apparent: f.apparent,
            mtime: f.mtime,
            ino: f.ino,
            nlink: f.nlink,
            matched: f.matched,
            flags: FileFlags::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    /// `NodeId::NONE` for the root.
    parent: NodeId,
    first_child: NodeId,
    child_count: u32,
    /// Children that are not `Done` yet.
    unfinished: u32,
    /// Exact bytes from readdir, never normalized.
    name: Box<OsStr>,
    /// Identity for the trash re-check (dev is the scan root's).
    ino: u64,
    /// Catalog marker bits of the entries: rescan context, package detection, git root.
    markers: u64,
    dir_alloc: u64,
    /// Direct non-directory entries.
    files: Usage,
    /// Includes the node's own entries; final once `state == Done`.
    total: Totals,
    /// Max mtime of the direct entries, in minutes (see `Minutes`).
    own_mtime: Minutes,
    /// Max mtime over the subtree, in minutes.
    newest_mtime: Minutes,
    large_files: Box<[LargeFile]>,
    entry_errors: u32,
    error: Option<ScanError>,
    verdict: Verdict,
    state: NodeState,
    flags: NodeFlags,
}

impl Node {
    fn vacant() -> Self {
        Node {
            parent: NodeId::NONE,
            first_child: NodeId::NONE,
            child_count: 0,
            unfinished: 0,
            name: OsStr::new("").into(),
            ino: 0,
            markers: 0,
            dir_alloc: 0,
            files: Usage::default(),
            total: Totals::default(),
            own_mtime: Minutes::NONE,
            newest_mtime: Minutes::NONE,
            large_files: Box::new([]),
            entry_errors: 0,
            error: None,
            verdict: Verdict::None,
            state: NodeState::Pending,
            flags: NodeFlags::default(),
        }
    }

    pub fn parent(&self) -> Option<NodeId> {
        (self.parent != NodeId::NONE).then_some(self.parent)
    }
    pub fn name(&self) -> &OsStr {
        &self.name
    }
    pub fn ino(&self) -> u64 {
        self.ino
    }
    pub fn markers(&self) -> u64 {
        self.markers
    }
    pub fn child_count(&self) -> u32 {
        self.child_count
    }
    pub fn dir_alloc(&self) -> u64 {
        self.dir_alloc
    }
    pub fn files(&self) -> Usage {
        self.files
    }
    pub fn total(&self) -> &Totals {
        &self.total
    }
    /// Max mtime over the subtree; `i64::MIN` if nothing was listed.
    pub fn newest_mtime(&self) -> i64 {
        self.newest_mtime.to_secs()
    }
    pub fn large_files(&self) -> &[LargeFile] {
        &self.large_files
    }
    pub fn entry_errors(&self) -> u32 {
        self.entry_errors
    }
    pub fn error(&self) -> Option<ScanError> {
        self.error
    }
    pub fn verdict(&self) -> Verdict {
        self.verdict
    }
    pub fn state(&self) -> NodeState {
        self.state
    }
    pub fn flags(&self) -> NodeFlags {
        self.flags
    }
    pub fn has_flag(&self, flag: NodeFlags) -> bool {
        self.flags.contains(flag)
    }
    pub fn is_done(&self) -> bool {
        self.state == NodeState::Done
    }
    fn is_listed(&self) -> bool {
        matches!(self.state, NodeState::Scanning | NodeState::Done)
    }

    /// What this node adds to each strict ancestor's totals, apart from itself as an item.
    fn contribution(&self) -> Totals {
        let mut t = self.total;
        t.error_dirs += u32::from(self.error.is_some());
        match self.state {
            NodeState::Skipped(Skip::MountPoint | Skip::OtherFilesystem) => t.boundaries += 1,
            NodeState::Skipped(Skip::CloudOnly) => t.cloud_items += 1,
            _ => {}
        }
        t
    }
}

/// An mtime in whole minutes since the epoch: the UI shows nothing finer ("8 months ago",
/// "2026-03-14 18:22"), and it keeps `Node` within its size budget (design §3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Minutes(i32);

impl Minutes {
    /// Nothing listed.
    const NONE: Minutes = Minutes(i32::MIN);

    fn from_secs(secs: i64) -> Minutes {
        if secs == i64::MIN {
            return Minutes::NONE;
        }
        Minutes(
            secs.div_euclid(60)
                .clamp(i64::from(i32::MIN) + 1, i64::from(i32::MAX)) as i32,
        )
    }

    fn to_secs(self) -> i64 {
        if self == Minutes::NONE {
            i64::MIN
        } else {
            i64::from(self.0) * 60
        }
    }
}

/// An unreadable directory, or an entry inside it that could not be stat'ed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorRecord {
    pub dir: NodeId,
    pub entry: Option<Box<OsStr>>,
    pub error: ScanError,
}

/// A cleanup candidate: a directory, or a large file by index into its parent's files.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CandidateRef {
    Dir(NodeId),
    File(NodeId, u32),
}

impl CandidateRef {
    pub fn dir(self) -> NodeId {
        match self {
            CandidateRef::Dir(id) | CandidateRef::File(id, _) => id,
        }
    }

    fn map(self, f: impl Fn(NodeId) -> NodeId) -> CandidateRef {
        match self {
            CandidateRef::Dir(id) => CandidateRef::Dir(f(id)),
            CandidateRef::File(id, index) => CandidateRef::File(f(id), index),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApplyEffect {
    /// The listed node is expanded, so the visible rows change.
    pub touched_expanded: bool,
    pub new_candidate: bool,
    pub root_listed: bool,
    /// The first `PrivacyBlocked` error of this scan (shows the Full Disk Access bar).
    pub first_privacy_error: bool,
}

pub struct Tree {
    /// Canonical once started; the requested path while opening.
    root_path: PathBuf,
    root_dev: u64,
    root_fs_type: String,
    root_ctx: RootContext,
    nodes: Arena,
    /// Tombstones; the compaction trigger.
    removed: u32,
    candidates: Vec<CandidateRef>,
    errors: Vec<ErrorRecord>,
    links: Arc<LinkOwners>,
    listed: u64,
    stopped: bool,
    saw_privacy_error: bool,
}

impl Tree {
    /// A tree for a scan that hasn't started; it has no nodes until `start`.
    pub fn opening(requested: PathBuf) -> Tree {
        Tree {
            root_path: requested,
            root_dev: 0,
            root_fs_type: String::new(),
            root_ctx: RootContext::default(),
            nodes: Arena::default(),
            removed: 0,
            candidates: Vec::new(),
            errors: Vec::new(),
            links: Arc::default(),
            listed: 0,
            stopped: false,
            saw_privacy_error: false,
        }
    }

    /// The scan opened its root: store the canonical path and create the root node.
    pub fn start(&mut self, info: RootInfo) {
        assert_eq!(self.nodes.len(), 0, "tree started twice");
        let mut root = Node::vacant();
        root.name = root_display_name(&info.root).into();
        root.ino = info.ino;
        self.nodes.push(root);
        self.root_path = info.root;
        self.root_dev = info.dev;
        self.root_fs_type = info.fs_type;
        self.root_ctx = info.context;
    }

    pub fn apply(&mut self, listing: DirListing) -> ApplyEffect {
        let mut effect = ApplyEffect::default();
        let DirListing {
            id,
            dir_alloc,
            files,
            linked_alloc,
            cloud_files,
            large_files,
            first_child,
            subdirs,
            error,
            vanished,
            entry_errors,
            entry_error_samples,
            newest_mtime,
            markers,
            verdict,
        } = listing;
        let child_count = subdirs.len() as u32;

        // 1. Announce the children. Their block may lie beyond the current end, and a
        //    vacant slot below it may belong to a listing still in flight.
        self.nodes.ensure_len(id.0 + 1);
        if child_count > 0 {
            self.nodes.ensure_len(first_child.0 + child_count);
        }
        let (mut unfinished, mut boundaries, mut cloud_dirs) = (0, 0, 0);
        for (i, subdir) in subdirs.into_iter().enumerate() {
            let child = &mut self.nodes[NodeId(first_child.0 + i as u32)];
            child.parent = id;
            child.name = subdir.name;
            child.ino = subdir.ino;
            match subdir.skip {
                None => unfinished += 1,
                Some(skip) => {
                    child.state = NodeState::Skipped(skip);
                    match skip {
                        Skip::MountPoint | Skip::OtherFilesystem => boundaries += 1,
                        Skip::CloudOnly => cloud_dirs += 1,
                        Skip::Excluded | Skip::LinkedDir => {}
                    }
                }
            }
        }

        // 2. Fill in the node itself.
        let own = Totals {
            alloc: files.alloc + dir_alloc,
            apparent: files.apparent,
            linked_alloc,
            items: files.count + u64::from(child_count),
            error_dirs: 0,
            boundaries,
            cloud_items: cloud_files + cloud_dirs,
        };
        let node = &mut self.nodes[id];
        debug_assert_eq!(
            node.state,
            NodeState::Pending,
            "listing for {id:?} applied twice"
        );
        node.first_child = first_child;
        node.child_count = child_count;
        node.unfinished = unfinished;
        node.markers = markers;
        node.dir_alloc = dir_alloc;
        node.files = files;
        node.own_mtime = Minutes::from_secs(newest_mtime);
        node.large_files = large_files.into_iter().map(LargeFile::from).collect();
        node.entry_errors = entry_errors;
        node.error = error;
        node.verdict = verdict;
        node.state = NodeState::Scanning;
        node.flags.set(NodeFlags::VANISHED, vanished);
        effect.touched_expanded = node.flags.contains(NodeFlags::EXPANDED);
        effect.root_listed = id == NodeId::ROOT;
        self.listed += 1;

        // 3. Add to the node and every ancestor.
        let newest_mtime = Minutes::from_secs(newest_mtime);
        let mut cur = id;
        loop {
            let n = &mut self.nodes[cur];
            n.total.add(&own);
            if cur != id && error.is_some() {
                n.total.error_dirs += 1;
            }
            n.newest_mtime = n.newest_mtime.max(newest_mtime);
            if n.parent == NodeId::NONE {
                break;
            }
            cur = n.parent;
        }

        // 4. Completion.
        if unfinished == 0 {
            self.complete(id);
        }

        // 5. Candidates. Protect matches never are.
        if matches!(verdict, Verdict::Target { .. }) {
            self.candidates.push(CandidateRef::Dir(id));
            effect.new_candidate = true;
        }
        for (index, file) in self.nodes[id].large_files.iter().enumerate() {
            if matches!(file.matched, Some(FileMatch::Target(_))) {
                self.candidates.push(CandidateRef::File(id, index as u32));
                effect.new_candidate = true;
            }
        }

        // 6. Error records.
        let records = error.map(|error| (None, error)).into_iter().chain(
            entry_error_samples
                .into_iter()
                .map(|(name, error)| (Some(name), error)),
        );
        for (entry, error) in records {
            if error.class == ErrorClass::PrivacyBlocked && !self.saw_privacy_error {
                self.saw_privacy_error = true;
                effect.first_privacy_error = true;
            }
            self.push_error(ErrorRecord {
                dir: id,
                entry,
                error,
            });
        }
        effect
    }

    /// Marks `id` done and propagates completion upward (amortized O(1)).
    fn complete(&mut self, id: NodeId) {
        self.nodes[id].state = NodeState::Done;
        self.propagate_done(id);
    }

    /// `id` just became done: finish every ancestor whose last unfinished child it was.
    fn propagate_done(&mut self, mut id: NodeId) {
        while let Some(parent) = self.nodes[id].parent() {
            let p = &mut self.nodes[parent];
            p.unfinished -= 1;
            if p.unfinished > 0 || p.state != NodeState::Scanning {
                return;
            }
            p.state = NodeState::Done;
            id = parent;
        }
    }

    fn push_error(&mut self, record: ErrorRecord) {
        if self.errors.len() < MAX_ERROR_RECORDS {
            self.errors.push(record);
        }
    }

    /// Stop was pressed: sizes of nodes that aren't done are lower bounds from now on.
    pub fn mark_stopped(&mut self) {
        self.stopped = true;
    }

    /// The scan finished: keep the hard-link owners for later rescans.
    pub fn finish(&mut self, summary: &Summary) {
        self.links = summary.links.clone();
    }

    // ---- reads ----

    pub fn root_path(&self) -> &Path {
        &self.root_path
    }
    pub fn root_dev(&self) -> u64 {
        self.root_dev
    }
    pub fn root_fs_type(&self) -> &str {
        &self.root_fs_type
    }
    pub fn root_context(&self) -> &RootContext {
        &self.root_ctx
    }
    pub fn is_started(&self) -> bool {
        self.nodes.len() > 0
    }
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }
    /// Number of slots, tombstones included.
    pub fn len(&self) -> u32 {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.len() == 0
    }
    pub fn listed(&self) -> u64 {
        self.listed
    }
    pub fn links(&self) -> &LinkOwners {
        &self.links
    }

    /// The size shown for `id` is only a lower bound ("≥ 12.3 GB").
    pub fn is_lower_bound(&self, id: NodeId) -> bool {
        self.stopped
            && !matches!(
                self.nodes[id].state,
                NodeState::Done | NodeState::Skipped(_)
            )
    }

    /// Panics on an id out of range.
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    /// `None` for an id out of range or a tombstone. The view resolves keys with this.
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id).filter(|n| n.state != NodeState::Removed)
    }

    pub fn children(&self, id: NodeId) -> impl ExactSizeIterator<Item = NodeId> + use<> {
        let node = &self.nodes[id];
        let first = if node.child_count == 0 {
            0
        } else {
            node.first_child.0
        };
        (first..first + node.child_count).map(NodeId)
    }

    /// Parent, grandparent, …, root.
    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.nodes[id].parent(), |&p| self.nodes[p].parent())
    }

    /// `id` is `ancestor` or lies below it. O(depth).
    pub fn is_within(&self, id: NodeId, ancestor: NodeId) -> bool {
        id == ancestor || self.ancestors(id).any(|a| a == ancestor)
    }

    pub fn path_of(&self, id: NodeId) -> PathBuf {
        let mut names: Vec<&OsStr> = std::iter::once(id)
            .chain(self.ancestors(id))
            .filter(|&a| a != NodeId::ROOT)
            .map(|a| &*self.nodes[a].name)
            .collect();
        names.reverse();
        let mut path = self.root_path.clone();
        path.extend(names);
        path
    }

    /// Names from below `base` down to `id`; `None` if `id` isn't within `base`.
    pub fn relative_path(&self, id: NodeId, base: NodeId) -> Option<Vec<Box<OsStr>>> {
        let mut names = Vec::new();
        let mut cur = id;
        while cur != base {
            names.push(self.nodes[cur].name.clone());
            cur = self.nodes[cur].parent()?;
        }
        names.reverse();
        Some(names)
    }

    /// Follows `rel` by name from `base`.
    pub fn find_relative(&self, base: NodeId, rel: &[Box<OsStr>]) -> Option<NodeId> {
        rel.iter()
            .try_fold(base, |dir, name| self.child_named(dir, name))
    }

    /// The node at an absolute path below (or at) the scan root.
    pub fn find_path(&self, path: &Path) -> Option<NodeId> {
        if !self.is_started() {
            return None;
        }
        let rel = path.strip_prefix(&self.root_path).ok()?;
        rel.components()
            .try_fold(NodeId::ROOT, |dir, component| match component {
                std::path::Component::Normal(name) => self.child_named(dir, name),
                _ => None,
            })
    }

    fn child_named(&self, dir: NodeId, name: &OsStr) -> Option<NodeId> {
        self.get(dir)?;
        self.children(dir).find(|&c| &*self.nodes[c].name == name)
    }

    pub fn candidates(&self) -> &[CandidateRef] {
        &self.candidates
    }

    pub fn errors(&self) -> &[ErrorRecord] {
        &self.errors
    }

    /// Hard-linked inodes whose owner lies outside `target`: a rescan of `target` must count
    /// them as 0 bytes, because the rest of the tree already counts them.
    pub fn counted_elsewhere(&self, target: NodeId) -> HashSet<(u64, u64)> {
        self.links
            .iter()
            .filter(|&(_, &owner)| !self.is_within(owner, target))
            .map(|(&key, _)| key)
            .collect()
    }

    // ---- UI flags ----

    pub fn set_flag(&mut self, id: NodeId, flag: NodeFlags, on: bool) {
        self.nodes[id].flags.set(flag, on);
    }

    pub fn set_file_flag(&mut self, dir: NodeId, index: u32, flag: FileFlags, on: bool) {
        if let Some(file) = self.nodes[dir].large_files.get_mut(index as usize) {
            file.flags.set(flag, on);
        }
    }
}

/// The root row's name: the last path component, or the whole path for "/" or "C:\".
fn root_display_name(root: &Path) -> &OsStr {
    root.file_name().unwrap_or(root.as_os_str())
}
