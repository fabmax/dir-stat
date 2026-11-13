//! Background scanning plumbing: std::thread worker -> futures mpsc -> iced Task / Subscription.
//!
//! Three variants are probed:
//!   A) `start()`           Task::stream(receiver).abortable()   <- RECOMMENDED
//!   B) `subscription()`    Subscription::run_with(request, fn)  (declarative alternative)
//!   C) `channel_stream()`  iced::stream::channel(size, async closure) forwarding a std thread
use iced::futures::channel::mpsc;
use iced::futures::{SinkExt, Stream, StreamExt};
use iced::{Subscription, Task};

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Messages produced by the scanner. Must be `Clone` (iced widgets require `Message: Clone`)
/// and `Send + 'static` (iced requires `Message: Send`). Batches are wrapped in `Arc` so that
/// cloning a message is O(1).
#[derive(Debug, Clone)]
pub enum Event {
    /// A batch of directories whose *own* (non-recursive) file bytes are known.
    Batch(Arc<Vec<DirRecord>>),
    /// Periodic counters (sent together with batches, ~10x per second).
    Progress(Progress),
    /// The walk finished (or was cancelled).
    Done { progress: Progress, cancelled: bool },
}

#[derive(Debug, Clone)]
pub struct DirRecord {
    pub path: PathBuf,
    pub parent: Option<PathBuf>,
    /// Bytes of regular files directly inside this directory (not recursive).
    pub own_bytes: u64,
    pub own_files: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub errors: u64,
    pub current: Option<PathBuf>,
    pub elapsed: Duration,
}

/// Owned by the application state while a scan runs. Dropping it cancels the scan:
/// the AtomicBool stops the thread promptly and the abort-on-drop handle kills the stream.
pub struct Scan {
    cancel: Arc<AtomicBool>,
    _task: iced::task::Handle,
}

impl Scan {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl std::fmt::Debug for Scan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scan").finish_non_exhaustive()
    }
}

/// Variant A (recommended): spawn a std thread, stream its output into the runtime.
pub fn start(root: PathBuf) -> (Scan, Task<Event>) {
    let cancel = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::unbounded::<Event>();

    let flag = Arc::clone(&cancel);
    std::thread::Builder::new()
        .name("dir-stat-scan".into())
        .spawn(move || worker(&root, &flag, &sender))
        .expect("spawn scanner thread");

    // Task::stream(receiver) == Task::run(receiver, |event| event)
    let (task, handle) = Task::stream(receiver).abortable();

    (
        Scan {
            cancel,
            _task: handle.abort_on_drop(),
        },
        task,
    )
}

/// Variant B: the scan lives as long as `subscription()` keeps returning it.
/// `run_with` takes a *function pointer* `fn(&D) -> S`; D must be `Hash + 'static`
/// and is part of the subscription identity (change `id` to restart).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScanRequest {
    pub id: u64,
    pub root: PathBuf,
}

pub fn subscription(request: &ScanRequest) -> Subscription<Event> {
    Subscription::run_with(request.clone(), |request: &ScanRequest| {
        let (sender, receiver) = mpsc::unbounded::<Event>();
        let root = request.root.clone();

        std::thread::spawn(move || {
            // No explicit flag: the worker stops when the receiver is dropped
            // (unbounded_send fails) i.e. when the subscription is no longer returned.
            worker(&root, &AtomicBool::new(false), &sender);
        });

        receiver
    })
}

/// Variant C: iced::stream::channel with an async closure (AsyncFnOnce in 0.14).
#[allow(dead_code)]
pub fn channel_stream(root: PathBuf) -> impl Stream<Item = Event> {
    iced::stream::channel(64, async move |mut output: mpsc::Sender<Event>| {
        let (sender, mut receiver) = mpsc::unbounded::<Event>();

        std::thread::spawn(move || worker(&root, &AtomicBool::new(false), &sender));

        while let Some(event) = receiver.next().await {
            // Sender::send comes from SinkExt; it awaits when the buffer (64) is full.
            if output.send(event).await.is_err() {
                break;
            }
        }
    })
}

/// The blocking walk. Sends a batch at most every FLUSH_EVERY so the UI receives ~10 msgs/s,
/// no matter how fast the disk is (avoids update/view storms).
fn worker(root: &Path, cancel: &AtomicBool, sender: &mpsc::UnboundedSender<Event>) {
    const FLUSH_EVERY: Duration = Duration::from_millis(100);

    let started = Instant::now();
    let mut last_flush = Instant::now();
    let mut progress = Progress::default();
    let mut batch: Vec<DirRecord> = Vec::new();

    // (dev, inode) of multiply-linked files already counted (unix only; unused elsewhere).
    let mut seen_inodes: HashSet<(u64, u64)> = HashSet::new();

    #[cfg(unix)]
    let root_dev = {
        use std::os::unix::fs::MetadataExt;
        std::fs::symlink_metadata(root).map(|m| m.dev()).ok()
    };

    let mut stack: Vec<(PathBuf, Option<PathBuf>)> = vec![(root.to_path_buf(), None)];

    while let Some((dir, parent)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        progress.dirs += 1;
        let mut own_bytes = 0u64;
        let mut own_files = 0u64;

        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    // DirEntry::file_type does not follow symlinks.
                    let Ok(file_type) = entry.file_type() else {
                        progress.errors += 1;
                        continue;
                    };

                    if file_type.is_dir() {
                        let path = entry.path();

                        // Do not cross into other mounted filesystems (unix only).
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::MetadataExt;
                            if let (Some(root_dev), Ok(meta)) = (root_dev, entry.metadata())
                                && meta.dev() != root_dev
                            {
                                continue;
                            }
                        }

                        stack.push((path, Some(dir.clone())));
                    } else if file_type.is_file() {
                        let Ok(meta) = entry.metadata() else {
                            progress.errors += 1;
                            continue;
                        };

                        own_files += 1;
                        own_bytes += allocated_size(&meta, &mut seen_inodes);
                    }
                    // Symlinks and special files are ignored (not followed).
                }
            }
            Err(_) => progress.errors += 1,
        }

        progress.files += own_files;
        progress.bytes += own_bytes;

        batch.push(DirRecord {
            path: dir.clone(),
            parent,
            own_bytes,
            own_files,
        });

        if last_flush.elapsed() >= FLUSH_EVERY {
            last_flush = Instant::now();
            progress.current = Some(dir);
            progress.elapsed = started.elapsed();

            let records = Arc::new(std::mem::take(&mut batch));
            // Receiver dropped (task aborted / subscription gone) => stop working.
            if sender.unbounded_send(Event::Batch(records)).is_err()
                || sender.unbounded_send(Event::Progress(progress.clone())).is_err()
            {
                eprintln!(
                    "[scan worker] receiver dropped after {} dirs -> stopping",
                    progress.dirs
                );
                return;
            }
        }
    }

    let cancelled = cancel.load(Ordering::Relaxed);
    eprintln!(
        "[scan worker] loop ended after {} dirs in {:?} (cancel flag = {cancelled})",
        progress.dirs,
        started.elapsed()
    );
    progress.current = None;
    progress.elapsed = started.elapsed();

    if !batch.is_empty() {
        let _ = sender.unbounded_send(Event::Batch(Arc::new(batch)));
    }
    let _ = sender.unbounded_send(Event::Done {
        progress,
        cancelled,
    });
}

/// Bytes actually allocated on disk (what `du` reports), with hard-link de-duplication.
#[cfg(unix)]
fn allocated_size(meta: &std::fs::Metadata, seen: &mut HashSet<(u64, u64)>) -> u64 {
    use std::os::unix::fs::MetadataExt;

    if meta.nlink() > 1 && !seen.insert((meta.dev(), meta.ino())) {
        return 0;
    }
    // st_blocks is always in 512-byte units, independent of the fs block size.
    meta.blocks() * 512
}

#[cfg(not(unix))]
fn allocated_size(meta: &std::fs::Metadata, _seen: &mut HashSet<(u64, u64)>) -> u64 {
    meta.len()
}
