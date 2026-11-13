//! The scanner: a parallel, work-stealing directory walker that streams one listing per
//! directory to the UI (design §4), and its event protocol (design §3.1).
//!
//! Guarantees the UI relies on:
//! 1. `Started` comes first, `Finished` last; after `Failed` nothing follows.
//! 2. Parent before child: a directory's listing arrives before any of its subdirectories'.
//! 3. Exactly once: every subdirectory announced without a `skip` gets one listing, unless
//!    the scan is cancelled.
//! 4. Sizes like `du -skx`: `st_blocks * 512`, hard links once, symlinks never followed,
//!    mount points and other filesystems below the root never entered.
//!
//! All event types are `Clone` because iced messages must be; they are never cloned on the
//! hot path (`Summary.links` sits behind an `Arc` for that reason).

mod batcher;
mod queue;
#[cfg(test)]
mod tests;
mod worker;

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub use batcher::{Delivery, ScanHandle, UpdateSink, start};

use crate::catalog::{self, RuleIdx};

pub struct Options {
    /// `available_parallelism().min(8)`: more threads are slower on macOS (SCAN §2.2).
    pub threads: usize,
    pub large_file_threshold: u64,
    /// At most one UI update per interval.
    pub batch_interval: Duration,
    pub engine: Arc<catalog::Engine>,
}

impl Options {
    pub fn new(engine: Arc<catalog::Engine>) -> Self {
        Options {
            threads: std::thread::available_parallelism()
                .map_or(4, |n| n.get())
                .min(8),
            large_file_threshold: LARGE_FILE,
            batch_interval: Duration::from_millis(50),
            engine,
        }
    }
}

/// Where the scan stops: mount points below the root are never entered or stat'ed.
#[derive(Clone, Debug)]
pub enum Boundaries {
    /// The mount table at scan start (design §4.7).
    System,
    /// Declared paths, for tests: fixture folders that act as mount points or excluded folders.
    Fixed {
        mounts: Vec<PathBuf>,
        excluded: Vec<PathBuf>,
    },
}

#[derive(Clone, Debug)]
pub struct ScanRequest {
    /// As given; canonicalized on the scan thread.
    pub root: PathBuf,
    /// `None`: computed on the scan thread. `Some`: rescans and tests.
    pub context: Option<catalog::RootContext>,
    pub boundaries: Boundaries,
    /// Subtree rescans: hard-linked inodes the rest of the tree already counts (D9).
    pub counted_elsewhere: Option<Arc<HashSet<(u64, u64)>>>,
}

impl ScanRequest {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        ScanRequest {
            root: root.into(),
            context: None,
            boundaries: Boundaries::System,
            counted_elsewhere: None,
        }
    }
}

/// A directory in the tree. Ids are dense and allocated in blocks by the scanner: the
/// subdirectories of one listing get consecutive ids starting at `DirListing::first_child`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct NodeId(pub u32);

impl NodeId {
    pub const ROOT: NodeId = NodeId(0);
    pub const NONE: NodeId = NodeId(u32::MAX);

    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// Sizes of non-directory entries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Allocated bytes (`st_blocks * 512`), hard links counted once.
    pub alloc: u64,
    pub apparent: u64,
    pub count: u64,
}

/// A catalog match on a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileMatch {
    Target(RuleIdx),
    Protected(RuleIdx),
}

/// A file with `max(alloc, apparent) >= LARGE_FILE`, reported individually.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub name: Box<OsStr>,
    pub alloc: u64,
    pub apparent: u64,
    /// Seconds since the epoch.
    pub mtime: i64,
    /// Identity re-check before trashing.
    pub ino: u64,
    /// `> 1`: trashing may free less than shown.
    pub nlink: u32,
    pub matched: Option<FileMatch>,
}

/// Files at least this large get their own `FileEntry`.
pub const LARGE_FILE: u64 = 1 << 20;

/// Why a subdirectory was announced but not scanned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// `st_dev` differs from the root's.
    OtherFilesystem,
    /// A mount-table entry (or its /System/Volumes/Data alias) below the root.
    MountPoint,
    /// macOS /.vol, /.nofollow, /.resolve when the root is "/"; Backups.backupdb, .zfs.
    Excluded,
    /// macOS dataless (cloud-only) directory: never read.
    CloudOnly,
    /// HFS+ directory hard link already counted elsewhere.
    LinkedDir,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subdir {
    pub name: Box<OsStr>,
    pub ino: u64,
    pub skip: Option<Skip>,
}

/// Errors are classified once, in core (FS §0 rule 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorClass {
    /// EPERM on macOS: TCC, SIP or the sandbox. Needs Full Disk Access.
    PrivacyBlocked,
    /// EACCES (and EPERM off macOS).
    PermissionDenied,
    /// EDEADLK: materialization of a cloud file was prevented.
    CloudNotDownloaded,
    /// ENAMETOOLONG.
    PathTooLong,
    /// A panic inside the worker was caught.
    Internal,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanError {
    pub class: ErrorClass,
    pub errno: Option<i32>,
}

/// One directory's direct contents.
#[derive(Clone, Debug)]
pub struct DirListing {
    pub id: NodeId,
    /// The directory's own blocks.
    pub dir_alloc: u64,
    pub files: Usage,
    /// Part of `files.alloc` in files with `nlink > 1`.
    pub linked_alloc: u64,
    /// Dataless (cloud-only) files among the direct entries.
    pub cloud_files: u32,
    pub large_files: Vec<FileEntry>,
    /// `subdirs[i]` has id `first_child + i`.
    pub first_child: NodeId,
    pub subdirs: Vec<Subdir>,
    /// The directory itself could not be (fully) read.
    pub error: Option<ScanError>,
    /// The directory disappeared before it was read. Not an error.
    pub vanished: bool,
    pub entry_errors: u32,
    /// The first few entries that could not be stat'ed.
    pub entry_error_samples: Vec<(Box<OsStr>, ScanError)>,
    /// Max mtime of the direct entries; `i64::MIN` if there are none.
    pub newest_mtime: i64,
    /// OR of the catalog marker bits of the entries.
    pub markers: u64,
    pub verdict: catalog::Verdict,
}

impl DirListing {
    /// An empty listing for `id`, to be filled in field by field.
    pub fn empty(id: NodeId, first_child: NodeId) -> Self {
        Self {
            id,
            dir_alloc: 0,
            files: Usage::default(),
            linked_alloc: 0,
            cloud_files: 0,
            large_files: Vec::new(),
            first_child,
            subdirs: Vec::new(),
            error: None,
            vanished: false,
            entry_errors: 0,
            entry_error_samples: Vec::new(),
            newest_mtime: i64::MIN,
            markers: 0,
            verdict: catalog::Verdict::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Progress {
    pub dirs_listed: u64,
    pub files: u64,
    pub alloc: u64,
    /// Unreadable directories plus entries that could not be stat'ed (vanished excluded).
    pub errors: u64,
    /// ENOENT/ENOTDIR/ELOOP: "changed during scan".
    pub vanished: u64,
    pub outstanding: u64,
    pub elapsed: Duration,
    /// The in-flight job shown in the activity field, and how long it has been running.
    pub current: Option<(NodeId, Duration)>,
}

#[derive(Clone, Debug)]
pub struct RootInfo {
    /// Canonical.
    pub root: PathBuf,
    pub dev: u64,
    pub ino: u64,
    pub fs_type: String,
    pub context: catalog::RootContext,
}

#[derive(Clone, Debug)]
pub struct StartError {
    pub kind: io::ErrorKind,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct Batch {
    pub listings: Vec<DirListing>,
    pub progress: Progress,
}

/// Owner of each hard-linked inode: `(dev, ino)` to the directory whose listing counted it.
/// Only inodes with `nlink > 1`.
pub type LinkOwners = HashMap<(u64, u64), NodeId>;

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub progress: Progress,
    pub cancelled: bool,
    pub hardlinks_deduped: u64,
    /// Caught panics.
    pub internal_errors: u32,
    pub links: Arc<LinkOwners>,
}

#[derive(Clone, Debug)]
pub enum Update {
    /// Always first on success.
    Started(RootInfo),
    /// The root could not be opened; nothing follows.
    Failed(StartError),
    Batch(Batch),
    /// Always last after `Started`.
    Finished(Summary),
}
