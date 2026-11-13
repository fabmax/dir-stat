//! Directory listing backends.
//!
//! Every backend lists exactly one directory and reports, for every entry except `.`/`..`:
//! its name, its kind (never following symlinks) and lstat-like metadata. The scanner only
//! ever needs this one primitive, so the backend is an implementation detail that can be swapped
//! per platform without touching the scanning logic.
//!
//! | backend | syscalls per directory with N entries | platforms |
//! |---------|----------------------------------------|-----------|
//! | `Std`   | open + getdirentries + N x `fstatat`  (std >= 1.9x on Apple, statx on Linux) | all |
//! | `At`    | same syscalls as `Std`, but no per-entry heap allocation / Arc clone | unix |
//! | `Bulk`  | open + ~N/40 x `getattrlistbulk` (+ nothing per entry) | macOS |
//!
//! Since Rust 1.9x (checked: 1.93.1, *not* 1.89) `std::fs::DirEntry::metadata()` on Apple targets
//! is `fstatat(dirfd, name, AT_SYMLINK_NOFOLLOW)` (relative to the open directory, no full path
//! resolution); on Linux/glibc it is `statx(dirfd, name, AT_SYMLINK_NOFOLLOW)`. Older Rust versions
//! did a full-path `lstat` on macOS. `DirEntry::file_type()` is free on APFS/ext4/btrfs/xfs (d_type).

use std::ffi::OsStr;
use std::io;
use std::path::Path;

/// What kind of filesystem object an entry is. Symlinks are never followed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
    Symlink,
    /// Sockets, FIFOs, device nodes, ...
    Other,
}

/// The subset of `struct stat` the scanner needs.
#[derive(Clone, Copy, Debug, Default)]
pub struct Meta {
    pub dev: u64,
    pub ino: u64,
    pub nlink: u64,
    /// Allocated size on disk: `st_blocks * 512` (what `du` reports).
    pub alloc: u64,
    /// Logical ("apparent") size: `st_size`.
    pub apparent: u64,
    /// Set by backends that can tell cheaply that a directory is a mount point (Bulk only).
    /// The others detect mount points by comparing `dev` with the scan root's device.
    pub mount_point: bool,
}

/// One directory entry, borrowed from the backend's buffers for the duration of the callback.
pub struct Entry<'a> {
    pub name: &'a OsStr,
    pub kind: Kind,
    /// `Err` if the entry could be listed but not stat'ed (vanished, EACCES, ...).
    pub meta: io::Result<Meta>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Std,
    #[cfg(unix)]
    At,
    #[cfg(target_os = "macos")]
    Bulk,
}

impl Backend {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "std" => Some(Self::Std),
            #[cfg(unix)]
            "at" => Some(Self::At),
            #[cfg(target_os = "macos")]
            "bulk" => Some(Self::Bulk),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Std => "std",
            #[cfg(unix)]
            Self::At => "at",
            #[cfg(target_os = "macos")]
            Self::Bulk => "bulk",
        }
    }
}

/// Lists `dir` and calls `f` for every entry. `f` returns `false` to stop early (cancellation).
///
/// Returns `Err` if the directory cannot be opened, or if reading fails part-way (then the entries
/// delivered before the error are still valid; the caller records the directory as incomplete).
pub fn list_dir(backend: Backend, dir: &Path, f: &mut dyn FnMut(Entry<'_>) -> bool) -> io::Result<()> {
    match backend {
        Backend::Std => list_std(dir, f),
        #[cfg(unix)]
        Backend::At => unix_at::list(dir, f),
        #[cfg(target_os = "macos")]
        Backend::Bulk => macos_bulk::list(dir, f),
    }
}

fn kind_of(ft: std::fs::FileType) -> Kind {
    if ft.is_dir() {
        Kind::Dir
    } else if ft.is_file() {
        Kind::File
    } else if ft.is_symlink() {
        Kind::Symlink
    } else {
        Kind::Other
    }
}

#[cfg(unix)]
pub fn meta_from_std(m: &std::fs::Metadata) -> Meta {
    use std::os::unix::fs::MetadataExt;
    Meta {
        dev: m.dev(),
        ino: m.ino(),
        nlink: m.nlink(),
        alloc: m.blocks() * 512,
        apparent: m.len(),
        mount_point: false,
    }
}

/// Windows (nice-to-have): std's metadata comes for free from FindNextFileW. There is no cheap
/// allocated size (that needs GetCompressedFileSizeW per file), so use the logical size and treat
/// every file as having a single link.
#[cfg(not(unix))]
pub fn meta_from_std(m: &std::fs::Metadata) -> Meta {
    Meta { dev: 0, ino: 0, nlink: 1, alloc: m.len(), apparent: m.len(), mount_point: false }
}

/// Portable backend: `std::fs::read_dir` + `DirEntry::metadata()` (fstatat / statx, see module docs).
fn list_std(dir: &Path, f: &mut dyn FnMut(Entry<'_>) -> bool) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let (kind, meta) = match entry.metadata() {
            Ok(m) => (kind_of(m.file_type()), Ok(meta_from_std(&m))),
            Err(e) => (entry.file_type().map(kind_of).unwrap_or(Kind::Other), Err(e)),
        };
        if !f(Entry { name: &name, kind, meta }) {
            break;
        }
    }
    Ok(())
}

/// Same syscalls as `Std` (open/getdirentries/fstatat), but borrows names straight from the
/// `dirent` buffer: no per-entry `CString` + `Arc` clone. Measures the overhead of std itself.
#[cfg(unix)]
mod unix_at {
    use super::{Entry, Kind, Meta};
    use std::ffi::{CStr, CString, OsStr};
    use std::io;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    struct DirStream(*mut libc::DIR);
    impl Drop for DirStream {
        fn drop(&mut self) {
            // closedir also closes the fd that was handed to fdopendir.
            unsafe { libc::closedir(self.0) };
        }
    }

    #[cfg(target_vendor = "apple")]
    unsafe fn errno_location() -> *mut libc::c_int {
        unsafe { libc::__error() }
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    unsafe fn errno_location() -> *mut libc::c_int {
        unsafe { libc::__errno_location() }
    }
    #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
    unsafe fn errno_location() -> *mut libc::c_int {
        unsafe { libc::__error() }
    }

    pub fn open_dir_fd(dir: &Path) -> io::Result<libc::c_int> {
        let c = CString::new(dir.as_os_str().as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        // O_NOFOLLOW: if the directory was replaced by a symlink since its parent was listed, fail
        // instead of following it (we never follow symlinks).
        let fd = unsafe {
            libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        };
        if fd < 0 { Err(io::Error::last_os_error()) } else { Ok(fd) }
    }

    fn kind_from_mode(mode: libc::mode_t) -> Kind {
        match mode & libc::S_IFMT {
            libc::S_IFDIR => Kind::Dir,
            libc::S_IFREG => Kind::File,
            libc::S_IFLNK => Kind::Symlink,
            _ => Kind::Other,
        }
    }

    fn kind_from_dtype(t: u8) -> Kind {
        match t {
            libc::DT_DIR => Kind::Dir,
            libc::DT_REG => Kind::File,
            libc::DT_LNK => Kind::Symlink,
            _ => Kind::Other,
        }
    }

    pub fn list(dir: &Path, f: &mut dyn FnMut(Entry<'_>) -> bool) -> io::Result<()> {
        let fd = open_dir_fd(dir)?;
        let dirp = unsafe { libc::fdopendir(fd) };
        if dirp.is_null() {
            let e = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e);
        }
        let stream = DirStream(dirp);
        loop {
            // readdir returns NULL both at the end and on error; only errno tells them apart.
            unsafe { *errno_location() = 0 };
            let ent = unsafe { libc::readdir(stream.0) };
            if ent.is_null() {
                let e = io::Error::last_os_error();
                return if e.raw_os_error() == Some(0) { Ok(()) } else { Err(e) };
            }
            let name = unsafe { CStr::from_ptr((*ent).d_name.as_ptr()) };
            let bytes = name.to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            let d_type = unsafe { (*ent).d_type };
            let mut st = MaybeUninit::<libc::stat>::uninit();
            let rc = unsafe { libc::fstatat(fd, name.as_ptr(), st.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) };
            let (kind, meta) = if rc == 0 {
                let st = unsafe { st.assume_init() };
                let meta = Meta {
                    dev: st.st_dev as u64,
                    ino: st.st_ino as u64,
                    nlink: st.st_nlink as u64,
                    alloc: st.st_blocks as u64 * 512,
                    apparent: st.st_size as u64,
                    mount_point: false,
                };
                (kind_from_mode(st.st_mode), Ok(meta))
            } else {
                (kind_from_dtype(d_type), Err(io::Error::last_os_error()))
            };
            if !f(Entry { name: OsStr::from_bytes(bytes), kind, meta }) {
                return Ok(());
            }
        }
    }
}

/// macOS `getattrlistbulk(2)`: one syscall returns names + attributes for dozens of entries, so
/// there is no per-entry stat at all. Caveats (from the man page, all handled or harmless here):
/// * mount points report the attributes of the *covered* directory (wrong `dev`), so we request
///   `ATTR_DIR_MOUNTSTATUS` to detect them;
/// * firmlinks (`SF_FIRMLINK`) report the attributes of the firmlink, not its target - fine,
///   because we only need to know it is a directory (opening it by path follows the firmlink);
/// * `ATTR_FILE_ALLOCSIZE` is the total allocation of all forks; `st_blocks*512` is the same
///   number on APFS (verified against `du` in the benchmark).
#[cfg(target_os = "macos")]
mod macos_bulk {
    use super::{Entry, Kind, Meta};
    use std::cell::RefCell;
    use std::ffi::OsStr;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    // enum vtype from <sys/vnode.h>
    const VREG: u32 = 1;
    const VDIR: u32 = 2;
    const VLNK: u32 = 5;
    // not exported by the libc crate (0.2.191); value from <sys/attr.h>
    const ATTR_CMN_ERROR: u32 = 0x2000_0000;

    /// Buffer per worker thread (u64 words => 8-byte aligned as required). Apple's fts uses 32 KiB,
    /// dua-core 64 KiB. Measured: 8-256 KiB makes no real difference on cached trees and 32 KiB is
    /// ~5% better than 256 KiB on a 4M-entry tree (scan-engine.md section 4).
    /// Override for experiments: SCAN_BULK_BUF_KIB=256.
    fn buf_words() -> usize {
        let kib = std::env::var("SCAN_BULK_BUF_KIB").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(32);
        kib.max(4) * 1024 / 8
    }

    thread_local! {
        static BUF: RefCell<Vec<u64>> = RefCell::new(vec![0u64; buf_words()]);
    }

    struct Fd(libc::c_int);
    impl Drop for Fd {
        fn drop(&mut self) {
            unsafe { libc::close(self.0) };
        }
    }

    fn u32_at(b: &[u8], off: usize) -> u32 {
        u32::from_ne_bytes(b[off..off + 4].try_into().unwrap())
    }
    fn u64_at(b: &[u8], off: usize) -> u64 {
        u64::from_ne_bytes(b[off..off + 8].try_into().unwrap())
    }

    /// Reads consecutive 4-byte-aligned attribute fields of one record.
    struct Cursor<'a> {
        b: &'a [u8],
        p: usize,
    }
    impl Cursor<'_> {
        fn u32(&mut self) -> u32 {
            self.p += 4;
            u32_at(self.b, self.p - 4)
        }
        fn u32_if(&mut self, present: bool) -> Option<u32> {
            present.then(|| self.u32())
        }
        fn u64_if(&mut self, present: bool) -> Option<u64> {
            present.then(|| {
                self.p += 8;
                u64_at(self.b, self.p - 8)
            })
        }
    }

    pub fn list(dir: &Path, f: &mut dyn FnMut(Entry<'_>) -> bool) -> io::Result<()> {
        let fd = Fd(super::unix_at::open_dir_fd(dir)?);
        // Layout of one record: u32 length, attribute_set_t `returned`, then - only for the bits
        // set in `returned` - u32 error, the common attrs in ascending bit order, the dir attrs,
        // the file attrs. Which groups are present differs per entry: dir attrs are omitted for
        // files and file attrs for directories (even with FSOPT_PACK_INVAL_ATTRS, measured), so
        // the parser must follow `returned` field by field.
        let mut req = libc::attrlist {
            bitmapcount: libc::ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: libc::ATTR_CMN_RETURNED_ATTRS
                | libc::ATTR_CMN_NAME
                | ATTR_CMN_ERROR
                | libc::ATTR_CMN_DEVID
                | libc::ATTR_CMN_OBJTYPE
                | libc::ATTR_CMN_FLAGS
                | libc::ATTR_CMN_FILEID,
            volattr: 0,
            dirattr: libc::ATTR_DIR_MOUNTSTATUS,
            fileattr: libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_TOTALSIZE | libc::ATTR_FILE_ALLOCSIZE,
            forkattr: 0,
        };
        BUF.with(|buf| {
            let mut buf = buf.borrow_mut();
            let len_bytes = buf.len() * 8;
            loop {
                let n = unsafe {
                    libc::getattrlistbulk(fd.0, (&raw mut req).cast(), buf.as_mut_ptr().cast(), len_bytes, 0)
                };
                if n < 0 {
                    return Err(io::Error::last_os_error());
                }
                if n == 0 {
                    return Ok(());
                }
                let bytes: &[u8] = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast(), len_bytes) };
                let mut rec_off = 0usize;
                for _ in 0..n {
                    let rec = &bytes[rec_off..];
                    rec_off += u32_at(rec, 0) as usize;
                    let (ret_common, ret_dir, ret_file) = (u32_at(rec, 4), u32_at(rec, 4 + 8), u32_at(rec, 4 + 12));
                    let mut r = Cursor { b: rec, p: 24 }; // after length + `returned`
                    let error = r.u32_if(ret_common & ATTR_CMN_ERROR != 0).unwrap_or(0);
                    // ATTR_CMN_NAME is mandatory: attrreference_t { i32 offset from itself, u32 len incl. NUL }
                    let name_ref = r.p;
                    let (name_off, name_len) = (r.u32() as i32, r.u32() as usize);
                    let start = (name_ref as isize + name_off as isize) as usize;
                    let name = OsStr::from_bytes(&rec[start..start + name_len.saturating_sub(1)]);
                    // dev_t is i32 on macOS; sign-extend exactly like std (MetadataExt::dev) does.
                    let dev = r.u32_if(ret_common & libc::ATTR_CMN_DEVID != 0).map_or(0, |d| d as i32 as u64);
                    let objtype = r.u32_if(ret_common & libc::ATTR_CMN_OBJTYPE != 0).unwrap_or(0);
                    let _flags = r.u32_if(ret_common & libc::ATTR_CMN_FLAGS != 0); // SF_FIRMLINK lives here
                    let fileid = r.u64_if(ret_common & libc::ATTR_CMN_FILEID != 0).unwrap_or(0);
                    let mntstatus = r.u32_if(ret_dir & libc::ATTR_DIR_MOUNTSTATUS != 0).unwrap_or(0);
                    let linkcount = r.u32_if(ret_file & libc::ATTR_FILE_LINKCOUNT != 0).unwrap_or(1);
                    let totalsize = r.u64_if(ret_file & libc::ATTR_FILE_TOTALSIZE != 0).unwrap_or(0);
                    let allocsize = r.u64_if(ret_file & libc::ATTR_FILE_ALLOCSIZE != 0).unwrap_or(0);

                    let kind = match objtype {
                        VDIR => Kind::Dir,
                        VREG => Kind::File,
                        VLNK => Kind::Symlink,
                        _ => Kind::Other,
                    };
                    let meta = if error != 0 {
                        Err(io::Error::from_raw_os_error(error as i32))
                    } else {
                        Ok(Meta {
                            dev,
                            ino: fileid,
                            nlink: linkcount as u64,
                            alloc: allocsize,
                            apparent: totalsize,
                            mount_point: kind == Kind::Dir && mntstatus & libc::DIR_MNTSTATUS_MNTPOINT != 0,
                        })
                    };
                    if !f(Entry { name, kind, meta }) {
                        return Ok(());
                    }
                }
            }
        })
    }
}
