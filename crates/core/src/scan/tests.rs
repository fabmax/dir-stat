//! Scanner unit tests that need crate internals. The fixture-based tests are in
//! `crates/core/tests/scan.rs`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use super::batcher::boundary_set;
use super::queue::StealingQueue;
use super::worker::{BoundarySet, Counters, Job, Shared};
use super::*;
use crate::catalog::{Engine, Resolver};
use crate::platform::Mount;
use crate::tree::{NodeState, Tree};

/// A directory with this name makes its worker panic (`worker::list_one`).
pub(super) const PANIC_TRIGGER: &str = "__dirstat_test_panic__";

fn mount(path: &str, fs_type: &str) -> Mount {
    Mount {
        path: path.into(),
        fs_type: fs_type.into(),
        local: true,
        read_only: false,
        browsable: true,
        automounted: false,
        dev: None,
        space: None,
    }
}

fn paths(set: &HashSet<PathBuf>) -> Vec<&str> {
    let mut v: Vec<&str> = set.iter().map(|p| p.to_str().unwrap()).collect();
    v.sort_unstable();
    v
}

#[test]
fn system_boundaries_are_the_mounts_strictly_below_the_root() {
    let mounts = [
        mount("/", "apfs"),
        mount("/System/Volumes/Data", "apfs"),
        mount("/Volumes/Backup", "hfs"),
        mount("/Users/u/pool", "zfs"),
        mount("/Users/u/net", "smbfs"),
    ];
    let set = boundary_set(Path::new("/Users/u"), &Boundaries::System, &mounts);
    assert_eq!(paths(&set.mounts), ["/Users/u/net", "/Users/u/pool"]);
    assert_eq!(paths(&set.excluded), ["/Users/u/pool/.zfs"]);
    assert_eq!(paths(&set.parents), ["/Users/u", "/Users/u/pool"]);

    // A chosen root is scanned even if it is a mount itself; its Time Machine folder is not.
    let set = boundary_set(Path::new("/Volumes/Backup"), &Boundaries::System, &mounts);
    assert!(set.mounts.is_empty());
    assert_eq!(paths(&set.excluded), ["/Volumes/Backup/Backups.backupdb"]);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_root_and_data_volume_rules() {
    let mounts = [
        mount("/", "apfs"),
        mount("/System/Volumes/Data", "apfs"),
        mount("/Volumes/Stick", "msdos"),
    ];
    let set = boundary_set(Path::new("/"), &Boundaries::System, &mounts);
    assert_eq!(
        paths(&set.mounts),
        ["/System/Volumes/Data", "/Volumes/Stick"]
    );
    assert_eq!(paths(&set.excluded), ["/.nofollow", "/.resolve", "/.vol"]);

    // Below the Data volume, firmlinked mounts also appear under their Data path (D2).
    let set = boundary_set(
        Path::new("/System/Volumes/Data"),
        &Boundaries::System,
        &mounts,
    );
    assert_eq!(paths(&set.mounts), ["/System/Volumes/Data/Volumes/Stick"]);
    assert!(set.excluded.is_empty());
}

fn shared_for_progress(workers: usize) -> Shared {
    Shared {
        threshold: LARGE_FILE,
        root_dev: 0,
        hfs: false,
        bounds: BoundarySet::default(),
        counted_elsewhere: None,
        queue: StealingQueue::new(
            workers,
            Job {
                id: NodeId::ROOT,
                path: "/".into(),
                dir_alloc: 0,
            },
        ),
        cancelled: Arc::new(AtomicBool::new(false)),
        next_id: AtomicU32::new(1),
        links: Mutex::default(),
        dir_links: Mutex::default(),
        in_flight: (0..workers).map(|_| AtomicU64::new(0)).collect(),
        counters: Counters::default(),
        started: Instant::now() - Duration::from_secs(10),
    }
}

#[test]
fn current_job_prefers_a_stuck_one() {
    let shared = shared_for_progress(3);
    assert_eq!(shared.progress().current, None);
    let slot = |id: u32, start_ms: u64| (u64::from(id) + 1) << 32 | start_ms;
    let now = shared.started.elapsed().as_millis() as u64;
    // Two fresh jobs: the most recently started one is shown.
    shared.in_flight[0].store(slot(5, now - 300), Relaxed);
    shared.in_flight[1].store(slot(7, now - 100), Relaxed);
    assert_eq!(shared.progress().current.map(|c| c.0), Some(NodeId(7)));
    // A job running for over a second is what the scan waits on.
    shared.in_flight[2].store(slot(9, now - 4000), Relaxed);
    let (id, running) = shared.progress().current.unwrap();
    assert_eq!(id, NodeId(9));
    assert!(running >= Duration::from_secs(4));
}

#[test]
fn a_panicking_folder_is_reported_and_the_scan_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::create_dir_all(root.join(PANIC_TRIGGER).join("never_seen")).unwrap();
    std::fs::create_dir_all(root.join("ok/deeper")).unwrap();
    std::fs::write(root.join("ok/file"), b"data").unwrap();

    let engine = Arc::new(Engine::new(&Resolver::from_env()));
    let opts = Options {
        threads: 2,
        batch_interval: Duration::from_millis(5),
        ..Options::new(engine)
    };
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = start(ScanRequest::new(&root), opts, tx);
    let mut tree = Tree::opening(root.clone());
    let summary = loop {
        match rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the scan must not hang")
        {
            Update::Started(info) => tree.start(info),
            Update::Batch(batch) => batch.listings.into_iter().for_each(|l| {
                tree.apply(l);
            }),
            Update::Finished(summary) => break summary,
            Update::Failed(e) => panic!("{}", e.message),
        }
    };
    handle.join();

    assert_eq!(summary.internal_errors, 1);
    assert!(!summary.cancelled);
    let bad = tree.find_path(&root.join(PANIC_TRIGGER)).unwrap();
    assert_eq!(
        tree.node(bad).error().map(|e| e.class),
        Some(ErrorClass::Internal)
    );
    assert_eq!(tree.node(bad).child_count(), 0);
    assert!(tree.node(NodeId::ROOT).is_done());
    let ok = tree.find_path(&root.join("ok/deeper")).unwrap();
    assert_eq!(tree.node(ok).state(), NodeState::Done);
    assert_eq!(tree.errors().len(), 1);
}

#[test]
fn an_inert_handle_can_be_cancelled_and_dropped() {
    let handle = ScanHandle::inert();
    handle.cancel();
    drop(handle);
}
