//! Crate-based alternatives, each computing the same totals as the reference scanner
//! (du -skx semantics: st_blocks*512, hard links once, no symlink following, stay on the root's
//! device, directory inodes counted). They only produce totals - none of them streams
//! per-directory results the way `scanner.rs` does; that would have to be built on top.

use crate::listing::{meta_from_std, Meta};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Default)]
pub struct Totals {
    pub dirs: u64,
    pub files: u64,
    pub alloc: u64,
    pub apparent: u64,
    pub errors: u64,
    pub hardlinks_deduped: u64,
}

impl Totals {
    fn merge(&mut self, o: &Totals) {
        self.dirs += o.dirs;
        self.files += o.files;
        self.alloc += o.alloc;
        self.apparent += o.apparent;
        self.errors += o.errors;
        self.hardlinks_deduped += o.hardlinks_deduped;
    }

    /// Account one entry. `is_dir` entries must already be known to be traversed (same device).
    fn add(&mut self, is_dir: bool, m: &Meta, hardlinks: &Mutex<HashSet<(u64, u64)>>) {
        if is_dir {
            self.dirs += 1;
            self.alloc += m.alloc;
            self.apparent += m.apparent;
            return;
        }
        self.files += 1;
        if m.nlink > 1 && !hardlinks.lock().unwrap().insert((m.dev, m.ino)) {
            self.hardlinks_deduped += 1;
            return;
        }
        self.alloc += m.alloc;
        self.apparent += m.apparent;
    }
}

fn root_dev(root: &Path) -> u64 {
    meta_from_std(&std::fs::metadata(root).expect("stat root")).dev
}

/// walkdir: single-threaded, full-path lstat per entry.
pub fn walkdir(root: &Path) -> Totals {
    let hl = Mutex::new(HashSet::new());
    let mut t = Totals::default();
    for entry in walkdir::WalkDir::new(root).follow_links(false).same_file_system(true) {
        match entry.and_then(|e| e.metadata().map(|m| (e.file_type().is_dir(), m))) {
            Ok((is_dir, m)) => t.add(is_dir, &meta_from_std(&m), &hl),
            Err(_) => t.errors += 1,
        }
    }
    t
}

/// jwalk 0.9 (rayon). Metadata is fetched inside `process_read_dir`, i.e. on the rayon workers.
pub fn jwalk(root: &Path, threads: usize) -> Totals {
    type State = ((), Option<Result<Meta, ()>>);
    let dev = root_dev(root);
    let walk = jwalk::WalkDirGeneric::<State>::new(root)
        .skip_hidden(false)
        .follow_links(false)
        .sort(false)
        .parallelism(jwalk::Parallelism::RayonNewPool(threads))
        .process_read_dir(move |_depth, _path, _state, children| {
            for child in children.iter_mut().flatten() {
                let meta = child.metadata().map(|m| meta_from_std(&m)).map_err(|_| ());
                if child.file_type.is_dir() && meta.as_ref().map_or(true, |m| m.dev != dev) {
                    child.read_children = None; // other filesystem / unreadable: don't descend
                }
                child.client_state = Some(meta);
            }
        });
    let hl = Mutex::new(HashSet::new());
    let mut t = Totals::default();
    for entry in walk {
        match entry {
            Ok(e) => {
                let meta = match e.client_state {
                    Some(m) => m,
                    None => e.metadata().map(|m| meta_from_std(&m)).map_err(|_| ()), // the root
                };
                match meta {
                    Ok(m) if e.file_type.is_dir() && m.dev != dev => {} // mount point, skipped
                    Ok(m) => t.add(e.file_type.is_dir(), &m, &hl),
                    Err(()) => t.errors += 1,
                }
            }
            Err(_) => t.errors += 1,
        }
    }
    t
}

/// ignore::WalkParallel (ripgrep's walker) with all filtering disabled.
pub fn ignore(root: &Path, threads: usize) -> Totals {
    struct Local {
        t: Totals,
        sink: Arc<Mutex<Totals>>,
    }
    impl Drop for Local {
        fn drop(&mut self) {
            self.sink.lock().unwrap().merge(&self.t);
        }
    }
    let sink = Arc::new(Mutex::new(Totals::default()));
    let hl = Arc::new(Mutex::new(HashSet::new()));
    ignore::WalkBuilder::new(root)
        .standard_filters(false)
        .follow_links(false)
        .same_file_system(true)
        .threads(threads)
        .build_parallel()
        .run(|| {
            let mut local = Local { t: Totals::default(), sink: Arc::clone(&sink) };
            let hl = Arc::clone(&hl);
            Box::new(move |entry| {
                // NB: go through `&mut local` (not `local.t`): with edition-2021+ disjoint closure
                // captures, `local.t` alone would be captured *by copy* (Totals: Copy) and the
                // merging `Drop` of `local` would run immediately with empty totals.
                let local = &mut local;
                match entry.and_then(|e| e.metadata().map(|m| (e.file_type().is_some_and(|f| f.is_dir()), m))) {
                    Ok((is_dir, m)) => local.t.add(is_dir, &meta_from_std(&m), &hl),
                    Err(_) => local.t.errors += 1,
                }
                ignore::WalkState::Continue
            })
        });
    *sink.lock().unwrap()
}

/// dua-core 6 (jwalk's successor; work-stealing pool, getattrlistbulk on macOS).
pub fn dua(root: &Path, threads: usize, parent_first: bool) -> Totals {
    #[cfg(not(target_os = "macos"))]
    use std::os::unix::fs::MetadataExt;
    let dev = root_dev(root);
    let order = if parent_first { dua_core::Order::ParentFirst } else { dua_core::Order::Completion };
    let walk = dua_core::walk(root, threads, order, dua_core::Options::default(), move |e: &dua_core::Entry| {
        matches!(&e.metadata, Some(Ok(m)) if m.dev() == dev)
    });
    let hl = Mutex::new(HashSet::new());
    let mut t = Totals::default();
    for entry in walk {
        let Ok(e) = entry else {
            t.errors += 1;
            continue;
        };
        match &e.metadata {
            Some(Ok(m)) => {
                let is_dir = e.file_type.is_dir();
                if is_dir && m.dev() != dev {
                    continue;
                }
                let meta = Meta {
                    dev: m.dev(),
                    ino: m.ino(),
                    nlink: m.nlink(),
                    alloc: m.blocks() * 512,
                    apparent: m.len(),
                    mount_point: false,
                };
                t.add(is_dir, &meta, &hl);
            }
            _ => t.errors += 1,
        }
    }
    t
}
