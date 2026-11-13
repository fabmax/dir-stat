//! Scan workers: list one directory per job (design §4.2, §4.3).

use std::collections::HashSet;
use std::ffi::OsStr;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::queue::StealingQueue;
use super::{
    DirListing, ErrorClass, FileEntry, LinkOwners, NodeId, Progress, ScanError, Skip, Subdir,
};
use crate::platform::{self, EntryKind};

/// Check the cancel flag (and publish the file counter) every this many entries.
const CANCEL_CHECK_EVERY: u32 = 256;
/// Entries that could not be stat'ed, kept by name per directory.
const MAX_ENTRY_ERROR_SAMPLES: usize = 4;
/// An in-flight job running this long is what the scan is waiting on.
const STUCK_AFTER: Duration = Duration::from_secs(1);

pub(super) struct Job {
    pub id: NodeId,
    pub path: PathBuf,
    /// The directory's own blocks, from the parent's stat of the entry.
    pub dir_alloc: u64,
}

/// Paths that are never stat'ed: mount points and excluded system folders below the root.
#[derive(Default)]
pub(super) struct BoundarySet {
    pub mounts: HashSet<PathBuf>,
    pub excluded: HashSet<PathBuf>,
    /// Parents of all of the above: only their listings build joined paths to check.
    pub parents: HashSet<PathBuf>,
}

impl BoundarySet {
    pub fn new(mounts: HashSet<PathBuf>, excluded: HashSet<PathBuf>) -> Self {
        let parents = mounts
            .iter()
            .chain(&excluded)
            .filter_map(|p| p.parent().map(Path::to_path_buf))
            .collect();
        BoundarySet {
            mounts,
            excluded,
            parents,
        }
    }

    fn skip_for(&self, path: &Path) -> Option<Skip> {
        if self.mounts.contains(path) {
            Some(Skip::MountPoint)
        } else if self.excluded.contains(path) {
            Some(Skip::Excluded)
        } else {
            None
        }
    }
}

#[derive(Default)]
pub(super) struct Counters {
    pub dirs_listed: AtomicU64,
    pub files: AtomicU64,
    pub alloc: AtomicU64,
    pub errors: AtomicU64,
    pub vanished: AtomicU64,
    pub hardlinks_deduped: AtomicU64,
    pub internal_errors: AtomicU32,
}

pub(super) struct Shared {
    pub threshold: u64,
    pub root_dev: u64,
    /// HFS+ allows directory hard links: count each directory once.
    pub hfs: bool,
    pub bounds: BoundarySet,
    pub counted_elsewhere: Option<Arc<HashSet<(u64, u64)>>>,
    pub queue: StealingQueue<Job>,
    pub cancelled: Arc<AtomicBool>,
    pub next_id: AtomicU32,
    /// Owner of each hard-linked inode seen so far.
    pub links: Mutex<LinkOwners>,
    /// HFS+ only: directories seen so far.
    pub dir_links: Mutex<HashSet<(u64, u64)>>,
    /// Per worker: `(id + 1) << 32 | start ms`, 0 when idle.
    pub in_flight: Vec<AtomicU64>,
    pub counters: Counters,
    pub started: Instant,
}

/// A worker that panicked while holding a lock must not take the scan down with it.
pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    pub fn cancel(&self) {
        self.cancelled.store(true, Relaxed);
        self.queue.wake_all();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Relaxed)
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64 & u64::from(u32::MAX)
    }

    pub fn progress(&self) -> Progress {
        let c = &self.counters;
        Progress {
            dirs_listed: c.dirs_listed.load(Relaxed),
            files: c.files.load(Relaxed),
            alloc: c.alloc.load(Relaxed),
            errors: c.errors.load(Relaxed),
            vanished: c.vanished.load(Relaxed),
            outstanding: self.queue.outstanding() as u64,
            elapsed: self.started.elapsed(),
            current: self.current(),
        }
    }

    /// The job the activity field shows: the oldest one if it has run for a second (the scan
    /// is waiting on it), else the most recently started one.
    fn current(&self) -> Option<(NodeId, Duration)> {
        let now = self.now_ms();
        let running: Vec<(NodeId, u64)> = self
            .in_flight
            .iter()
            .map(|slot| slot.load(Relaxed))
            .filter(|&v| v != 0)
            .map(|v| (NodeId((v >> 32) as u32 - 1), v & u64::from(u32::MAX)))
            .collect();
        let oldest = running.iter().min_by_key(|&&(_, start)| start)?;
        let pick = if now.saturating_sub(oldest.1) >= STUCK_AFTER.as_millis() as u64 {
            oldest
        } else {
            running.iter().max_by_key(|&&(_, start)| start)?
        };
        Some((pick.0, Duration::from_millis(now.saturating_sub(pick.1))))
    }
}

pub(super) fn run(shared: &Shared, me: usize, tx: &mpsc::Sender<DirListing>) {
    platform::init_io_thread();
    let mut children = Vec::new();
    while let Some(job) = shared.queue.pop(me, &shared.cancelled) {
        shared.in_flight[me].store((u64::from(job.id.0) + 1) << 32 | shared.now_ms(), Relaxed);
        let listing = catch_unwind(AssertUnwindSafe(|| list_one(shared, &job, &mut children)))
            .unwrap_or_else(|_| {
                // The termination protocol needs `complete()` for every job: report the
                // directory as unreadable instead of losing it.
                children.clear();
                shared.counters.internal_errors.fetch_add(1, Relaxed);
                shared.counters.errors.fetch_add(1, Relaxed);
                DirListing {
                    dir_alloc: job.dir_alloc,
                    error: Some(ScanError {
                        class: ErrorClass::Internal,
                        errno: None,
                    }),
                    ..DirListing::empty(job.id, NodeId::NONE)
                }
            });
        shared.in_flight[me].store(0, Relaxed);
        // Every listing counts, also vanished or unreadable ones: the tree adds them all.
        let counters = &shared.counters;
        counters.dirs_listed.fetch_add(1, Relaxed);
        counters
            .alloc
            .fetch_add(listing.dir_alloc + listing.files.alloc, Relaxed);
        // The listing goes out before the children become visible to other workers: that is
        // the parent-before-child guarantee.
        if tx.send(listing).is_err() {
            shared.cancel();
        }
        shared.queue.complete(me, &mut children);
    }
}

/// Lists one directory: aggregates its files, collects its subdirectories into `children`.
fn list_one(shared: &Shared, job: &Job, children: &mut Vec<Job>) -> DirListing {
    #[cfg(test)]
    if job.path.file_name() == Some(OsStr::new(super::tests::PANIC_TRIGGER)) {
        panic!("test panic in {}", job.path.display());
    }
    let mut listing = DirListing {
        dir_alloc: job.dir_alloc,
        ..DirListing::empty(job.id, NodeId::NONE)
    };
    let counters = &shared.counters;

    let entries = match std::fs::read_dir(&job.path) {
        Ok(entries) => entries,
        Err(e) if platform::is_vanished_dir(&e) => {
            listing.vanished = true;
            counters.vanished.fetch_add(1, Relaxed);
            return listing;
        }
        Err(e) => {
            listing.error = Some(scan_error(&e));
            counters.errors.fetch_add(1, Relaxed);
            return listing;
        }
    };

    let check_bounds = shared.bounds.parents.contains(&job.path);
    // (name, ino, skip, dir_alloc)
    let mut subdirs: Vec<(Box<OsStr>, u64, Option<Skip>, u64)> = Vec::new();
    let (mut seen, mut unpublished) = (0u32, 0u64);
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                // Reading failed part-way: what was listed so far still counts.
                listing.error = Some(scan_error(&e));
                counters.errors.fetch_add(1, Relaxed);
                break;
            }
        };
        seen += 1;
        if seen.is_multiple_of(CANCEL_CHECK_EVERY) {
            if shared.is_cancelled() {
                break;
            }
            counters
                .files
                .fetch_add(std::mem::take(&mut unpublished), Relaxed);
        }
        let name = entry.file_name();

        // Mount points and excluded folders are never stat'ed: a dead network mount would
        // hang the worker. d_type is free; an unknown type might be a directory.
        if check_bounds
            && entry.file_type().map_or(true, |t| t.is_dir())
            && let Some(skip) = shared.bounds.skip_for(&job.path.join(&name))
        {
            subdirs.push((name.into(), 0, Some(skip), 0));
            continue;
        }

        let meta = match entry.metadata() {
            Ok(m) => platform::entry_meta(&m),
            Err(e) if platform::is_vanished_entry(&e) => {
                counters.vanished.fetch_add(1, Relaxed);
                continue;
            }
            Err(e) => {
                listing.entry_errors += 1;
                counters.errors.fetch_add(1, Relaxed);
                if listing.entry_error_samples.len() < MAX_ENTRY_ERROR_SAMPLES {
                    listing
                        .entry_error_samples
                        .push((name.into(), scan_error(&e)));
                }
                continue;
            }
        };
        listing.newest_mtime = listing.newest_mtime.max(meta.mtime);

        if meta.kind == EntryKind::Dir {
            let skip = if meta.dev != shared.root_dev {
                Some(Skip::OtherFilesystem)
            } else if meta.dataless {
                Some(Skip::CloudOnly)
            } else if shared.hfs && !lock(&shared.dir_links).insert((meta.dev, meta.ino)) {
                Some(Skip::LinkedDir)
            } else {
                None
            };
            subdirs.push((name.into(), meta.ino, skip, meta.alloc));
            continue;
        }

        listing.files.count += 1;
        unpublished += 1;
        if meta.dataless {
            listing.cloud_files += 1;
        }
        let linked = meta.nlink > 1;
        if linked && !owns_link(shared, job.id, meta.dev, meta.ino) {
            continue;
        }
        listing.files.alloc += meta.alloc;
        listing.files.apparent += meta.apparent;
        if linked {
            listing.linked_alloc += meta.alloc;
        }
        if meta.alloc.max(meta.apparent) >= shared.threshold {
            listing.large_files.push(FileEntry {
                name: name.into(),
                alloc: meta.alloc,
                apparent: meta.apparent,
                mtime: meta.mtime,
                ino: meta.ino,
                nlink: meta.nlink.min(u64::from(u32::MAX)) as u32,
                matched: None,
            });
        }
    }

    // One fetch_add reserves a consecutive id block for all subdirectories.
    let first_child = shared.next_id.fetch_add(subdirs.len() as u32, Relaxed);
    listing.first_child = NodeId(first_child);
    for (i, (name, ino, skip, dir_alloc)) in subdirs.into_iter().enumerate() {
        if skip.is_none() {
            children.push(Job {
                id: NodeId(first_child + i as u32),
                path: job.path.join(&*name),
                dir_alloc,
            });
        }
        listing.subdirs.push(Subdir { name, ino, skip });
    }

    counters.files.fetch_add(unpublished, Relaxed);
    listing
}

/// A hard-linked inode is counted where it is seen first; a rescan counts inodes owned
/// outside its root as 0 bytes (D9).
fn owns_link(shared: &Shared, dir: NodeId, dev: u64, ino: u64) -> bool {
    if shared
        .counted_elsewhere
        .as_ref()
        .is_some_and(|set| set.contains(&(dev, ino)))
    {
        return false;
    }
    let mut links = lock(&shared.links);
    match links.entry((dev, ino)) {
        std::collections::hash_map::Entry::Vacant(slot) => {
            slot.insert(dir);
            true
        }
        std::collections::hash_map::Entry::Occupied(_) => {
            shared.counters.hardlinks_deduped.fetch_add(1, Relaxed);
            false
        }
    }
}

fn scan_error(e: &std::io::Error) -> ScanError {
    ScanError {
        class: platform::classify_error(e),
        errno: e.raw_os_error(),
    }
}
