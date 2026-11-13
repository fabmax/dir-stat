//! On-demand listing of one folder's small files, when its "≡ N files" row is expanded.
//! The scanner keeps only files of 1 MiB or more by name (UX §6.3).

use std::ffi::OsStr;
use std::path::Path;

use crate::platform::{self, EntryKind};
use crate::scan::LARGE_FILE;
use crate::tree::FileFlags;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmallFile {
    pub name: Box<OsStr>,
    pub alloc: u64,
    pub apparent: u64,
    pub mtime: i64,
    pub ino: u64,
    pub kind: EntryKind,
    pub nlink: u32,
    pub flags: FileFlags,
}

/// The non-directory entries below the large-file threshold, largest first. One `read_dir`;
/// symlinks are listed as links, never followed. Runs on a blocking thread.
pub fn list(dir: &Path) -> Result<Vec<SmallFile>, String> {
    let dir_meta = std::fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
    // Reading a cloud placeholder would download it.
    if platform::entry_meta(&dir_meta).dataless {
        return Err("In iCloud – not listed".into());
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let Ok(entry) = entry else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        let meta = platform::entry_meta(&meta);
        if meta.kind == EntryKind::Dir || meta.alloc.max(meta.apparent) >= LARGE_FILE {
            continue;
        }
        files.push(SmallFile {
            name: entry.file_name().into(),
            alloc: meta.alloc,
            apparent: meta.apparent,
            mtime: meta.mtime,
            ino: meta.ino,
            kind: meta.kind,
            nlink: meta.nlink.min(u64::from(u32::MAX)) as u32,
            flags: FileFlags::default(),
        });
    }
    files.sort_by(|a, b| b.alloc.cmp(&a.alloc).then_with(|| a.name.cmp(&b.name)));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_only_small_non_directories_largest_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), vec![b'x'; 10]).unwrap();
        std::fs::write(root.join("b.txt"), vec![b'x'; 100_000]).unwrap();
        std::fs::write(root.join("big.bin"), vec![b'x'; 2 << 20]).unwrap();
        std::fs::create_dir(root.join("sub")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/nonexistent", root.join("link")).unwrap();

        let files = list(root).unwrap();
        let names: Vec<_> = files.iter().map(|f| f.name.to_str().unwrap()).collect();
        assert_eq!(&names[..2], ["b.txt", "a.txt"]);
        assert!(!names.contains(&"big.bin") && !names.contains(&"sub"));
        #[cfg(unix)]
        assert!(
            files
                .iter()
                .any(|f| &*f.name == "link" && f.kind == EntryKind::Symlink)
        );
        assert!(list(&root.join("missing")).is_err());
    }
}
