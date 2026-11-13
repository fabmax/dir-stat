//! Windows (nice-to-have, D12): logical sizes, no hard-link identity, no mount table.
//! Junctions and symlinks are never followed.

use std::time::UNIX_EPOCH;

use super::{EntryKind, EntryMeta, Mount};

pub fn init_process() {}

pub fn init_io_thread() {}

pub fn entry_meta(m: &std::fs::Metadata) -> EntryMeta {
    let file_type = m.file_type();
    let kind = if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Dir
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    let mtime = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    EntryMeta {
        kind,
        dev: 0,
        ino: 0,
        nlink: 1,
        alloc: m.len(),
        apparent: m.len(),
        mtime,
        dataless: false,
        flags: 0,
    }
}

pub fn mounts() -> Vec<Mount> {
    Vec::new()
}

pub fn system_disk_name() -> String {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    format!("Local Disk ({drive})")
}

pub fn full_disk_access() -> Option<bool> {
    None
}

pub fn is_read_only(_path: &std::path::Path) -> bool {
    false
}
