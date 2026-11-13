//! Sends one temp file through the real OS Trash. Ignored by default, because it writes to
//! the user's Trash: run manually with `cargo test -p dirstat-core --test trash_smoke -- --ignored`.

use std::collections::HashSet;

use dirstat_core::safety::Guards;
use dirstat_core::trash::{
    Identity, Item, Mode, SystemTrasher, TrashJob, TrashJobId, TrashOutcome, execute,
};

#[test]
#[ignore = "writes to the real Trash"]
fn a_temp_file_reaches_the_real_trash() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let file = root.join("dir-stat-trash-smoke-test.txt");
    std::fs::write(
        &file,
        b"created by the dir-stat trash smoke test; safe to delete",
    )
    .unwrap();
    let expect = Identity::of(&std::fs::symlink_metadata(&file).unwrap());
    let job = TrashJob {
        id: TrashJobId(1),
        tree_gen: 0,
        scan_root: root.clone(),
        items: vec![Item {
            path: file.clone(),
            expect,
            mode: Mode::Item,
        }],
    };
    let guards = Guards::for_home(root.join("not-home"), HashSet::new());
    let outcomes = execute(&job, &guards, &SystemTrasher);
    assert_eq!(outcomes, [TrashOutcome::Trashed(file.clone())]);
    assert!(!file.exists());
}
