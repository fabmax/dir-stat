//! The `scan-main` thread: opens the root, starts the workers, then coalesces their
//! listings into at most one UI update per batch interval (design §4.1, §4.4).

use std::collections::HashSet;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use super::queue::StealingQueue;
use super::worker::{self, BoundarySet, Counters, Job, Shared, lock};
use super::{
    Batch, Boundaries, DirListing, NodeId, Options, RootInfo, ScanRequest, StartError, Summary,
    Update,
};
use crate::platform::{self, Mount};

pub enum Delivery {
    Accepted,
    /// The receiver is still busy with the previous update. Ownership comes back, so the
    /// batcher can merge it into the next one.
    Busy(Update),
    /// The receiver is gone; the scan cancels itself.
    Closed,
}

/// Where updates go. The engine depends on no UI or async crate; the app wraps its channel.
pub trait UpdateSink: Send + 'static {
    fn try_deliver(&mut self, update: Update) -> Delivery;
    /// `Started`/`Failed`, the final batch and `Finished`: must not be dropped; may wait.
    fn deliver_final(&mut self, update: Update);
}

/// A bounded std channel as sink (tests, the example). Capacity 1: one batch waits at most.
impl UpdateSink for mpsc::SyncSender<Update> {
    fn try_deliver(&mut self, update: Update) -> Delivery {
        match self.try_send(update) {
            Ok(()) => Delivery::Accepted,
            Err(mpsc::TrySendError::Full(update)) => Delivery::Busy(update),
            Err(mpsc::TrySendError::Disconnected(_)) => Delivery::Closed,
        }
    }

    fn deliver_final(&mut self, update: Update) {
        let _ = self.send(update);
    }
}

#[derive(Default)]
struct Control {
    cancelled: Arc<AtomicBool>,
    /// Set once the workers exist, so `cancel` can wake them.
    shared: OnceLock<Arc<Shared>>,
}

/// A running scan. Dropping it cancels the scan; threads are never joined on the UI thread.
pub struct ScanHandle {
    control: Arc<Control>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ScanHandle {
    /// Returns at once. Workers stop after their current directory (or within 256 entries of
    /// a huge one), then `Finished { cancelled: true }` follows. A syscall that hangs on a dead
    /// network volume can't be interrupted, so the UI treats Stop as final immediately.
    pub fn cancel(&self) {
        self.control.cancelled.store(true, Relaxed);
        if let Some(shared) = self.control.shared.get() {
            shared.queue.wake_all();
        }
    }

    /// Waits until every scan thread has exited. For tests and the example only.
    pub fn join(mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    /// A handle with no scan behind it, for app tests that feed updates by hand.
    #[cfg(any(test, feature = "test-util"))]
    pub fn inert() -> ScanHandle {
        ScanHandle {
            control: Arc::default(),
            thread: None,
        }
    }
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Starts a scan on its own `scan-main` thread and returns at once: no I/O happens here, so
/// a privacy prompt or a dead network root can only delay `Started`.
pub fn start(req: ScanRequest, opts: Options, sink: impl UpdateSink) -> ScanHandle {
    let control = Arc::new(Control::default());
    let thread = {
        let control = Arc::clone(&control);
        // If even this spawn fails, nothing ever arrives; the UI stays in "Opening…" and
        // Stop still works.
        thread::Builder::new()
            .name("scan-main".into())
            .spawn(move || scan_main(req, opts, sink, &control))
            .ok()
    };
    ScanHandle { control, thread }
}

fn scan_main(req: ScanRequest, opts: Options, mut sink: impl UpdateSink, control: &Control) {
    platform::init_io_thread();
    let (root, meta) = match open_root(&req.root) {
        Ok(opened) => opened,
        Err(error) => return sink.deliver_final(Update::Failed(error)),
    };
    let mounts = platform::mounts();
    let fs_type = platform::fs_type_of(&root, &mounts);
    let bounds = boundary_set(&root, &req.boundaries, &mounts);
    let hfs = fs_type == "hfs";
    sink.deliver_final(Update::Started(RootInfo {
        root: root.clone(),
        dev: meta.dev,
        ino: meta.ino,
        fs_type,
        context: req.context.unwrap_or_default(),
    }));

    let threads = opts.threads.max(1);
    let shared = Arc::new(Shared {
        threshold: opts.large_file_threshold,
        root_dev: meta.dev,
        hfs,
        bounds,
        counted_elsewhere: req.counted_elsewhere,
        queue: StealingQueue::new(
            threads,
            Job {
                id: NodeId::ROOT,
                path: root,
                dir_alloc: meta.alloc,
            },
        ),
        cancelled: Arc::clone(&control.cancelled),
        next_id: AtomicU32::new(1),
        links: Mutex::default(),
        dir_links: Mutex::new(if hfs {
            HashSet::from([(meta.dev, meta.ino)])
        } else {
            HashSet::new()
        }),
        in_flight: (0..threads).map(|_| AtomicU64::new(0)).collect(),
        counters: Counters::default(),
        started: Instant::now(),
    });
    let _ = control.shared.set(Arc::clone(&shared));

    let (tx, rx) = mpsc::channel::<DirListing>();
    let mut spawned = 0;
    for me in 0..threads {
        let (shared, tx) = (Arc::clone(&shared), tx.clone());
        let worker = thread::Builder::new()
            .name(format!("scan-worker-{me}"))
            .stack_size(256 * 1024) // the walker isn't recursive
            .spawn(move || worker::run(&shared, me, &tx));
        spawned += usize::from(worker.is_ok());
    }
    drop(tx); // the batcher sees `Disconnected` once the last worker has exited
    if spawned == 0 {
        shared.cancel();
    }

    let batched = catch_unwind(AssertUnwindSafe(|| {
        batch_loop(&shared, &rx, opts.batch_interval, &mut sink)
    }));
    if batched.is_err() {
        shared.cancel();
        sink.deliver_final(Update::Finished(Summary {
            progress: shared.progress(),
            cancelled: true,
            ..Summary::default()
        }));
    }
}

/// Canonical path and metadata of a root that is a directory.
fn open_root(requested: &Path) -> Result<(PathBuf, platform::EntryMeta), StartError> {
    let start_error = |e: io::Error| StartError {
        kind: e.kind(),
        message: e.to_string(),
    };
    let root = std::fs::canonicalize(requested).map_err(start_error)?;
    let meta = std::fs::metadata(&root).map_err(start_error)?;
    if !meta.is_dir() {
        return Err(StartError {
            kind: io::ErrorKind::NotADirectory,
            message: "Not a folder".into(),
        });
    }
    Ok((root, platform::entry_meta(&meta)))
}

/// Mount points and excluded folders strictly below the root (design §4.7).
pub(super) fn boundary_set(root: &Path, boundaries: &Boundaries, mounts: &[Mount]) -> BoundarySet {
    let below = |p: &PathBuf| p != root && p.starts_with(root);
    let (mut points, mut excluded) = (HashSet::new(), HashSet::new());
    match boundaries {
        Boundaries::Fixed {
            mounts,
            excluded: fixed,
        } => {
            let canonical = |p: &PathBuf| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            points.extend(mounts.iter().map(canonical));
            excluded.extend(fixed.iter().map(canonical));
        }
        Boundaries::System => {
            // Below /System/Volumes/Data, firmlinked mounts also appear under their Data path.
            let data = Path::new("/System/Volumes/Data");
            let aliases = cfg!(target_os = "macos") && root.starts_with(data);
            for mount in mounts {
                points.insert(mount.path.clone());
                if aliases
                    && !mount.path.starts_with(data)
                    && let Ok(rel) = mount.path.strip_prefix("/")
                {
                    points.insert(data.join(rel));
                }
                match mount.fs_type.as_str() {
                    "hfs" => excluded.insert(mount.path.join("Backups.backupdb")),
                    "zfs" => excluded.insert(mount.path.join(".zfs")),
                    _ => false,
                };
            }
            if cfg!(target_os = "macos") && root == Path::new("/") {
                excluded.extend(["/.vol", "/.nofollow", "/.resolve"].map(PathBuf::from));
            }
        }
    }
    points.retain(below);
    excluded.retain(below);
    BoundarySet::new(points, excluded)
}

fn batch_loop(
    shared: &Shared,
    rx: &mpsc::Receiver<DirListing>,
    interval: Duration,
    sink: &mut impl UpdateSink,
) {
    let mut pending: Vec<DirListing> = Vec::new();
    let mut next_flush = Instant::now() + interval;
    let mut last_files = 0;
    loop {
        let now = Instant::now();
        if now >= next_flush {
            let progress = shared.progress();
            // Progress-only batches too: inside a huge directory no listing arrives for
            // seconds, but the file counter moves.
            if !pending.is_empty() || progress.files != last_files {
                match sink.try_deliver(Update::Batch(Batch {
                    listings: std::mem::take(&mut pending),
                    progress,
                })) {
                    Delivery::Accepted => last_files = progress.files,
                    Delivery::Busy(Update::Batch(batch)) => pending = batch.listings,
                    Delivery::Busy(_) => {}
                    Delivery::Closed => return shared.cancel(),
                }
            }
            next_flush = now + interval;
        }
        match rx.recv_timeout(next_flush.saturating_duration_since(now)) {
            Ok(listing) => {
                pending.push(listing);
                pending.extend(rx.try_iter());
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break, // every worker has exited
        }
    }
    let progress = shared.progress();
    if !pending.is_empty() {
        sink.deliver_final(Update::Batch(Batch {
            listings: pending,
            progress,
        }));
    }
    sink.deliver_final(Update::Finished(Summary {
        progress,
        cancelled: shared.is_cancelled(),
        hardlinks_deduped: shared.counters.hardlinks_deduped.load(Relaxed),
        internal_errors: shared.counters.internal_errors.load(Relaxed),
        links: Arc::new(std::mem::take(&mut *lock(&shared.links))),
    }));
}
