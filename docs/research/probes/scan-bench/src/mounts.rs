//! Mount table lookup, used for the mount-point policy.
//!
//! Comparing `st_dev` is not enough on its own:
//! * macOS: the APFS *volume group* (sealed System volume + Data volume) reports ONE `st_dev`, and
//!   the Data volume's top-level folders (`/Users`, `/Applications`, `/Library`, `/private`, `/opt`,
//!   `/usr/local`, ... see `/usr/share/firmlinks`) are *firmlinked* into the read-only system tree:
//!   `/Users` and `/System/Volumes/Data/Users` have the same (dev, ino). A walker started at `/`
//!   therefore reaches the same data twice. `/System/Volumes/Data` *is* listed in the mount table,
//!   so treating every mount-table entry below the root as a boundary skips the duplicate while
//!   the firmlinked paths (which are not mounts) are still scanned - user data shows up at the
//!   paths the user knows (`/Users/...`), counted once. This is also what `gdu -x` does.
//! * Linux: bind mounts of the same filesystem keep `st_dev`; they appear in the mount table.
//!
//! The result is a set of absolute mount-point paths strictly below `root`.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::Path;

pub fn mount_points_below(root: &Path) -> HashSet<OsString> {
    all_mount_points()
        .into_iter()
        .filter(|m| {
            let m = Path::new(m);
            m != root && m.starts_with(root)
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn all_mount_points() -> Vec<OsString> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;
    let mut buf: *mut libc::statfs = std::ptr::null_mut();
    // MNT_NOWAIT: use cached information, never block on unresponsive (network) filesystems.
    let n = unsafe { libc::getmntinfo(&mut buf, libc::MNT_NOWAIT) };
    if n <= 0 || buf.is_null() {
        return Vec::new();
    }
    let mounts = unsafe { std::slice::from_raw_parts(buf, n as usize) };
    mounts
        .iter()
        .map(|m| {
            let p = unsafe { CStr::from_ptr(m.f_mntonname.as_ptr()) };
            std::ffi::OsStr::from_bytes(p.to_bytes()).to_owned()
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn all_mount_points() -> Vec<OsString> {
    use std::os::unix::ffi::OsStringExt;
    // /proc/self/mountinfo: "36 35 98:0 /mnt1 /mnt/parent rw,noatime master:1 - ext3 /dev/root rw"
    // field 5 is the mount point, with space/tab/newline/backslash escaped as \ooo.
    let Ok(text) = std::fs::read("/proc/self/mountinfo") else { return Vec::new() };
    let mut out = Vec::new();
    for line in text.split(|&b| b == b'\n') {
        let Some(field) = line.split(|&b| b == b' ').nth(4) else { continue };
        let mut path = Vec::with_capacity(field.len());
        let mut i = 0;
        while i < field.len() {
            if field[i] == b'\\' && i + 3 < field.len() && field[i + 1..i + 4].iter().all(u8::is_ascii_digit) {
                let v = (field[i + 1] - b'0') * 64 + (field[i + 2] - b'0') * 8 + (field[i + 3] - b'0');
                path.push(v);
                i += 4;
            } else {
                path.push(field[i]);
                i += 1;
            }
        }
        out.push(OsString::from_vec(path));
    }
    out
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn all_mount_points() -> Vec<OsString> {
    Vec::new()
}
