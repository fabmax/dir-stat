//! The scanner's event protocol (design §3.1). The scanner itself arrives in M2.
//!
//! All event types are `Clone` because iced messages must be; they are never cloned on the
//! hot path (`Summary.links` sits behind an `Arc` for that reason).

use std::collections::HashMap;
use std::ffi::OsStr;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::catalog::{self, RuleIdx};

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
