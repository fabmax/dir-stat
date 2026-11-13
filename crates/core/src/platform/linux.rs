//! Linux: mountinfo for the mount table, std's statx-based metadata.

use super::{EntryMeta, Mount, meta_from_raw, mountinfo};

pub fn init_process() {
    super::unix::raise_nofile_limit();
}

pub fn init_io_thread() {}

pub fn entry_meta(m: &std::fs::Metadata) -> EntryMeta {
    meta_from_raw(&super::unix::raw_stat(m))
}

pub fn mounts() -> Vec<Mount> {
    std::fs::read("/proc/self/mountinfo")
        .map(|text| mountinfo::parse(&text))
        .unwrap_or_default()
}
