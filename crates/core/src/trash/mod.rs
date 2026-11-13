//! Move to Trash (design §7). M1 only defines the outcome the tree accounts for; planning,
//! the guards and execution arrive in M8.

use std::ffi::OsStr;
use std::path::PathBuf;

/// Result of trashing a folder's contents (removal mode `Contents`). The amounts are what
/// was actually moved, so a partial result is accounted exactly.
#[derive(Clone, Debug, Default)]
pub struct ContentsOutcome {
    /// Scanned subdirectories that were moved; the tree subtracts their totals.
    pub trashed_dirs: Vec<Box<OsStr>>,
    pub files_alloc: u64,
    pub files_apparent: u64,
    pub files_count: u64,
    /// Entries that were new or changed since the scan and were not touched.
    pub left_in_place: u32,
    pub failures: Vec<(PathBuf, TrashError)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrashError {
    /// The item changed since the scan (identity re-check failed).
    Changed,
    /// The volume has no usable Trash; dir-stat never deletes permanently.
    NoTrash,
    Os(String),
}
