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
    let mut mounts = std::fs::read("/proc/self/mountinfo")
        .map(|text| mountinfo::parse(&text))
        .unwrap_or_default();
    for mount in &mut mounts {
        // A dead network server would block statvfs in uninterruptible sleep (D11).
        if LOCAL_BLOCK_TYPES.contains(&mount.fs_type.as_str()) {
            mount.space = statvfs(&mount.path);
        }
    }
    mounts
}

/// Filesystems on local block devices: safe to statvfs.
const LOCAL_BLOCK_TYPES: &[&str] = &[
    "ext2", "ext3", "ext4", "xfs", "btrfs", "f2fs", "vfat", "exfat", "ntfs3", "zfs",
];

fn statvfs(path: &std::path::Path) -> Option<super::Space> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let block = s.f_frsize as u64;
    Some(super::Space {
        total: s.f_blocks as u64 * block,
        used: (s.f_blocks as u64).saturating_sub(s.f_bfree as u64) * block,
        free: s.f_bavail as u64 * block,
        inodes_used: Some((s.f_files as u64).saturating_sub(s.f_ffree as u64)),
    })
}

pub fn system_disk_name() -> String {
    "Computer".into()
}

pub fn full_disk_access() -> Option<bool> {
    None
}

pub fn is_read_only(path: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    unsafe { libc::statvfs(c_path.as_ptr(), &mut s) == 0 && s.f_flag & libc::ST_RDONLY != 0 }
}
