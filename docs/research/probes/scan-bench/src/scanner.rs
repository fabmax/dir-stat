//! # dir-stat scanning engine - reference implementation
//!
//! A small, dependency-free (std + libc) parallel directory walker that *streams* one result per
//! directory to the UI, so the tree view can be built while the scan is still running.
//!
//! ```text
//!   start(root) -> Job { id: 0, path: root }
//!                       |
//!     +-----------------v-------------------+   pop own back /     +------------+
//!     | work queue (queue.rs)               |   steal others' front | worker x N |  read_dir + 1 stat per
//!     |  per-worker VecDeque<Job> (default) | --------------------> |            |  entry, aggregate files,
//!     |  outstanding = queued + in-flight   | <-------------------- |            |  collect subdirs
//!     +-------------------------------------+  push own subdirs     +-----+------+
//!                                              (AFTER the send)           | DirListing: 1 message per directory
//!                                                                         v std::sync::mpsc (unbounded, never blocks)
//!                                                                  +--------------+
//!                                                                  | batcher (x1) |  coalesces, flushes every 50 ms
//!                                                                  +------+-------+
//!                                                                         v UpdateSink::try_deliver (non-blocking)
//!                                                                  UI thread: TreeModel::apply(batch)
//! ```
//!
//! ## Guarantees the UI can rely on
//! 1. **Parent before child.** A directory's [`DirListing`] is delivered before the listing of any
//!    of its subdirectories: a worker sends the listing *before* it makes the child jobs visible to
//!    other workers, and the channel preserves that happens-before order.
//! 2. **Exactly once.** Every subdirectory announced with `skip == None` receives exactly one
//!    [`DirListing`] later (possibly one with `error: Some(..)`), unless the scan is cancelled.
//!    This is what makes per-subtree completion tracking possible on the UI side (see `model.rs`).
//! 3. **Dense ids.** The root is `0`; a listing with `n` subdirs owns ids
//!    `first_child .. first_child + n` (allocated with a single `fetch_add`). Skipped subdirs
//!    (mount points) get an id too, so the UI can show them, but never get a listing.
//! 4. **du semantics.** total(dir) = `dir_alloc` + `files.alloc` + sum(total(subdir)), where
//!    alloc = `st_blocks * 512`, hard links (nlink > 1) are counted once per scan, symlinks are
//!    never followed (they count with their own, usually zero, blocks), and mount points below
//!    the root are not crossed. This matches `du -skx` exactly (see `verify` in main.rs).
//!
//! On macOS every worker also sets per-thread I/O policies (no iCloud/File Provider
//! materialisation, no atime writes), see `macos_thread_io_policies`.
//! 5. **Bounded message rate.** The UI receives at most one [`Update::Batch`] per `batch_interval`
//!    (default 50 ms = 20/s). If the UI is still busy with the previous batch, the batcher keeps
//!    accumulating into the next one (coalescing backpressure); workers never block on the UI.

use crate::listing::{self, Backend, Kind};
pub use crate::queue::Schedule;
use crate::queue::WorkQueue;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Index of a directory in the scan (and in the UI's arena). `u32` is plenty (4 G directories).
pub type DirId = u32;
pub const ROOT_ID: DirId = 0;

/// How often (at most) the UI is updated.
pub const DEFAULT_BATCH_INTERVAL: Duration = Duration::from_millis(50);
/// Files at least this large are reported individually; smaller ones only as per-dir aggregate.
pub const DEFAULT_LARGE_FILE_THRESHOLD: u64 = 1 << 20; // 1 MiB
/// How many inaccessible paths are kept for the "N folders could not be read" dialog.
const MAX_ERROR_SAMPLES: usize = 200;
/// Check the cancel flag every this many entries inside (huge) directories.
const CANCEL_CHECK_EVERY: u32 = 256;

#[derive(Clone, Debug)]
pub struct Options {
    /// Worker threads. Default `available_parallelism().min(8)`: on macOS more than 8 threads is
    /// slower (kernel contention), see scan-engine.md section 2.2.
    pub threads: usize,
    /// How directories are listed (`std` everywhere; `bulk` = getattrlistbulk on macOS).
    pub backend: Backend,
    /// Queue discipline, see `queue.rs` (benchmark: section 2.3 of scan-engine.md).
    pub schedule: Schedule,
    pub large_file_threshold: u64,
    pub batch_interval: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            threads: thread::available_parallelism().map_or(4, |n| n.get()).min(8),
            // v1: std everywhere (as fast as hand-written fstatat). v1.1 on macOS: Backend::Bulk.
            backend: Backend::Std,
            schedule: Schedule::WorkStealing,
            large_file_threshold: DEFAULT_LARGE_FILE_THRESHOLD,
            batch_interval: DEFAULT_BATCH_INTERVAL,
        }
    }
}

/// Sizes of a set of non-directory entries (regular files, symlinks, sockets, ...).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Allocated bytes (`st_blocks * 512`), hard links counted once.
    pub alloc: u64,
    /// Logical bytes (`st_size`), hard links counted once.
    pub apparent: u64,
    /// Number of entries (every name counts, including extra hard links).
    pub files: u64,
}

impl Usage {
    pub fn add(&mut self, o: &Usage) {
        self.alloc += o.alloc;
        self.apparent += o.apparent;
        self.files += o.files;
    }
}

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub name: Box<OsStr>,
    pub alloc: u64,
    pub apparent: u64,
}

/// Why a subdirectory is shown but not scanned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// `st_dev` differs from the root's device (other volume, devfs, procfs, autofs, network ...).
    OtherFilesystem,
    /// Listed in the mount table although on the same device: macOS `/System/Volumes/Data`
    /// (APFS volume group, its content is already reachable via firmlinks) or a Linux bind mount.
    MountPoint,
}

#[derive(Clone, Debug)]
pub struct Subdir {
    pub name: Box<OsStr>,
    pub skip: Option<Skip>,
}

/// Why a directory (or entry) could not be read. Keeps the raw errno because the UI wants to tell
/// EPERM (macOS privacy protection / TCC: "grant Full Disk Access") from EACCES (Unix permissions);
/// `io::ErrorKind` maps both to `PermissionDenied`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanError {
    pub kind: io::ErrorKind,
    pub errno: Option<i32>,
}

impl From<&io::Error> for ScanError {
    fn from(e: &io::Error) -> Self {
        ScanError { kind: e.kind(), errno: e.raw_os_error() }
    }
}

/// The result of listing one directory. Sizes cover only the directory's *direct* content; the
/// UI adds them to all ancestors.
#[derive(Clone, Debug)]
pub struct DirListing {
    pub id: DirId,
    /// Allocated bytes of the directory inode itself (du counts it; 0 on APFS, >= 4 KiB on ext4).
    pub dir_alloc: u64,
    /// All non-directory entries directly in this directory (including the large ones).
    pub files: Usage,
    /// The subset of `files` with `max(alloc, apparent) >= large_file_threshold`.
    pub large_files: Vec<FileEntry>,
    /// Id of `subdirs[0]`; `subdirs[i]` has id `first_child + i`.
    pub first_child: DirId,
    pub subdirs: Vec<Subdir>,
    /// Set if the directory could not be opened or read completely (EACCES/EPERM/EIO/...).
    pub error: Option<ScanError>,
    /// Entries that were listed but could not be stat'ed (vanished, permission, ...).
    pub entry_errors: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Progress {
    pub dirs_listed: u64,
    pub files: u64,
    pub alloc: u64,
    pub errors: u64,
    /// Directories queued or being listed right now (0 at the end).
    pub outstanding: u64,
    pub elapsed: Duration,
}

#[derive(Clone, Debug)]
pub struct Batch {
    pub listings: Vec<DirListing>,
    pub progress: Progress,
}

#[derive(Clone, Debug)]
pub struct Summary {
    pub progress: Progress,
    pub cancelled: bool,
    pub hardlinks_deduped: u64,
    /// First `MAX_ERROR_SAMPLES` paths that could not be read, with the reason.
    pub error_samples: Vec<(PathBuf, ScanError)>,
}

/// What the UI receives. `Finished` is always the last update (also after cancellation).
#[derive(Clone, Debug)]
pub enum Update {
    Batch(Batch),
    Finished(Summary),
}

pub enum Delivery {
    Accepted,
    /// The receiver is still busy with the previous update; ownership is handed back so the
    /// batcher can merge it with the next one.
    Busy(Update),
    /// The receiver is gone; the scan is cancelled.
    Closed,
}

/// Where batches go. The engine does not depend on any UI or async crate. iced 0.14 integration
/// (verified against the iced 0.14.0 sources: `Task::run(stream, f)`, `Task::abortable()`,
/// `Handle::abort_on_drop()`; `iced::futures` is re-exported *without* the executor feature, so
/// `deliver_final` retries instead of using `block_on`):
///
/// ```ignore
/// use iced::futures::channel::mpsc;
/// impl UpdateSink for mpsc::Sender<Update> {
///     fn try_deliver(&mut self, u: Update) -> Delivery {
///         match self.try_send(u) {
///             Ok(()) => Delivery::Accepted,
///             Err(e) if e.is_full() => Delivery::Busy(e.into_inner()),
///             Err(_) => Delivery::Closed,
///         }
///     }
///     fn deliver_final(&mut self, mut u: Update) {
///         loop {
///             match self.try_send(u) {
///                 Err(e) if e.is_full() => { u = e.into_inner(); std::thread::sleep(Duration::from_millis(2)) }
///                 _ => return, // delivered, or the UI is gone
///             }
///         }
///     }
/// }
/// // in update(): Message::Scan(Update) needs `Clone` because iced widgets require it.
/// let (tx, rx) = mpsc::channel(1);
/// self.scan = Some(scanner::start(&root, Options::default(), tx)?); // drop = cancel
/// let (task, handle) = Task::run(rx, Message::Scan).abortable();
/// self.scan_task = Some(handle.abort_on_drop());
/// return task;
/// ```
pub trait UpdateSink: Send + 'static {
    fn try_deliver(&mut self, update: Update) -> Delivery;
    /// Deliver an update that must not be dropped (the final batch and `Finished`); may block.
    fn deliver_final(&mut self, update: Update);
}

/// Bounded std channel as sink (benchmark / tests). Use capacity 1: at most one batch waits.
impl UpdateSink for mpsc::SyncSender<Update> {
    fn try_deliver(&mut self, update: Update) -> Delivery {
        match self.try_send(update) {
            Ok(()) => Delivery::Accepted,
            Err(mpsc::TrySendError::Full(u)) => Delivery::Busy(u),
            Err(mpsc::TrySendError::Disconnected(_)) => Delivery::Closed,
        }
    }
    fn deliver_final(&mut self, update: Update) {
        let _ = self.send(update);
    }
}

/// Handle to a running scan. Dropping it cancels the scan (threads wind down in the background).
pub struct ScanHandle {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
    pub root: PathBuf,
}

impl ScanHandle {
    /// Request cancellation. Returns immediately; workers stop after their current directory
    /// (or within `CANCEL_CHECK_EVERY` entries of a huge one) and `Finished { cancelled: true }`
    /// is delivered. A syscall that hangs (dead network mount) cannot be interrupted, which is
    /// why the UI should treat the scan as cancelled immediately and not wait for `Finished`.
    pub fn cancel(&self) {
        self.shared.cancel();
    }

    /// Wait for all threads (tests / benchmark only; never call this on the UI thread).
    pub fn join(mut self) {
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        if !self.threads.is_empty() {
            self.shared.cancel();
        }
    }
}

/// Start scanning `root` (must be a directory; symlinks in the root path itself are resolved).
pub fn start(root: &Path, opts: Options, sink: impl UpdateSink) -> io::Result<ScanHandle> {
    let root = std::fs::canonicalize(root)?;
    let meta = std::fs::metadata(&root)?;
    if !meta.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "scan root is not a directory"));
    }
    let root_meta = listing::meta_from_std(&meta);
    let shared = Arc::new(Shared {
        root_dev: root_meta.dev,
        mounts: crate::mounts::mount_points_below(&root),
        queue: WorkQueue::new(
            opts.schedule,
            opts.threads.max(1),
            Job { id: ROOT_ID, path: root.clone(), dir_alloc: root_meta.alloc },
        ),
        cancelled: AtomicBool::new(false),
        next_id: AtomicU32::new(ROOT_ID + 1),
        hardlinks: Mutex::new(HashSet::new()),
        dirs_listed: AtomicU64::new(0),
        files: AtomicU64::new(0),
        alloc: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        hardlinks_deduped: AtomicU64::new(0),
        error_samples: Mutex::new(Vec::new()),
        started: Instant::now(),
        opts,
    });

    let (tx, rx) = mpsc::channel::<DirListing>();
    let mut threads = Vec::with_capacity(shared.opts.threads + 1);
    for i in 0..shared.opts.threads.max(1) {
        let (shared, tx) = (Arc::clone(&shared), tx.clone());
        threads.push(
            thread::Builder::new()
                .name(format!("scan-worker-{i}"))
                .stack_size(256 * 1024) // the walker is not recursive; small stacks are fine
                .spawn(move || worker(&shared, i, &tx))?,
        );
    }
    drop(tx); // the batcher sees `Disconnected` once the last worker has exited
    let s = Arc::clone(&shared);
    threads.push(thread::Builder::new().name("scan-batcher".into()).spawn(move || batcher(&s, &rx, sink))?);
    Ok(ScanHandle { shared, threads, root })
}

// ---------------------------------------------------------------------------------------------
// internals
// ---------------------------------------------------------------------------------------------

struct Job {
    id: DirId,
    path: PathBuf,
    /// The directory inode's own allocation, taken from the parent's stat of this entry.
    dir_alloc: u64,
}


struct Shared {
    opts: Options,
    root_dev: u64,
    mounts: HashSet<OsString>,
    queue: WorkQueue<Job>,
    cancelled: AtomicBool,
    next_id: AtomicU32,
    /// (dev, ino) of files with nlink > 1 seen so far. Only touched for hard-linked files, which
    /// are rare, so a plain mutex has no measurable contention.
    hardlinks: Mutex<HashSet<(u64, u64)>>,
    dirs_listed: AtomicU64,
    files: AtomicU64,
    alloc: AtomicU64,
    errors: AtomicU64,
    hardlinks_deduped: AtomicU64,
    error_samples: Mutex<Vec<(PathBuf, ScanError)>>,
    started: Instant,
}

impl Shared {
    fn cancel(&self) {
        self.cancelled.store(true, Relaxed);
        self.queue.wake_all();
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Relaxed)
    }

    /// True the first time a hard-linked inode is seen in this scan.
    fn first_sighting(&self, dev: u64, ino: u64) -> bool {
        let first = self.hardlinks.lock().unwrap().insert((dev, ino));
        if !first {
            self.hardlinks_deduped.fetch_add(1, Relaxed);
        }
        first
    }

    fn record_error(&self, path: PathBuf, err: &io::Error) {
        self.errors.fetch_add(1, Relaxed);
        let mut samples = self.error_samples.lock().unwrap();
        if samples.len() < MAX_ERROR_SAMPLES {
            samples.push((path, ScanError::from(err)));
        }
    }

    fn progress(&self) -> Progress {
        Progress {
            dirs_listed: self.dirs_listed.load(Relaxed),
            files: self.files.load(Relaxed),
            alloc: self.alloc.load(Relaxed),
            errors: self.errors.load(Relaxed),
            outstanding: self.queue.outstanding() as u64,
            elapsed: self.started.elapsed(),
        }
    }
}

fn worker(shared: &Shared, me: usize, tx: &mpsc::Sender<DirListing>) {
    #[cfg(target_os = "macos")]
    if std::env::var_os("SCAN_NO_IOPOL").is_none() {
        // (the env switch exists only for the benchmark that checks the policies cost nothing)
        macos_thread_io_policies();
    }
    let mut children = Vec::new();
    while let Some(job) = shared.queue.pop(me, &shared.cancelled) {
        let listing = list_one(shared, &job, &mut children);
        // 1) Publish the listing first, so it reaches the UI before any child's listing ...
        if tx.send(listing).is_err() {
            shared.cancel(); // batcher gone
        }
        // 2) ... then make the children visible to the other workers.
        shared.queue.complete(me, &mut children);
    }
}

/// Lists one directory: aggregates its files, collects its subdirectories (pushed to `children`).
fn list_one(shared: &Shared, job: &Job, children: &mut Vec<Job>) -> DirListing {
    let threshold = shared.opts.large_file_threshold;
    let mut files = Usage::default();
    let mut large_files = Vec::new();
    // (name, path, dir_alloc, skip)
    let mut subdirs: Vec<(Box<OsStr>, Option<Skip>, u64)> = Vec::new();
    let mut entry_errors = 0u32;
    let mut seen = 0u32;
    let mut unpublished_files = 0u64;

    let result = listing::list_dir(shared.opts.backend, &job.path, &mut |e| {
        seen += 1;
        if seen % CANCEL_CHECK_EVERY == 0 {
            if shared.is_cancelled() {
                return false;
            }
            // keep the progress counter moving inside huge directories
            shared.files.fetch_add(unpublished_files, Relaxed);
            unpublished_files = 0;
        }
        let meta = match e.meta {
            Ok(m) => m,
            Err(err) => {
                entry_errors += 1;
                shared.record_error(job.path.join(e.name), &err);
                return true;
            }
        };
        if e.kind == Kind::Dir {
            let skip = if meta.dev != shared.root_dev {
                Some(Skip::OtherFilesystem)
            } else if meta.mount_point
                || (!shared.mounts.is_empty() && shared.mounts.contains(job.path.join(e.name).as_os_str()))
            {
                Some(Skip::MountPoint)
            } else {
                None
            };
            subdirs.push((e.name.into(), skip, meta.alloc));
        } else {
            files.files += 1;
            unpublished_files += 1;
            // A hard-linked inode is counted where it is seen first (nondeterministic with
            // several threads, but the total is exact).
            if meta.nlink <= 1 || shared.first_sighting(meta.dev, meta.ino) {
                files.alloc += meta.alloc;
                files.apparent += meta.apparent;
                if meta.alloc.max(meta.apparent) >= threshold {
                    large_files.push(FileEntry { name: e.name.into(), alloc: meta.alloc, apparent: meta.apparent });
                }
            }
        }
        true
    });

    let error = match result {
        Ok(()) => None,
        Err(err) => {
            shared.record_error(job.path.clone(), &err);
            Some(ScanError::from(&err))
        }
    };

    // One fetch_add reserves a contiguous id block for all subdirectories.
    let first_child = shared.next_id.fetch_add(subdirs.len() as u32, Relaxed);
    let mut announced = Vec::with_capacity(subdirs.len());
    for (i, (name, skip, dir_alloc)) in subdirs.into_iter().enumerate() {
        if skip.is_none() {
            children.push(Job { id: first_child + i as DirId, path: job.path.join(&*name), dir_alloc });
        }
        announced.push(Subdir { name, skip });
    }

    shared.dirs_listed.fetch_add(1, Relaxed);
    shared.files.fetch_add(unpublished_files, Relaxed);
    shared.alloc.fetch_add(files.alloc + job.dir_alloc, Relaxed);

    DirListing {
        id: job.id,
        dir_alloc: job.dir_alloc,
        files,
        large_files,
        first_child,
        subdirs: announced,
        error,
        entry_errors,
    }
}

/// Coalesces per-directory messages into at most one UI update per `batch_interval`.
fn batcher(shared: &Shared, rx: &mpsc::Receiver<DirListing>, mut sink: impl UpdateSink) {
    let interval = shared.opts.batch_interval;
    let mut pending: Vec<DirListing> = Vec::new();
    let mut next_flush = Instant::now() + interval;
    let mut last_sent = Progress::default();
    loop {
        let now = Instant::now();
        if now >= next_flush {
            let progress = shared.progress();
            // Also flush progress-only batches: inside a directory with 1M files no listing is
            // produced for seconds, but the file counter keeps moving.
            if !pending.is_empty() || progress.files != last_sent.files {
                let batch = Update::Batch(Batch { listings: std::mem::take(&mut pending), progress });
                match sink.try_deliver(batch) {
                    Delivery::Accepted => last_sent = progress,
                    Delivery::Busy(Update::Batch(b)) => pending = b.listings, // merge into the next one
                    Delivery::Busy(Update::Finished(_)) => unreachable!(),
                    Delivery::Closed => {
                        shared.cancel();
                        return;
                    }
                }
            }
            next_flush = now + interval;
        }
        match rx.recv_timeout(next_flush.saturating_duration_since(now)) {
            Ok(listing) => {
                pending.push(listing);
                pending.extend(rx.try_iter()); // drain greedily, cheap
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break, // every worker has exited
        }
    }
    let progress = shared.progress();
    if !pending.is_empty() {
        sink.deliver_final(Update::Batch(Batch { listings: pending, progress }));
    }
    sink.deliver_final(Update::Finished(Summary {
        progress,
        cancelled: shared.is_cancelled(),
        hardlinks_deduped: shared.hardlinks_deduped.load(Relaxed),
        error_samples: std::mem::take(&mut *shared.error_samples.lock().unwrap()),
    }));
}

/// Per-worker-thread I/O policies (thread scope: our workers are dedicated OS threads, so the rest
/// of the app keeps the defaults; no privileges needed; `setiopolicy_np` is not in the libc crate):
/// * never download iCloud Drive / File Provider ("dataless") content just because we looked at
///   it. Apple's du does the same via the `vfs.nspace.prevent_materialization` sysctl, gdu via
///   `setiopolicy_np` (Apple TN3150);
/// * do not update access times of the millions of directories we read (documented in
///   setiopolicy_np(3) as "useful for applications which access a large number of files to reduce
///   the metadata I/O writes").
#[cfg(target_os = "macos")]
fn macos_thread_io_policies() {
    const IOPOL_TYPE_VFS_ATIME_UPDATES: libc::c_int = 2;
    const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: libc::c_int = 3;
    const IOPOL_SCOPE_THREAD: libc::c_int = 1;
    const IOPOL_ATIME_UPDATES_OFF: libc::c_int = 1;
    const IOPOL_MATERIALIZE_DATALESS_FILES_OFF: libc::c_int = 1;
    unsafe extern "C" {
        fn setiopolicy_np(iotype: libc::c_int, scope: libc::c_int, policy: libc::c_int) -> libc::c_int;
    }
    unsafe {
        setiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_THREAD, IOPOL_MATERIALIZE_DATALESS_FILES_OFF);
        setiopolicy_np(IOPOL_TYPE_VFS_ATIME_UPDATES, IOPOL_SCOPE_THREAD, IOPOL_ATIME_UPDATES_OFF);
    }
}
