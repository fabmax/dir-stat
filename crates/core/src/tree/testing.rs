//! Builds trees from nested literals, without a scanner or a disk (feature `test-util`).
//!
//! ```ignore
//! let builder = TreeBuilder::new("/r", dir("r").file("a.bin", 2 << 20).child(dir("src").small_files(3, 12_000)));
//! let tree = builder.build();
//! ```
//! Ids are allocated like the scanner does: each listed directory's subdirectories get the
//! next consecutive block, in listing order (breadth first).

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::path::PathBuf;

use super::Tree;
use crate::catalog::{RootContext, Verdict};
use crate::scan::{
    DirListing, ErrorClass, FileEntry, LARGE_FILE, NodeId, RootInfo, ScanError, Skip, Subdir,
    Summary, Usage,
};

/// A directory literal.
#[derive(Clone, Debug)]
pub struct Dir {
    name: Box<OsStr>,
    ino: u64,
    dir_alloc: u64,
    files: Usage,
    linked_alloc: u64,
    cloud_files: u32,
    large_files: Vec<FileEntry>,
    subdirs: Vec<Dir>,
    skip: Option<Skip>,
    error: Option<ScanError>,
    vanished: bool,
    entry_errors: u32,
    entry_error_samples: Vec<(Box<OsStr>, ScanError)>,
    newest_mtime: i64,
    markers: u64,
    verdict: Verdict,
}

/// A directory with 4 kB of its own blocks and nothing in it.
pub fn dir(name: impl AsRef<OsStr>) -> Dir {
    Dir {
        name: name.as_ref().into(),
        ino: 0,
        dir_alloc: 4096,
        files: Usage::default(),
        linked_alloc: 0,
        cloud_files: 0,
        large_files: Vec::new(),
        subdirs: Vec::new(),
        skip: None,
        error: None,
        vanished: false,
        entry_errors: 0,
        entry_error_samples: Vec::new(),
        newest_mtime: i64::MIN,
        markers: 0,
        verdict: Verdict::None,
    }
}

/// A large-file entry with `apparent == alloc`, mtime 0 and one link.
pub fn large(name: impl AsRef<OsStr>, alloc: u64) -> FileEntry {
    FileEntry {
        name: name.as_ref().into(),
        alloc,
        apparent: alloc,
        mtime: 0,
        ino: 0,
        nlink: 1,
        matched: None,
    }
}

pub fn error(class: ErrorClass) -> ScanError {
    ScanError { class, errno: None }
}

impl Dir {
    pub fn child(mut self, child: Dir) -> Self {
        self.subdirs.push(child);
        self
    }

    /// One file with `apparent == alloc`; files of at least 1 MiB are also large files.
    pub fn file(self, name: impl AsRef<OsStr>, alloc: u64) -> Self {
        if alloc >= LARGE_FILE {
            self.large(large(name, alloc))
        } else {
            self.small_files(1, alloc)
        }
    }

    /// A large file, counted in the directory's usage too.
    pub fn large(mut self, entry: FileEntry) -> Self {
        self.add_usage(1, entry.alloc, entry.apparent, entry.nlink > 1);
        self.newest_mtime = self.newest_mtime.max(entry.mtime);
        self.large_files.push(entry);
        self
    }

    /// `count` small files totalling `alloc` bytes.
    pub fn small_files(mut self, count: u64, alloc: u64) -> Self {
        self.add_usage(count, alloc, alloc, false);
        self
    }

    /// One small hard-linked file (`nlink > 1`).
    pub fn linked_file(mut self, alloc: u64) -> Self {
        self.add_usage(1, alloc, alloc, true);
        self
    }

    fn add_usage(&mut self, count: u64, alloc: u64, apparent: u64, linked: bool) {
        self.files.count += count;
        self.files.alloc += alloc;
        self.files.apparent += apparent;
        if linked {
            self.linked_alloc += alloc;
        }
    }

    pub fn dir_alloc(mut self, alloc: u64) -> Self {
        self.dir_alloc = alloc;
        self
    }
    pub fn ino(mut self, ino: u64) -> Self {
        self.ino = ino;
        self
    }
    pub fn cloud_files(mut self, n: u32) -> Self {
        self.cloud_files = n;
        self
    }
    /// Announced by the parent but never listed.
    pub fn skip(mut self, skip: Skip) -> Self {
        self.skip = Some(skip);
        self
    }
    pub fn error(mut self, class: ErrorClass) -> Self {
        self.error = Some(error(class));
        self
    }
    pub fn entry_error(mut self, name: impl AsRef<OsStr>, class: ErrorClass) -> Self {
        self.entry_errors += 1;
        self.entry_error_samples
            .push((name.as_ref().into(), error(class)));
        self
    }
    pub fn vanished(mut self) -> Self {
        self.vanished = true;
        self
    }
    pub fn mtime(mut self, mtime: i64) -> Self {
        self.newest_mtime = self.newest_mtime.max(mtime);
        self
    }
    pub fn markers(mut self, markers: u64) -> Self {
        self.markers = markers;
        self
    }
    pub fn verdict(mut self, verdict: Verdict) -> Self {
        self.verdict = verdict;
        self
    }
}

pub struct TreeBuilder {
    root_path: PathBuf,
    root: Dir,
}

impl TreeBuilder {
    pub fn new(root_path: impl Into<PathBuf>, root: Dir) -> Self {
        Self {
            root_path: root_path.into(),
            root,
        }
    }

    pub fn root_info(&self) -> RootInfo {
        RootInfo {
            root: self.root_path.clone(),
            dev: 1,
            ino: self.root.ino,
            fs_type: "apfs".into(),
            context: RootContext::default(),
        }
    }

    /// The listings a scanner would send, breadth first (parents before children).
    pub fn listings(&self) -> Vec<DirListing> {
        let mut listings = Vec::new();
        let mut next = 1;
        let mut queue = VecDeque::from([(&self.root, NodeId::ROOT)]);
        while let Some((d, id)) = queue.pop_front() {
            let first_child = NodeId(next);
            next += d.subdirs.len() as u32;
            for (i, child) in d.subdirs.iter().enumerate() {
                if child.skip.is_none() {
                    queue.push_back((child, NodeId(first_child.0 + i as u32)));
                }
            }
            listings.push(DirListing {
                id,
                dir_alloc: d.dir_alloc,
                files: d.files,
                linked_alloc: d.linked_alloc,
                cloud_files: d.cloud_files,
                large_files: d.large_files.clone(),
                first_child,
                subdirs: d
                    .subdirs
                    .iter()
                    .map(|c| Subdir {
                        name: c.name.clone(),
                        ino: c.ino,
                        skip: c.skip,
                    })
                    .collect(),
                error: d.error,
                vanished: d.vanished,
                entry_errors: d.entry_errors,
                entry_error_samples: d.entry_error_samples.clone(),
                newest_mtime: d.newest_mtime,
                markers: d.markers,
                verdict: d.verdict,
            });
        }
        listings
    }

    /// A started tree with no listings applied.
    pub fn started(&self) -> Tree {
        let mut tree = Tree::opening(self.root_path.clone());
        tree.start(self.root_info());
        tree
    }

    /// Every listing applied and the scan finished.
    pub fn build(&self) -> Tree {
        let mut tree = self.started();
        for listing in self.listings() {
            tree.apply(listing);
        }
        tree.finish(&Summary::default());
        tree
    }
}
