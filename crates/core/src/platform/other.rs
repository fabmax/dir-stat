//! Other Unixes: std metadata, no mount table (the `st_dev` check still stops at other
//! filesystems).

use super::{EntryMeta, Mount, meta_from_raw};

pub fn init_process() {
    super::unix::raise_nofile_limit();
}

pub fn init_io_thread() {}

pub fn entry_meta(m: &std::fs::Metadata) -> EntryMeta {
    meta_from_raw(&super::unix::raw_stat(m))
}

pub fn mounts() -> Vec<Mount> {
    Vec::new()
}

pub fn system_disk_name() -> String {
    "Computer".into()
}

pub fn full_disk_access() -> Option<bool> {
    None
}

pub fn is_read_only(_path: &std::path::Path) -> bool {
    false
}
