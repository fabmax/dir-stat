//! Helpers shared by the Unix implementations.

use std::os::unix::fs::MetadataExt;

use super::RawStat;

pub fn raw_stat(m: &std::fs::Metadata) -> RawStat {
    RawStat {
        mode: m.mode(),
        dev: m.dev(),
        ino: m.ino(),
        nlink: m.nlink(),
        blocks: m.blocks(),
        size: m.size(),
        mtime: m.mtime(),
        flags: 0,
    }
}

/// launchd and some shells start processes with a soft limit of 256 open files; each scan
/// worker holds one directory open at a time, plus git and trash (FS §7.2).
pub fn raise_nofile_limit() {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        return;
    }
    let wanted = limit.rlim_max.min(10_240);
    if limit.rlim_cur < wanted {
        limit.rlim_cur = wanted;
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
    }
}
