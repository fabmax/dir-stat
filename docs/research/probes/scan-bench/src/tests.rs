//! Invariant tests for the reference scanner (run: `cargo test --release`).
//! Fixtures are created under /tmp/dirstat-work/probes/fixtures (never elsewhere).

use crate::listing::Backend;
use crate::model::{NodeState, TreeModel};
use crate::scanner::{self, DirId, Options, Schedule, Update, ROOT_ID};
use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(format!("/tmp/dirstat-work/probes/fixtures/{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn write(p: &Path, bytes: usize) {
    fs::write(p, vec![b'x'; bytes]).unwrap();
}

/// root/
///   a/f1 (10 KB), a/f2 = hard link to a/f1, a/sub/f3 = hard link to a/f1
///   b/c/d/big (2 MiB), b/to_a -> ../a (dir symlink, must not be followed), b/dangling -> nowhere
///   empty/
///   locked/ (mode 000, contains a file)
///   wide/ 3000 files of 1 KB
fn build_tree(root: &Path) {
    fs::create_dir_all(root.join("a/sub")).unwrap();
    write(&root.join("a/f1"), 10_000);
    fs::hard_link(root.join("a/f1"), root.join("a/f2")).unwrap();
    fs::hard_link(root.join("a/f1"), root.join("a/sub/f3")).unwrap();
    fs::create_dir_all(root.join("b/c/d")).unwrap();
    write(&root.join("b/c/d/big"), 2 << 20);
    symlink("../a", root.join("b/to_a")).unwrap();
    symlink("/nonexistent/target", root.join("b/dangling")).unwrap();
    fs::create_dir(root.join("empty")).unwrap();
    fs::create_dir(root.join("locked")).unwrap();
    write(&root.join("locked/secret"), 5000);
    fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
    fs::create_dir(root.join("wide")).unwrap();
    for i in 0..3000 {
        write(&root.join(format!("wide/f{i}")), 1000);
    }
}

fn unlock(root: &Path) {
    let _ = fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o755));
}

/// Expected allocation computed independently (single-threaded, lstat, unique inodes, du rules).
fn expected_alloc(root: &Path) -> u64 {
    fn walk(p: &Path, seen: &mut HashSet<(u64, u64)>, total: &mut u64) {
        let m = fs::symlink_metadata(p).unwrap();
        if m.is_dir() {
            *total += m.blocks() * 512;
            if let Ok(rd) = fs::read_dir(p) {
                for e in rd.flatten() {
                    walk(&e.path(), seen, total);
                }
            }
        } else if m.nlink() <= 1 || seen.insert((m.dev(), m.ino())) {
            *total += m.blocks() * 512;
        }
    }
    let (mut seen, mut total) = (HashSet::new(), 0);
    walk(root, &mut seen, &mut total);
    total
}

struct Run {
    model: TreeModel,
    summary: scanner::Summary,
    listed: Vec<DirId>,
}

/// Runs a scan and checks the streaming invariants while applying the batches.
fn run(root: &Path, backend: Backend, schedule: Schedule, threads: usize) -> Run {
    let opts = Options { threads, backend, schedule, batch_interval: Duration::from_millis(5), ..Options::default() };
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = scanner::start(root, opts, tx).unwrap();
    let mut model = TreeModel::new(handle.root.clone());
    let mut announced: HashSet<DirId> = HashSet::from([ROOT_ID]);
    let mut listed = Vec::new();
    let summary = loop {
        match rx.recv().unwrap() {
            Update::Batch(b) => {
                for l in b.listings {
                    // parent before child: the id must have been announced by an earlier listing
                    assert!(announced.contains(&l.id), "listing {} arrived before its parent's", l.id);
                    assert!(!listed.contains(&l.id), "listing {} delivered twice", l.id);
                    for (i, sd) in l.subdirs.iter().enumerate() {
                        if sd.skip.is_none() {
                            announced.insert(l.first_child + i as DirId);
                        }
                    }
                    listed.push(l.id);
                    model.apply(l);
                }
            }
            Update::Finished(s) => break s,
        }
    };
    // nothing after Finished
    assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
    handle.join();
    // exactly once: every announced directory was listed
    assert_eq!(announced.len(), listed.len(), "announced vs listed");
    Run { model, summary, listed }
}

fn backends() -> Vec<Backend> {
    ["std", "at", "bulk"].iter().filter_map(|b| Backend::parse(b)).collect()
}

#[test]
fn totals_and_invariants_all_backends_and_schedules() {
    let root = fixture("tree");
    build_tree(&root);
    let expected = expected_alloc(&root); // `locked` is unreadable here too
    for backend in backends() {
        for schedule in [Schedule::WorkStealing, Schedule::SharedLifo, Schedule::SharedFifo] {
            for threads in [1, 3, 8] {
                let r = run(&root, backend, schedule, threads);
                let ctx = format!("{backend:?} {schedule:?} t={threads}");
                r.model.check_complete().unwrap();
                assert_eq!(r.model.root().state, NodeState::Done, "{ctx}");
                assert_eq!(r.model.root().total.alloc, expected, "{ctx}");
                assert_eq!(r.summary.progress.alloc, expected, "{ctx}");
                assert_eq!(r.summary.hardlinks_deduped, 2, "{ctx}"); // f2 and f3
                assert!(!r.summary.cancelled, "{ctx}");
                // the locked directory is reported with EACCES and counted as an error
                assert_eq!(r.summary.progress.errors, 1, "{ctx}");
                let locked = (0..r.model.nodes.len() as DirId).find(|&i| r.model.nodes[i as usize].name.to_str() == Some("locked")).unwrap();
                assert_eq!(r.model.nodes[locked as usize].error.and_then(|e| e.errno), Some(libc::EACCES), "{ctx}");
                // symlinks are files, never followed: b has 2 entries (to_a, dangling) and 1 subdir
                let b = (0..r.model.nodes.len() as DirId).find(|&i| r.model.nodes[i as usize].name.to_str() == Some("b")).unwrap();
                assert_eq!(r.model.nodes[b as usize].own.files, 2, "{ctx}");
                assert_eq!(r.model.nodes[b as usize].child_count, 1, "{ctx}");
                // the 2 MiB file is kept individually, the 1 KB files only as aggregate
                let big = r.model.nodes.iter().flat_map(|n| &n.large_files).map(|f| f.name.to_str().unwrap()).collect::<Vec<_>>();
                assert_eq!(big, vec!["big"], "{ctx}");
                assert_eq!(r.listed.len(), 9, "{ctx}"); // root a a/sub b b/c b/c/d empty locked wide
            }
        }
    }
    unlock(&root);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn cancellation_delivers_finished_quickly() {
    let root = fixture("cancel");
    for d in 0..200 {
        let dir = root.join(format!("d{d}"));
        fs::create_dir(&dir).unwrap();
        for f in 0..50 {
            write(&dir.join(format!("f{f}")), 100);
        }
    }
    for schedule in [Schedule::WorkStealing, Schedule::SharedLifo] {
        let opts = Options { threads: 4, schedule, ..Options::default() };
        let (tx, rx) = mpsc::sync_channel(1);
        let handle = scanner::start(&root, opts, tx).unwrap();
        handle.cancel();
        let t = std::time::Instant::now();
        let finished = loop {
            if let Update::Finished(s) = rx.recv().unwrap() {
                break s;
            }
        };
        assert!(finished.cancelled);
        assert!(t.elapsed() < Duration::from_secs(1));
        handle.join();
    }
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn dropping_the_receiver_cancels_the_scan() {
    let root = fixture("drop");
    for d in 0..100 {
        fs::create_dir_all(root.join(format!("d{d}/x/y"))).unwrap();
    }
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = scanner::start(&root, Options { threads: 4, ..Options::default() }, tx).unwrap();
    drop(rx);
    handle.join(); // must terminate
    fs::remove_dir_all(&root).unwrap();
}
