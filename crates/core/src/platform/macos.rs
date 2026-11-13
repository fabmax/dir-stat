//! macOS: `getmntinfo`, `st_flags`, and the I/O policies that keep a scan from downloading
//! iCloud content, updating atimes or triggering automounts (FS §1.11, §2.11; SCAN §7.11).

use std::ffi::{CStr, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use super::{EntryMeta, Mount, Space, meta_from_raw};

const IOPOL_TYPE_VFS_ATIME_UPDATES: libc::c_int = 2;
const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: libc::c_int = 3;
const IOPOL_TYPE_VFS_TRIGGER_RESOLVE: libc::c_int = 5;
const IOPOL_SCOPE_PROCESS: libc::c_int = 0;
const IOPOL_SCOPE_THREAD: libc::c_int = 1;
/// The `*_OFF` value of all three policies.
const IOPOL_OFF: libc::c_int = 1;

unsafe extern "C" {
    fn setiopolicy_np(iotype: libc::c_int, scope: libc::c_int, policy: libc::c_int) -> libc::c_int;
}

pub fn init_process() {
    // Never mount autofs maps just because the scan looked at them.
    unsafe {
        setiopolicy_np(
            IOPOL_TYPE_VFS_TRIGGER_RESOLVE,
            IOPOL_SCOPE_PROCESS,
            IOPOL_OFF,
        )
    };
    super::unix::raise_nofile_limit();
}

pub fn init_io_thread() {
    unsafe {
        setiopolicy_np(
            IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
            IOPOL_SCOPE_THREAD,
            IOPOL_OFF,
        );
        setiopolicy_np(IOPOL_TYPE_VFS_ATIME_UPDATES, IOPOL_SCOPE_THREAD, IOPOL_OFF);
    }
}

pub fn entry_meta(m: &std::fs::Metadata) -> EntryMeta {
    use std::os::macos::fs::MetadataExt as _;
    let mut raw = super::unix::raw_stat(m);
    raw.flags = m.st_flags();
    meta_from_raw(&raw)
}

pub fn mounts() -> Vec<Mount> {
    let mut buf: *mut libc::statfs = std::ptr::null_mut();
    // MNT_NOWAIT: cached statistics only, never a call into a dead network filesystem.
    let n = unsafe { libc::getmntinfo(&mut buf, libc::MNT_NOWAIT) };
    if n <= 0 || buf.is_null() {
        return Vec::new();
    }
    let entries = unsafe { std::slice::from_raw_parts(buf, n as usize) };
    entries.iter().map(mount_from_statfs).collect()
}

fn mount_from_statfs(s: &libc::statfs) -> Mount {
    let text = |chars: &[libc::c_char]| {
        unsafe { CStr::from_ptr(chars.as_ptr()) }
            .to_bytes()
            .to_vec()
    };
    let flags = s.f_flags;
    let block = u64::from(s.f_bsize);
    Mount {
        path: PathBuf::from(OsStr::from_bytes(&text(&s.f_mntonname))),
        fs_type: String::from_utf8_lossy(&text(&s.f_fstypename)).into_owned(),
        local: flags & libc::MNT_LOCAL as u32 != 0,
        read_only: flags & libc::MNT_RDONLY as u32 != 0,
        browsable: flags & libc::MNT_DONTBROWSE as u32 == 0,
        automounted: flags & libc::MNT_AUTOMOUNTED as u32 != 0,
        dev: None,
        // APFS reports the container here (FS §2.11): "used" is the container view.
        space: Some(Space {
            total: s.f_blocks * block,
            used: s.f_blocks.saturating_sub(s.f_bavail) * block,
            free: s.f_bavail * block,
            inodes_used: Some(s.f_files.saturating_sub(s.f_ffree)),
        }),
    }
}

pub fn system_disk_name() -> String {
    let named_root = std::fs::read_dir("/Volumes").ok().and_then(|entries| {
        entries.flatten().find_map(|e| {
            let target = std::fs::read_link(e.path()).ok()?;
            (target == std::path::Path::new("/"))
                .then(|| e.file_name().to_string_lossy().into_owned())
        })
    });
    named_root.unwrap_or_else(|| "Macintosh HD".into())
}

pub fn full_disk_access() -> Option<bool> {
    // Listing this folder needs Full Disk Access; it exists on every macOS.
    Some(std::fs::read_dir("/Library/Application Support/com.apple.TCC").is_ok())
}

pub fn is_read_only(path: &std::path::Path) -> bool {
    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut s: libc::statfs = unsafe { std::mem::zeroed() };
    unsafe {
        libc::statfs(c_path.as_ptr(), &mut s) == 0 && s.f_flags & libc::MNT_RDONLY as u32 != 0
    }
}
