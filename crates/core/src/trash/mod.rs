//! Move to Trash (design §7). `execute` runs the guards again right before every move;
//! nothing here ever deletes permanently (G8).

#[cfg(any(target_os = "linux", test))]
mod freedesktop;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::platform::{self, EntryKind};
use crate::safety::{self, Guards, Refusal};

/// What a path was at scan time; re-checked right before the move (G6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub dev: u64,
    pub ino: u64,
    pub kind: EntryKind,
    /// Windows only, where std has no stable file id (D12).
    pub len_mtime: Option<(u64, i64)>,
}

impl Identity {
    pub fn of(meta: &std::fs::Metadata) -> Identity {
        let m = platform::entry_meta(meta);
        let len_mtime =
            (cfg!(windows) && m.kind == EntryKind::File).then_some((m.apparent, m.mtime));
        Identity {
            dev: m.dev,
            ino: m.ino,
            kind: m.kind,
            len_mtime,
        }
    }

    pub fn matches(&self, now: &Identity) -> bool {
        self.kind == now.kind
            && self.dev == now.dev
            && self.ino == now.ino
            && self.len_mtime == now.len_mtime
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentsEntry {
    pub name: Box<OsStr>,
    pub expect: Identity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Move the item itself.
    Item,
    /// Move every unchanged direct child, keep the folder (D13).
    Contents {
        entries: Vec<ContentsEntry>,
        scan_started: SystemTime,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub path: PathBuf,
    pub expect: Identity,
    pub mode: Mode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrashJobId(pub u64);

/// A confirmed trash request, created only by the confirmation dialog.
#[derive(Clone, Debug)]
pub struct TrashJob {
    pub id: TrashJobId,
    /// Results of a job for a replaced tree must not touch the new one.
    pub tree_gen: u32,
    pub scan_root: PathBuf,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrashOutcome {
    Trashed(PathBuf),
    Contents {
        path: PathBuf,
        outcome: ContentsOutcome,
    },
    Failed(PathBuf, TrashError),
}

/// Result of trashing a folder's contents. The amounts are what was actually moved, so a
/// partial result is accounted exactly.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
    Refused(Refusal),
    /// The item changed since the scan (identity re-check failed).
    Changed,
    /// The volume has no usable Trash; dir-stat never deletes permanently.
    NoTrash,
    Os(String),
}

impl TrashError {
    /// The error toast's text (UX §9).
    pub fn message(&self, name: &str) -> String {
        match self {
            TrashError::Refused(refusal) => refusal.message(name),
            TrashError::Changed => {
                format!("“{name}” changed since the scan. Rescan, then try again.")
            }
            TrashError::NoTrash => {
                "This volume has no Trash. dir-stat doesn't delete files permanently.".into()
            }
            TrashError::Os(error) => format!("Couldn't move “{name}” to the Trash: {error}"),
        }
    }
}

/// Moves one path to the Trash. Implementations never delete permanently.
pub trait Trasher: Send + Sync {
    fn trash(&self, path: &Path) -> Result<(), TrashError>;
}

/// The OS Trash: NSFileManager on macOS (no Finder automation prompt), the Recycle Bin on
/// Windows, the rename-only freedesktop trash on Linux (D14).
pub struct SystemTrasher;

impl Trasher for SystemTrasher {
    fn trash(&self, path: &Path) -> Result<(), TrashError> {
        #[cfg(target_os = "macos")]
        {
            use trash::macos::{DeleteMethod, TrashContextExtMacos};
            let mut context = trash::TrashContext::default();
            context.set_delete_method(DeleteMethod::NsFileManager);
            context
                .delete(path)
                .map_err(|e| TrashError::Os(e.to_string()))
        }
        #[cfg(windows)]
        {
            trash::delete(path).map_err(|e| TrashError::Os(e.to_string()))
        }
        #[cfg(target_os = "linux")]
        {
            freedesktop::trash(path)
        }
        #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
        {
            let _ = path;
            Err(TrashError::NoTrash)
        }
    }
}

/// Runs a confirmed job. Every item passes the path guards and the live checks again.
pub fn execute(job: &TrashJob, guards: &Guards, trasher: &dyn Trasher) -> Vec<TrashOutcome> {
    platform::init_io_thread();
    job.items
        .iter()
        .map(|item| execute_item(job, item, guards, trasher))
        .collect()
}

fn execute_item(
    job: &TrashJob,
    item: &Item,
    guards: &Guards,
    trasher: &dyn Trasher,
) -> TrashOutcome {
    let failed = |error| TrashOutcome::Failed(item.path.clone(), error);
    if let Err(refusal) = safety::check(guards, &job.scan_root, &item.path) {
        return failed(TrashError::Refused(refusal));
    }
    if let Err(error) = safety::check_live(guards, &item.path, &item.expect) {
        return failed(error);
    }
    match &item.mode {
        Mode::Item => match trasher.trash(&item.path) {
            Ok(()) => TrashOutcome::Trashed(item.path.clone()),
            Err(error) => failed(error),
        },
        Mode::Contents {
            entries,
            scan_started,
        } => match trash_contents(job, item, entries, *scan_started, guards, trasher) {
            Ok(outcome) => TrashOutcome::Contents {
                path: item.path.clone(),
                outcome,
            },
            Err(error) => failed(error),
        },
    }
}

/// G9: each direct child that is unchanged since the scan goes to the Trash; new or changed
/// entries stay.
fn trash_contents(
    job: &TrashJob,
    item: &Item,
    planned: &[ContentsEntry],
    scan_started: SystemTime,
    guards: &Guards,
    trasher: &dyn Trasher,
) -> Result<ContentsOutcome, TrashError> {
    let folder = std::fs::symlink_metadata(&item.path).map_err(|_| TrashError::Changed)?;
    if platform::entry_meta(&folder).dataless {
        return Err(TrashError::Os(
            "the folder is in iCloud and not downloaded".into(),
        ));
    }
    let mut outcome = ContentsOutcome::default();
    let entries = std::fs::read_dir(&item.path).map_err(|e| TrashError::Os(e.to_string()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let now = Identity::of(&meta);
        let name = entry.file_name();
        let planned_match = planned
            .iter()
            .any(|p| *p.name == *name && p.expect.matches(&now));
        let raw = platform::entry_meta(&meta);
        let old_enough = meta.modified().is_ok_and(|m| m < scan_started);
        let eligible = match now.kind {
            EntryKind::Dir => planned_match,
            _ => planned_match || old_enough,
        };
        if !eligible {
            outcome.left_in_place += 1;
            continue;
        }
        let result = safety::check(guards, &job.scan_root, &path)
            .map_err(TrashError::Refused)
            .and_then(|()| safety::check_live(guards, &path, &now))
            .and_then(|()| trasher.trash(&path));
        match result {
            Ok(()) if now.kind == EntryKind::Dir => outcome.trashed_dirs.push(name.into()),
            Ok(()) => {
                outcome.files_alloc += raw.alloc;
                outcome.files_apparent += raw.apparent;
                outcome.files_count += 1;
            }
            Err(error) => outcome.failures.push((path, error)),
        }
    }
    Ok(outcome)
}

/// For tests: moves paths into a temp "trash" folder by renaming, records them, and fails
/// for chosen paths.
#[cfg(any(test, feature = "test-util"))]
pub struct RecordingTrasher {
    dir: PathBuf,
    pub moved: std::sync::Mutex<Vec<PathBuf>>,
    pub fail: std::sync::Mutex<Vec<PathBuf>>,
}

#[cfg(any(test, feature = "test-util"))]
impl RecordingTrasher {
    pub fn into_dir(dir: PathBuf) -> RecordingTrasher {
        RecordingTrasher {
            dir,
            moved: Default::default(),
            fail: Default::default(),
        }
    }
}

#[cfg(any(test, feature = "test-util"))]
impl Trasher for RecordingTrasher {
    fn trash(&self, path: &Path) -> Result<(), TrashError> {
        if self.fail.lock().unwrap().iter().any(|p| p == path) {
            return Err(TrashError::Os("Operation not permitted".into()));
        }
        let mut moved = self.moved.lock().unwrap();
        let target = self.dir.join(format!(
            "{}-{}",
            moved.len(),
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
        std::fs::rename(path, &target).map_err(|e| TrashError::Os(e.to_string()))?;
        moved.push(path.to_path_buf());
        Ok(())
    }
}

#[cfg(test)]
mod tests;
