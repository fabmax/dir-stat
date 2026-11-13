//! `execute` against a temp dir with the recording trasher (design §10.1): every guard case
//! runs in plain `cargo test`, nothing reaches the real Trash.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::*;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    trasher: RecordingTrasher,
    guards: Guards,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap();
    let root = base.join("scan");
    let bin = base.join("trash");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let guards = Guards::for_home(base.join("home"), HashSet::new());
    Fixture {
        _dir: dir,
        root,
        trasher: RecordingTrasher::into_dir(bin),
        guards,
    }
}

fn identity(path: &Path) -> Identity {
    Identity::of(&fs::symlink_metadata(path).unwrap())
}

fn job(f: &Fixture, items: Vec<Item>) -> TrashJob {
    TrashJob {
        id: TrashJobId(1),
        tree_gen: 0,
        scan_root: f.root.clone(),
        items,
    }
}

fn item(path: &Path) -> Item {
    Item {
        path: path.into(),
        expect: identity(path),
        mode: Mode::Item,
    }
}

#[test]
fn files_and_folders_go_to_the_trash() {
    let f = fixture();
    let file = f.root.join("big.iso");
    let folder = f.root.join("target");
    fs::write(&file, b"x").unwrap();
    fs::create_dir_all(folder.join("debug")).unwrap();
    let outcomes = execute(
        &job(&f, vec![item(&file), item(&folder)]),
        &f.guards,
        &f.trasher,
    );
    assert_eq!(
        outcomes,
        [
            TrashOutcome::Trashed(file.clone()),
            TrashOutcome::Trashed(folder.clone())
        ]
    );
    assert!(!file.exists() && !folder.exists());
    assert_eq!(f.trasher.moved.lock().unwrap().len(), 2);
}

#[cfg(unix)]
#[test]
fn a_folder_replaced_by_a_symlink_is_changed() {
    let f = fixture();
    let folder = f.root.join("target");
    fs::create_dir(&folder).unwrap();
    let planned = item(&folder);
    fs::remove_dir(&folder).unwrap();
    std::os::unix::fs::symlink(&f.root, &folder).unwrap();
    let outcomes = execute(&job(&f, vec![planned]), &f.guards, &f.trasher);
    assert_eq!(
        outcomes,
        [TrashOutcome::Failed(folder.clone(), TrashError::Changed)]
    );
    assert!(folder.exists(), "nothing moved");
}

#[cfg(unix)]
#[test]
fn a_symlink_row_is_trashed_as_a_link() {
    let f = fixture();
    let real = f.root.join("real");
    fs::create_dir(&real).unwrap();
    let link = f.root.join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let outcomes = execute(&job(&f, vec![item(&link)]), &f.guards, &f.trasher);
    assert_eq!(outcomes, [TrashOutcome::Trashed(link.clone())]);
    assert!(real.exists(), "the target of the link stays");
}

#[cfg(target_os = "macos")]
#[test]
fn locked_items_are_refused() {
    let f = fixture();
    let file = f.root.join("locked");
    fs::write(&file, b"x").unwrap();
    let c = std::ffi::CString::new(file.to_str().unwrap()).unwrap();
    const UF_IMMUTABLE: u32 = 0x2;
    assert_eq!(unsafe { libc::chflags(c.as_ptr(), UF_IMMUTABLE) }, 0);
    let outcomes = execute(&job(&f, vec![item(&file)]), &f.guards, &f.trasher);
    unsafe { libc::chflags(c.as_ptr(), 0) };
    assert_eq!(
        outcomes,
        [TrashOutcome::Failed(
            file,
            TrashError::Refused(Refusal::ImmutableFlag)
        )]
    );
}

#[test]
fn guards_run_again_at_execution() {
    let f = fixture();
    let outcomes = execute(&job(&f, vec![item(&f.root)]), &f.guards, &f.trasher);
    assert_eq!(
        outcomes,
        [TrashOutcome::Failed(
            f.root.clone(),
            TrashError::Refused(Refusal::ScanRootOrAncestor)
        )]
    );
}

#[test]
fn os_failures_are_reported() {
    let f = fixture();
    let file = f.root.join("f");
    fs::write(&file, b"x").unwrap();
    f.trasher.fail.lock().unwrap().push(file.clone());
    let outcomes = execute(&job(&f, vec![item(&file)]), &f.guards, &f.trasher);
    assert_eq!(
        outcomes,
        [TrashOutcome::Failed(
            file.clone(),
            TrashError::Os("Operation not permitted".into())
        )]
    );
    assert!(file.exists());
}

#[test]
fn contents_mode_moves_only_what_is_unchanged() {
    let f = fixture();
    let cache = f.root.join("cache");
    fs::create_dir_all(cache.join("kept-dir")).unwrap();
    fs::create_dir_all(cache.join("changed-dir")).unwrap();
    fs::write(cache.join("old-small"), vec![b'x'; 5000]).unwrap();
    fs::write(cache.join("fails"), b"x").unwrap();
    let planned = vec![
        ContentsEntry {
            name: std::ffi::OsStr::new("kept-dir").into(),
            expect: identity(&cache.join("kept-dir")),
        },
        ContentsEntry {
            name: std::ffi::OsStr::new("changed-dir").into(),
            expect: identity(&cache.join("changed-dir")),
        },
    ];
    let scan_started = SystemTime::now() + Duration::from_secs(1);
    std::thread::sleep(Duration::from_millis(1100));
    // After the scan: a folder replaced (new identity) and a new file appear.
    fs::remove_dir(cache.join("changed-dir")).unwrap();
    fs::create_dir(cache.join("changed-dir")).unwrap();
    fs::write(cache.join("new-file"), b"new").unwrap();
    f.trasher.fail.lock().unwrap().push(cache.join("fails"));

    let contents = Item {
        path: cache.clone(),
        expect: identity(&cache),
        mode: Mode::Contents {
            entries: planned,
            scan_started,
        },
    };
    let outcomes = execute(&job(&f, vec![contents]), &f.guards, &f.trasher);
    let [TrashOutcome::Contents { path, outcome }] = outcomes.as_slice() else {
        panic!("{outcomes:?}")
    };
    assert_eq!(path, &cache);
    assert_eq!(
        outcome.trashed_dirs,
        [Box::from(std::ffi::OsStr::new("kept-dir"))]
    );
    assert_eq!(outcome.files_count, 1, "old-small");
    assert!(outcome.files_alloc > 0 && outcome.files_apparent == 5000);
    assert_eq!(outcome.left_in_place, 2, "changed-dir and new-file");
    assert_eq!(
        outcome.failures,
        [(
            cache.join("fails"),
            TrashError::Os("Operation not permitted".into())
        )]
    );
    assert!(cache.exists(), "the folder itself stays");
    assert!(cache.join("new-file").exists() && cache.join("changed-dir").exists());
}
