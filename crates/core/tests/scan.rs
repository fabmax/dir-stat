//! Scanner tests on fixture trees in a temp dir (design §10.2): streaming invariants,
//! accounting, du parity, boundaries, vanished folders, start failures, cancellation and
//! rescans with hard links.

#![cfg(unix)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

use dirstat_core::catalog::{Engine, Resolver};
use dirstat_core::scan::{
    self, Boundaries, Delivery, ErrorClass, NodeId, Options, ScanRequest, Skip, Summary, Update,
    UpdateSink,
};
use dirstat_core::tree::{NodeState, Totals, Tree};

fn engine() -> Arc<Engine> {
    static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();
    ENGINE
        .get_or_init(|| Arc::new(Engine::new(&Resolver::from_env())))
        .clone()
}

fn options(threads: usize) -> Options {
    Options {
        threads,
        batch_interval: Duration::from_millis(5),
        ..Options::new(engine())
    }
}

fn is_root_user() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// A fresh fixture directory, canonical (on macOS $TMPDIR is behind the /var symlink).
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    (dir, root)
}

fn write(path: &Path, bytes: usize) {
    fs::write(path, vec![b'x'; bytes]).unwrap();
}

fn lock_dir(path: &Path) {
    fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
}

/// Restores permissions so the temp dir can be removed.
struct Unlock(Vec<PathBuf>);

impl Drop for Unlock {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
        }
    }
}

/// root/
///   a/f1 (10 kB), a/f2 = hard link to a/f1, a/sub/f3 = hard link to a/f1
///   b/c/d/big (2 MiB), b/to_a -> ../a (never followed), b/dangling -> nowhere
///   empty/
///   locked/ (mode 000, holds a file)
///   wide/ 3,000 files of 1 kB
///   mnt/ (declared a mount point) holds locked2/ (mode 000: a stat would be an error)
///   skipme/ (declared excluded)
fn build_tree(root: &Path) -> Unlock {
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
    fs::create_dir(root.join("wide")).unwrap();
    for i in 0..3000 {
        write(&root.join(format!("wide/f{i}")), 1000);
    }
    fs::create_dir_all(root.join("mnt/locked2")).unwrap();
    fs::create_dir_all(root.join("skipme/inner")).unwrap();
    write(&root.join("skipme/inner/data"), 50_000);
    lock_dir(&root.join("locked"));
    lock_dir(&root.join("mnt/locked2"));
    Unlock(vec![root.join("locked"), root.join("mnt/locked2")])
}

fn fixture_boundaries(root: &Path) -> Boundaries {
    Boundaries::Fixed {
        mounts: vec![root.join("mnt")],
        excluded: vec![root.join("skipme")],
    }
}

/// du rules, single-threaded with lstat, independent of the scanner.
fn expected_alloc(root: &Path, skip: &[PathBuf]) -> u64 {
    fn walk(p: &Path, skip: &[PathBuf], seen: &mut HashSet<(u64, u64)>, total: &mut u64) {
        if skip.iter().any(|s| s == p) {
            return;
        }
        let m = fs::symlink_metadata(p).unwrap();
        if m.is_dir() {
            *total += m.blocks() * 512;
            if let Ok(entries) = fs::read_dir(p) {
                for e in entries.flatten() {
                    walk(&e.path(), skip, seen, total);
                }
            }
        } else if m.nlink() <= 1 || seen.insert((m.dev(), m.ino())) {
            *total += m.blocks() * 512;
        }
    }
    let (mut seen, mut total) = (HashSet::new(), 0);
    walk(root, skip, &mut seen, &mut total);
    total
}

struct Run {
    tree: Tree,
    summary: Summary,
    listed: usize,
}

/// Runs a scan to the end, checking the streaming invariants while applying the batches.
fn run(req: ScanRequest, threads: usize) -> Run {
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = scan::start(req.clone(), options(threads), tx);
    let mut tree = Tree::opening(req.root);
    let mut announced = HashSet::from([NodeId::ROOT]);
    let mut listed = HashSet::new();
    let mut first = true;
    let summary = loop {
        let update = rx.recv().expect("the scan ended without Finished");
        match update {
            Update::Started(info) => {
                assert!(first, "Started must come first");
                tree.start(info);
            }
            Update::Failed(e) => panic!("scan failed: {}", e.message),
            Update::Batch(batch) => {
                assert!(!first, "a batch before Started");
                for l in batch.listings {
                    assert!(
                        announced.contains(&l.id),
                        "{:?} arrived before its parent's listing",
                        l.id
                    );
                    assert!(listed.insert(l.id), "{:?} delivered twice", l.id);
                    for (i, sub) in l.subdirs.iter().enumerate() {
                        if sub.skip.is_none() {
                            announced.insert(NodeId(l.first_child.0 + i as u32));
                        }
                    }
                    tree.apply(l);
                }
            }
            Update::Finished(summary) => break summary,
        }
        first = false;
    };
    handle.join();
    assert!(rx.try_recv().is_err(), "nothing may follow Finished");
    if !summary.cancelled {
        assert_eq!(
            announced, listed,
            "every announced folder is listed exactly once"
        );
    }
    tree.finish(&summary);
    Run {
        tree,
        summary,
        listed: listed.len(),
    }
}

fn find(tree: &Tree, root: &Path, rel: &str) -> NodeId {
    tree.find_path(&root.join(rel))
        .unwrap_or_else(|| panic!("{rel} not in the tree"))
}

/// Every live node by path relative to the root, with its totals.
fn snapshot(tree: &Tree) -> HashMap<PathBuf, (Totals, NodeState)> {
    let mut out = HashMap::new();
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        let node = tree.node(id);
        let rel: PathBuf = tree
            .relative_path(id, NodeId::ROOT)
            .unwrap()
            .iter()
            .map(|n| &**n)
            .collect();
        out.insert(rel, (*node.total(), node.state()));
        stack.extend(tree.children(id));
    }
    out
}

#[test]
fn streaming_invariants_and_accounting() {
    let (_dir, root) = fixture();
    let _unlock = build_tree(&root);
    let expected = expected_alloc(&root, &[root.join("mnt"), root.join("skipme")]);
    for threads in [1, 3, 8] {
        let req = ScanRequest {
            boundaries: fixture_boundaries(&root),
            ..ScanRequest::new(&root)
        };
        let Run {
            tree,
            summary,
            listed,
        } = run(req, threads);
        let ctx = format!("{threads} threads");
        let total = *tree.node(NodeId::ROOT).total();

        assert!(tree.node(NodeId::ROOT).is_done(), "{ctx}");
        assert!(!summary.cancelled, "{ctx}");
        assert_eq!(total.alloc, expected, "{ctx}");
        assert_eq!(summary.progress.alloc, expected, "{ctx}: summary == tree");
        assert_eq!(summary.hardlinks_deduped, 2, "{ctx}: f2 and f3");
        assert_eq!(summary.internal_errors, 0, "{ctx}");
        assert_eq!(tree.links().len(), 1, "{ctx}: one owner for f1's inode");
        // root a a/sub b b/c b/c/d empty locked wide; mnt and skipme are never listed.
        assert_eq!(listed, 9, "{ctx}");

        let b = find(&tree, &root, "b");
        assert_eq!(
            tree.node(b).files().count,
            2,
            "{ctx}: symlinks are files, never followed"
        );
        assert_eq!(tree.node(b).child_count(), 1, "{ctx}");
        let large: Vec<_> = (0..tree.len())
            .map(NodeId)
            .flat_map(|id| tree.node(id).large_files().iter().map(|f| f.name.clone()))
            .collect();
        assert_eq!(large, [Box::from(std::ffi::OsStr::new("big"))], "{ctx}");
        assert_eq!(
            total.linked_alloc,
            tree.node(find(&tree, &root, "a")).total().linked_alloc,
            "{ctx}"
        );
        assert!(total.linked_alloc > 0, "{ctx}");

        let mnt = find(&tree, &root, "mnt");
        assert_eq!(
            tree.node(mnt).state(),
            NodeState::Skipped(Skip::MountPoint),
            "{ctx}"
        );
        assert_eq!(
            tree.node(find(&tree, &root, "skipme")).state(),
            NodeState::Skipped(Skip::Excluded),
            "{ctx}"
        );
        assert_eq!(total.boundaries, 1, "{ctx}");

        if !is_root_user() {
            // Only `locked` is an error: nothing inside the mount point was touched.
            assert_eq!(summary.progress.errors, 1, "{ctx}");
            let locked = tree.node(find(&tree, &root, "locked"));
            let error = locked.error().expect("locked is unreadable");
            assert_eq!(
                (error.class, error.errno),
                (ErrorClass::PermissionDenied, Some(libc::EACCES)),
                "{ctx}"
            );
            assert_eq!(total.error_dirs, 1, "{ctx}");
            assert_eq!(tree.errors().len(), 1, "{ctx}");
        }
    }
}

#[test]
fn matches_du_per_folder() {
    let (_dir, root) = fixture();
    let _unlock = build_tree(&root);
    if std::process::Command::new("du")
        .arg("-sk")
        .arg(&root)
        .output()
        .is_err()
    {
        return; // no du on this system
    }
    let req = ScanRequest {
        boundaries: fixture_boundaries(&root),
        ..ScanRequest::new(&root)
    };
    let Run { tree, .. } = run(req, 4);
    // Folders without mount points or exclusions below them (du doesn't know ours). Not
    // `locked`: macOS du prints no size at all for an unreadable folder.
    for rel in ["a", "b", "empty", "wide"] {
        let out = std::process::Command::new("du")
            .arg("-skx")
            .arg(root.join(rel))
            .output()
            .unwrap();
        let du_kib: u64 = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let ours = tree.node(find(&tree, &root, rel)).total().alloc;
        assert_eq!(
            ours / 1024,
            du_kib,
            "{rel}: {ours} bytes vs du {du_kib} KiB"
        );
    }
}

/// Runs a hook when `Started` is delivered: before any worker exists.
struct OnStarted<F: FnMut() + Send + 'static> {
    inner: mpsc::SyncSender<Update>,
    hook: F,
}

impl<F: FnMut() + Send + 'static> UpdateSink for OnStarted<F> {
    fn try_deliver(&mut self, update: Update) -> Delivery {
        self.inner.try_deliver(update)
    }

    fn deliver_final(&mut self, update: Update) {
        if matches!(update, Update::Started(_)) {
            (self.hook)();
        }
        self.inner.deliver_final(update);
    }
}

#[test]
fn a_folder_that_vanishes_is_not_an_error() {
    let (_dir, base) = fixture();
    let root = base.join("going");
    fs::create_dir_all(root.join("x")).unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    let doomed = root.clone();
    let sink = OnStarted {
        inner: tx,
        hook: move || fs::remove_dir_all(&doomed).unwrap(),
    };
    let handle = scan::start(ScanRequest::new(&root), options(2), sink);

    let mut tree = Tree::opening(root);
    let summary = loop {
        match rx.recv().unwrap() {
            Update::Started(info) => tree.start(info),
            Update::Batch(batch) => batch.listings.into_iter().for_each(|l| {
                assert!(l.vanished && l.error.is_none() && l.subdirs.is_empty());
                tree.apply(l);
            }),
            Update::Finished(summary) => break summary,
            Update::Failed(e) => panic!("{}", e.message),
        }
    };
    handle.join();
    assert!(summary.progress.vanished > 0);
    assert_eq!(summary.progress.errors, 0);
    assert!(tree.errors().is_empty());
    assert!(tree.node(NodeId::ROOT).is_done());
}

#[test]
fn start_failures_send_failed_and_nothing_else() {
    let (_dir, root) = fixture();
    write(&root.join("file"), 10);
    for (path, kind) in [
        (root.join("missing"), std::io::ErrorKind::NotFound),
        (root.join("file"), std::io::ErrorKind::NotADirectory),
    ] {
        let (tx, rx) = mpsc::sync_channel(1);
        let handle = scan::start(ScanRequest::new(&path), options(2), tx);
        match rx.recv().unwrap() {
            Update::Failed(e) => assert_eq!(e.kind, kind, "{}", path.display()),
            other => panic!("expected Failed, got {other:?}"),
        }
        handle.join();
        assert!(rx.recv().is_err(), "nothing follows Failed");
    }
}

fn many_folders(root: &Path, dirs: usize, files: usize) {
    for d in 0..dirs {
        let dir = root.join(format!("d{d}/x/y"));
        fs::create_dir_all(&dir).unwrap();
        for f in 0..files {
            write(&dir.join(format!("f{f}")), 100);
        }
    }
}

#[test]
fn cancel_delivers_finished_quickly() {
    let (_dir, root) = fixture();
    many_folders(&root, 200, 30);
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = scan::start(ScanRequest::new(&root), options(4), tx);
    handle.cancel();
    let cancelled_at = Instant::now();
    let summary = loop {
        if let Update::Finished(summary) = rx.recv().unwrap() {
            break summary;
        }
    };
    assert!(summary.cancelled);
    assert!(cancelled_at.elapsed() < Duration::from_secs(1));
    handle.join();
}

#[test]
fn dropping_the_receiver_stops_the_scan() {
    let (_dir, root) = fixture();
    many_folders(&root, 100, 0);
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = scan::start(ScanRequest::new(&root), options(4), tx);
    drop(rx);
    handle.join(); // must terminate
}

#[test]
fn dropping_the_handle_cancels() {
    let (_dir, root) = fixture();
    many_folders(&root, 200, 30);
    let (tx, rx) = mpsc::sync_channel(1);
    drop(scan::start(ScanRequest::new(&root), options(4), tx));
    let started = Instant::now();
    let summary = loop {
        match rx
            .recv_timeout(Duration::from_secs(5))
            .expect("Finished within 5 s")
        {
            Update::Finished(summary) => break summary,
            _ => continue,
        }
    };
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(summary.cancelled || summary.progress.outstanding == 0);
}

/// Rescans `rel` and splices it in, as the app does after a rescan finishes.
fn rescan(tree: &mut Tree, root: &Path, rel: &str) {
    let target = find(tree, root, rel);
    let req = ScanRequest {
        counted_elsewhere: Some(Arc::new(tree.counted_elsewhere(target))),
        ..ScanRequest::new(tree.path_of(target))
    };
    let sub = run(req, 3).tree;
    tree.splice(target, sub);
}

#[test]
fn rescans_with_shared_hard_links_match_a_fresh_scan() {
    let (_dir, root) = fixture();
    fs::create_dir_all(root.join("x/deep")).unwrap();
    fs::create_dir_all(root.join("y")).unwrap();
    write(&root.join("x/shared"), 40_000);
    fs::hard_link(root.join("x/shared"), root.join("y/shared")).unwrap();
    fs::hard_link(root.join("x/shared"), root.join("x/deep/shared")).unwrap();
    write(&root.join("y/own"), 8000);

    let mut tree = run(ScanRequest::new(&root), 3).tree;
    // Whichever folder owns the inode, rescanning either side must not count it twice.
    // Inside the target the owner may move to another folder (the first sighting wins), so
    // nodes strictly inside it are compared only through the target's total.
    for rel in ["y", "x", "x/deep", "y"] {
        let outside =
            |snap: HashMap<PathBuf, (Totals, NodeState)>| -> HashMap<PathBuf, (Totals, NodeState)> {
                snap.into_iter()
                    .filter(|(p, _)| p == Path::new(rel) || !p.starts_with(rel))
                    .collect()
            };
        let before = outside(snapshot(&tree));
        rescan(&mut tree, &root, rel);
        assert_eq!(outside(snapshot(&tree)), before, "after rescanning {rel}");
    }
    let expected = expected_alloc(&root, &[]);
    assert_eq!(tree.node(NodeId::ROOT).total().alloc, expected);
}

#[test]
fn rescan_after_changes_matches_a_fresh_scan() {
    let (_dir, root) = fixture();
    many_folders(&root, 5, 10);
    let mut tree = run(ScanRequest::new(&root), 3).tree;

    fs::remove_dir_all(root.join("d1/x/y")).unwrap();
    fs::create_dir_all(root.join("d1/x/new/deeper")).unwrap();
    write(&root.join("d1/x/new/deeper/blob"), 3 << 20);
    write(&root.join("d1/x/top"), 500);
    rescan(&mut tree, &root, "d1/x");

    let fresh = run(ScanRequest::new(&root), 3).tree;
    assert_eq!(snapshot(&tree), snapshot(&fresh));
    assert!(tree.find_path(&root.join("d1/x/y")).is_none());
    let blob_dir = tree.node(find(&tree, &root, "d1/x/new/deeper"));
    assert_eq!(blob_dir.large_files().len(), 1);
}
