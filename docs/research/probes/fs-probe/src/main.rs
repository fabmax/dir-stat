//! fs-probe: read-only probes of filesystem semantics relevant to a disk-usage analyzer.
//! macOS only. Never deletes or modifies anything outside the scratch dir passed explicitly.
//!
//! Subcommands:
//!   mounts                      getmntinfo(MNT_NOWAIT) snapshot with decoded flags
//!   statfs <path>...            statfs() of each path
//!   attrs <path>...             lstat + getattrlist (FSOPT_NOFOLLOW, extended common attrs)
//!   volcaps <mountpoint>...     volume capabilities / supported attributes
//!   bulk <dir>                  getattrlistbulk listing of one directory (non-recursive)
//!   iopol                       show / set per-process VFS I/O policies (affects this process only)
//!   opendir <path>...           try opendir+readdir, report errno
//!   walk <dir>                  std::fs based walk (lstat semantics) with du-like accounting
//!   bwalk <dir>                 getattrlistbulk + openat based walk with clone/private-size accounting
//!   names <scratch-dir>         non-UTF-8 / case / normalization behaviour (creates files in scratch)
//!   longpath <scratch-dir>      PATH_MAX behaviour (creates nested dirs in scratch)

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("fs-probe only supports macOS");
}

#[cfg(target_os = "macos")]
fn main() {
    mac::main();
}

#[cfg(target_os = "macos")]
mod mac {
    use std::collections::{HashMap, HashSet};
    use std::ffi::{CString, OsString};
    use std::fs;
    use std::io;
    use std::os::macos::fs::MetadataExt as MacMetaExt;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    // ---- constants missing from libc 0.2.191 -------------------------------------------
    const ATTR_CMN_ERROR: u32 = 0x2000_0000;
    const ATTR_CMNEXT_CLONE_REFCNT: u32 = 0x0000_1000;
    const EF_MAY_SHARE_BLOCKS: u64 = 0x01;
    const EF_NO_XATTRS: u64 = 0x02;
    const EF_IS_SYNC_ROOT: u64 = 0x04;
    const EF_IS_PURGEABLE: u64 = 0x08;
    const EF_IS_SPARSE: u64 = 0x10;
    const EF_IS_SYNTHETIC: u64 = 0x20;
    const EF_SHARES_ALL_BLOCKS: u64 = 0x40;
    const SF_RESTRICTED: u32 = 0x0008_0000;
    const SF_NOUNLINK: u32 = 0x0010_0000;
    const SF_FIRMLINK: u32 = 0x0080_0000;
    const SF_DATALESS: u32 = 0x4000_0000;
    const UF_TRACKED: u32 = 0x0000_0040;
    const UF_DATAVAULT: u32 = 0x0000_0080;
    const MNT_REMOVABLE: u32 = 0x0000_0200;
    const MNT_EXT_ROOT_DATA_VOL: u32 = 0x1;
    const MNT_EXT_FSKIT: u32 = 0x2;
    const VOL_CAP_FMT_VOL_GROUPS: u32 = 0x0100_0000;
    const VOL_CAP_FMT_SEALED: u32 = 0x0200_0000;
    const VOL_CAP_FMT_CLONE_MAPPING: u32 = 0x0400_0000;
    const VOL_CAP_FMT_SHARED_SPACE: u32 = 0x0080_0000;
    const VOL_CAP_FMT_DECMPFS_COMPRESSION: u32 = 0x0001_0000;
    const VOL_CAP_FMT_DIR_HARDLINKS: u32 = 0x0004_0000;
    const VOL_CAP_FMT_SPARSE_FILES: u32 = 0x40;
    const VOL_CAP_FMT_CASE_SENSITIVE: u32 = 0x100;
    const VOL_CAP_FMT_HARDLINKS: u32 = 0x4;
    const VOL_CAP_INT_CLONE: u32 = 0x0001_0000;
    // fsobj_type_t
    const VREG: u32 = 1;
    const VDIR: u32 = 2;
    const VLNK: u32 = 5;
    // iopolicy
    const IOPOL_TYPE_DISK: i32 = 0;
    const IOPOL_TYPE_VFS_ATIME_UPDATES: i32 = 2;
    const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: i32 = 3;
    const IOPOL_TYPE_VFS_STATFS_NO_DATA_VOLUME: i32 = 4;
    const IOPOL_TYPE_VFS_TRIGGER_RESOLVE: i32 = 5;
    const IOPOL_SCOPE_PROCESS: i32 = 0;
    const IOPOL_SCOPE_THREAD: i32 = 1;
    const IOPOL_MATERIALIZE_DATALESS_FILES_OFF: i32 = 1;
    const IOPOL_VFS_TRIGGER_RESOLVE_OFF: i32 = 1;
    const IOPOL_ATIME_UPDATES_OFF: i32 = 1;
    const IOPOL_UTILITY: i32 = 4;

    extern "C" {
        fn getiopolicy_np(iotype: libc::c_int, scope: libc::c_int) -> libc::c_int;
        fn setiopolicy_np(iotype: libc::c_int, scope: libc::c_int, policy: libc::c_int)
            -> libc::c_int;
    }

    pub fn main() {
        let args: Vec<OsString> = std::env::args_os().skip(1).collect();
        if args.is_empty() {
            eprintln!("usage: fs-probe <mounts|statfs|attrs|volcaps|bulk|iopol|opendir|walk|bwalk|names|longpath> ...");
            std::process::exit(2);
        }
        let cmd = args[0].to_string_lossy().to_string();
        let rest: Vec<PathBuf> = args[1..].iter().map(PathBuf::from).collect();
        match cmd.as_str() {
            "mounts" => mounts(),
            "statfs" => rest.iter().for_each(|p| statfs_one(p)),
            "attrs" => rest.iter().for_each(|p| attrs_one(p)),
            "volcaps" => rest.iter().for_each(|p| volcaps(p)),
            "volspace" => rest.iter().for_each(|p| volspace(p)),
            "bulk" => rest.iter().for_each(|p| bulk_list(p)),
            "iopol" => iopol(),
            "opendir" => rest.iter().for_each(|p| try_opendir(p)),
            "walk" => {
                scan_policies();
                walk_std(&rest[0])
            }
            "bwalk" => {
                scan_policies();
                walk_bulk(&rest[0])
            }
            "names" => names(&rest[0]),
            "longpath" => longpath(&rest[0]),
            _ => eprintln!("unknown command {cmd}"),
        }
    }

    /// Process-wide policies a scanner should set before walking (see fs-semantics.md):
    /// never materialize dataless (iCloud/FileProvider) files or directories, never resolve
    /// autofs triggers, never update atime.
    fn scan_policies() {
        unsafe {
            setiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_PROCESS, IOPOL_MATERIALIZE_DATALESS_FILES_OFF);
            setiopolicy_np(IOPOL_TYPE_VFS_TRIGGER_RESOLVE, IOPOL_SCOPE_PROCESS, IOPOL_VFS_TRIGGER_RESOLVE_OFF);
            setiopolicy_np(IOPOL_TYPE_VFS_ATIME_UPDATES, IOPOL_SCOPE_PROCESS, IOPOL_ATIME_UPDATES_OFF);
        }
    }

    fn cpath(p: &Path) -> CString {
        CString::new(p.as_os_str().as_bytes()).expect("NUL in path")
    }

    fn errno() -> i32 {
        io::Error::last_os_error().raw_os_error().unwrap_or(0)
    }

    fn errname(e: i32) -> String {
        let name = match e {
            libc::EPERM => "EPERM",
            libc::ENOENT => "ENOENT",
            libc::EACCES => "EACCES",
            libc::ENOTDIR => "ENOTDIR",
            libc::EDEADLK => "EDEADLK",
            libc::ENAMETOOLONG => "ENAMETOOLONG",
            libc::EILSEQ => "EILSEQ",
            libc::EINVAL => "EINVAL",
            libc::ELOOP => "ELOOP",
            libc::EEXIST => "EEXIST",
            libc::ETIMEDOUT => "ETIMEDOUT",
            libc::EMFILE => "EMFILE",
            libc::ENOTSUP => "ENOTSUP",
            _ => "",
        };
        format!(
            "{e}{}{} ({})",
            if name.is_empty() { "" } else { "=" },
            name,
            io::Error::from_raw_os_error(e)
        )
    }

    fn cstr_arr(a: &[libc::c_char]) -> String {
        let bytes: Vec<u8> = a.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn decode_mnt_flags(f: u32) -> String {
        let table: &[(u32, &str)] = &[
            (libc::MNT_RDONLY as u32, "rdonly"),
            (libc::MNT_NOEXEC as u32, "noexec"),
            (libc::MNT_NOSUID as u32, "nosuid"),
            (libc::MNT_NODEV as u32, "nodev"),
            (libc::MNT_UNION as u32, "union"),
            (libc::MNT_CPROTECT as u32, "cprotect"),
            (libc::MNT_EXPORTED as u32, "exported"),
            (MNT_REMOVABLE, "removable"),
            (libc::MNT_QUARANTINE as u32, "quarantine"),
            (libc::MNT_LOCAL as u32, "local"),
            (libc::MNT_QUOTA as u32, "quota"),
            (libc::MNT_ROOTFS as u32, "rootfs"),
            (libc::MNT_DONTBROWSE as u32, "dontbrowse"),
            (libc::MNT_IGNORE_OWNERSHIP as u32, "ignore_ownership"),
            (libc::MNT_AUTOMOUNTED as u32, "automounted"),
            (libc::MNT_JOURNALED as u32, "journaled"),
            (libc::MNT_NOUSERXATTR as u32, "nouserxattr"),
            (libc::MNT_DEFWRITE as u32, "defwrite"),
            (libc::MNT_MULTILABEL as u32, "multilabel"),
            (libc::MNT_NOATIME as u32, "noatime"),
            (libc::MNT_SNAPSHOT as u32, "snapshot"),
            (0x8000_0000u32, "strictatime"),
        ];
        table
            .iter()
            .filter(|(b, _)| f & b != 0)
            .map(|(_, n)| *n)
            .collect::<Vec<_>>()
            .join(",")
    }

    fn decode_st_flags(f: u32) -> String {
        let table: &[(u32, &str)] = &[
            (libc::UF_NODUMP, "UF_NODUMP"),
            (libc::UF_IMMUTABLE, "UF_IMMUTABLE"),
            (libc::UF_APPEND, "UF_APPEND"),
            (libc::UF_OPAQUE, "UF_OPAQUE"),
            (libc::UF_COMPRESSED, "UF_COMPRESSED"),
            (UF_TRACKED, "UF_TRACKED"),
            (UF_DATAVAULT, "UF_DATAVAULT"),
            (libc::UF_HIDDEN, "UF_HIDDEN"),
            (libc::SF_ARCHIVED, "SF_ARCHIVED"),
            (libc::SF_IMMUTABLE, "SF_IMMUTABLE"),
            (libc::SF_APPEND, "SF_APPEND"),
            (SF_RESTRICTED, "SF_RESTRICTED"),
            (SF_NOUNLINK, "SF_NOUNLINK"),
            (SF_FIRMLINK, "SF_FIRMLINK"),
            (SF_DATALESS, "SF_DATALESS"),
        ];
        let v: Vec<&str> = table.iter().filter(|(b, _)| f & b != 0).map(|(_, n)| *n).collect();
        if v.is_empty() {
            "-".into()
        } else {
            v.join("|")
        }
    }

    fn decode_ef(f: u64) -> String {
        let table: &[(u64, &str)] = &[
            (EF_MAY_SHARE_BLOCKS, "MAY_SHARE_BLOCKS"),
            (EF_NO_XATTRS, "NO_XATTRS"),
            (EF_IS_SYNC_ROOT, "IS_SYNC_ROOT"),
            (EF_IS_PURGEABLE, "IS_PURGEABLE"),
            (EF_IS_SPARSE, "IS_SPARSE"),
            (EF_IS_SYNTHETIC, "IS_SYNTHETIC"),
            (EF_SHARES_ALL_BLOCKS, "SHARES_ALL_BLOCKS"),
        ];
        let v: Vec<&str> = table.iter().filter(|(b, _)| f & b != 0).map(|(_, n)| *n).collect();
        if v.is_empty() {
            "-".into()
        } else {
            v.join("|")
        }
    }

    // ---- mounts --------------------------------------------------------------------------
    pub struct Mount {
        pub on: PathBuf,
    }

    pub fn mount_snapshot() -> Vec<Mount> {
        let mut buf: *mut libc::statfs = std::ptr::null_mut();
        // MNT_NOWAIT: use cached statistics, never block on unresponsive (network) filesystems.
        let n = unsafe { libc::getmntinfo(&mut buf, libc::MNT_NOWAIT) };
        let mut out = Vec::new();
        if n <= 0 {
            return out;
        }
        let s = unsafe { std::slice::from_raw_parts(buf, n as usize) };
        for m in s {
            out.push(Mount { on: PathBuf::from(cstr_arr(&m.f_mntonname)) });
        }
        out
    }

    fn mounts() {
        let mut buf: *mut libc::statfs = std::ptr::null_mut();
        let n = unsafe { libc::getmntinfo(&mut buf, libc::MNT_NOWAIT) };
        println!("getmntinfo(MNT_NOWAIT) -> {n} mounts");
        let s = unsafe { std::slice::from_raw_parts(buf, n.max(0) as usize) };
        for m in s {
            let fsid: [i32; 2] = unsafe { std::mem::transmute(m.f_fsid) };
            println!(
                "{:<36} {:<6} sub={} from={:<16} fsid=({:#x},{:#x}) bsize={} blocks={} bfree={} bavail={} files={} ffree={} flags=[{}] ext=[{}{}]",
                cstr_arr(&m.f_mntonname),
                cstr_arr(&m.f_fstypename),
                m.f_fssubtype,
                cstr_arr(&m.f_mntfromname),
                fsid[0],
                fsid[1],
                m.f_bsize,
                m.f_blocks,
                m.f_bfree,
                m.f_bavail,
                m.f_files,
                m.f_ffree,
                decode_mnt_flags(m.f_flags),
                if m.f_flags_ext & MNT_EXT_ROOT_DATA_VOL != 0 { "ROOT_DATA_VOL " } else { "" },
                if m.f_flags_ext & MNT_EXT_FSKIT != 0 { "FSKIT" } else { "" },
            );
        }
    }

    fn statfs_one(p: &Path) {
        let mut s: libc::statfs = unsafe { std::mem::zeroed() };
        let r = unsafe { libc::statfs(cpath(p).as_ptr(), &mut s) };
        if r != 0 {
            println!("statfs {:?}: {}", p, errname(errno()));
            return;
        }
        let fsid: [i32; 2] = unsafe { std::mem::transmute(s.f_fsid) };
        let bs = s.f_bsize as u64;
        println!(
            "statfs {:<34} -> on={} type={} from={} fsid=({:#x},{:#x}) total={:.2}GB used(blocks-bfree)={:.2}GB bfree={:.2}GB bavail={:.2}GB inodes_used(files-ffree)={} flags=[{}] ext={:#x}",
            p.display(),
            cstr_arr(&s.f_mntonname),
            cstr_arr(&s.f_fstypename),
            cstr_arr(&s.f_mntfromname),
            fsid[0],
            fsid[1],
            (s.f_blocks * bs) as f64 / 1e9,
            ((s.f_blocks - s.f_bfree) * bs) as f64 / 1e9,
            (s.f_bfree * bs) as f64 / 1e9,
            (s.f_bavail * bs) as f64 / 1e9,
            s.f_files.saturating_sub(s.f_ffree),
            decode_mnt_flags(s.f_flags),
            s.f_flags_ext
        );
    }

    // ---- attribute buffer reader ---------------------------------------------------------
    struct Rd<'a> {
        b: &'a [u8],
        pos: usize,
    }
    impl<'a> Rd<'a> {
        fn u32(&mut self) -> u32 {
            let v = u32::from_ne_bytes(self.b[self.pos..self.pos + 4].try_into().unwrap());
            self.pos += 4;
            v
        }
        fn i32(&mut self) -> i32 {
            self.u32() as i32
        }
        fn u64(&mut self) -> u64 {
            let v = u64::from_ne_bytes(self.b[self.pos..self.pos + 8].try_into().unwrap());
            self.pos += 8;
            v
        }
        fn i64(&mut self) -> i64 {
            self.u64() as i64
        }
        fn attrset(&mut self) -> [u32; 5] {
            [self.u32(), self.u32(), self.u32(), self.u32(), self.u32()]
        }
        /// returns the referenced bytes (without trailing NUL)
        fn attrref(&mut self) -> &'a [u8] {
            let start = self.pos;
            let off = self.i32();
            let len = self.u32() as usize;
            let s = (start as isize + off as isize) as usize;
            let e = (s + len).min(self.b.len());
            let sl = &self.b[s.min(e)..e];
            match sl.iter().position(|&c| c == 0) {
                Some(i) => &sl[..i],
                None => sl,
            }
        }
    }

    #[derive(Default, Debug, Clone)]
    pub struct Attrs {
        pub name: Vec<u8>,
        pub error: u32,
        pub devid: i32,
        pub fsid: (i32, i32),
        pub objtype: u32,
        pub flags: u32,
        pub fileid: u64,
        pub dir_linkcount: u32,
        pub dir_entrycount: u32,
        pub dir_mountstatus: u32,
        pub dir_allocsize: i64,
        pub dir_datalength: i64,
        pub file_linkcount: u32,
        pub file_totalsize: i64,
        pub file_allocsize: i64,
        pub file_datalength: i64,
        pub file_dataallocsize: i64,
        pub file_rsrclength: i64,
        pub file_rsrcallocsize: i64,
        pub privatesize: Option<i64>,
        pub nofirmlinkpath: Option<String>,
        pub realdevid: Option<i32>,
        pub realfsid: Option<(i32, i32)>,
        pub cloneid: Option<u64>,
        pub ext_flags: Option<u64>,
        pub recursive_gencount: Option<u64>,
        pub clone_refcnt: Option<u32>,
        pub returned: [u32; 5],
    }

    struct Req {
        common: u32,
        dir: u32,
        file: u32,
        ext: u32,
    }

    /// Parse one attribute group. Order: length, RETURNED_ATTRS, ERROR, then common attrs in
    /// bit order, dir attrs, file attrs, extended-common attrs (forkattr slot). Only attributes
    /// flagged in RETURNED_ATTRS are present.
    fn parse_attrs(b: &[u8]) -> Attrs {
        let mut r = Rd { b, pos: 4 };
        let mut a = Attrs::default();
        let ret = r.attrset();
        a.returned = ret;
        let (c, d, f, x) = (ret[0], ret[2], ret[3], ret[4]);
        if c & ATTR_CMN_ERROR != 0 {
            a.error = r.u32();
        }
        if c & libc::ATTR_CMN_NAME != 0 {
            a.name = r.attrref().to_vec();
        }
        if c & libc::ATTR_CMN_DEVID != 0 {
            a.devid = r.i32();
        }
        if c & libc::ATTR_CMN_FSID != 0 {
            a.fsid = (r.i32(), r.i32());
        }
        if c & libc::ATTR_CMN_OBJTYPE != 0 {
            a.objtype = r.u32();
        }
        if c & libc::ATTR_CMN_FLAGS != 0 {
            a.flags = r.u32();
        }
        if c & libc::ATTR_CMN_FILEID != 0 {
            a.fileid = r.u64();
        }
        if d & libc::ATTR_DIR_LINKCOUNT != 0 {
            a.dir_linkcount = r.u32();
        }
        if d & libc::ATTR_DIR_ENTRYCOUNT != 0 {
            a.dir_entrycount = r.u32();
        }
        if d & libc::ATTR_DIR_MOUNTSTATUS != 0 {
            a.dir_mountstatus = r.u32();
        }
        if d & libc::ATTR_DIR_ALLOCSIZE != 0 {
            a.dir_allocsize = r.i64();
        }
        if d & libc::ATTR_DIR_DATALENGTH != 0 {
            a.dir_datalength = r.i64();
        }
        if f & libc::ATTR_FILE_LINKCOUNT != 0 {
            a.file_linkcount = r.u32();
        }
        if f & libc::ATTR_FILE_TOTALSIZE != 0 {
            a.file_totalsize = r.i64();
        }
        if f & libc::ATTR_FILE_ALLOCSIZE != 0 {
            a.file_allocsize = r.i64();
        }
        if f & libc::ATTR_FILE_DATALENGTH != 0 {
            a.file_datalength = r.i64();
        }
        if f & libc::ATTR_FILE_DATAALLOCSIZE != 0 {
            a.file_dataallocsize = r.i64();
        }
        if f & libc::ATTR_FILE_RSRCLENGTH != 0 {
            a.file_rsrclength = r.i64();
        }
        if f & libc::ATTR_FILE_RSRCALLOCSIZE != 0 {
            a.file_rsrcallocsize = r.i64();
        }
        if x & libc::ATTR_CMNEXT_PRIVATESIZE != 0 {
            a.privatesize = Some(r.i64());
        }
        if x & libc::ATTR_CMNEXT_NOFIRMLINKPATH != 0 {
            a.nofirmlinkpath = Some(String::from_utf8_lossy(r.attrref()).into_owned());
        }
        if x & libc::ATTR_CMNEXT_REALDEVID != 0 {
            a.realdevid = Some(r.i32());
        }
        if x & libc::ATTR_CMNEXT_REALFSID != 0 {
            a.realfsid = Some((r.i32(), r.i32()));
        }
        if x & libc::ATTR_CMNEXT_CLONEID != 0 {
            a.cloneid = Some(r.u64());
        }
        if x & libc::ATTR_CMNEXT_EXT_FLAGS != 0 {
            a.ext_flags = Some(r.u64());
        }
        if x & libc::ATTR_CMNEXT_RECURSIVE_GENCOUNT != 0 {
            a.recursive_gencount = Some(r.u64());
        }
        if x & ATTR_CMNEXT_CLONE_REFCNT != 0 {
            a.clone_refcnt = Some(r.u32());
        }
        a
    }

    fn make_attrlist(req: &Req) -> libc::attrlist {
        libc::attrlist {
            bitmapcount: libc::ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: req.common | libc::ATTR_CMN_RETURNED_ATTRS,
            volattr: 0,
            dirattr: req.dir,
            fileattr: req.file,
            forkattr: req.ext,
        }
    }

    fn getattrs(p: &Path, req: &Req, nofollow: bool) -> Result<Attrs, i32> {
        let mut al = make_attrlist(req);
        let mut buf = vec![0u8; 8192];
        let mut opts = libc::FSOPT_ATTR_CMN_EXTENDED;
        if nofollow {
            opts |= libc::FSOPT_NOFOLLOW;
        }
        let r = unsafe {
            libc::getattrlist(
                cpath(p).as_ptr(),
                &mut al as *mut _ as *mut libc::c_void,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                opts,
            )
        };
        if r != 0 {
            return Err(errno());
        }
        Ok(parse_attrs(&buf))
    }

    fn full_req() -> Req {
        Req {
            common: libc::ATTR_CMN_DEVID
                | libc::ATTR_CMN_FSID
                | libc::ATTR_CMN_OBJTYPE
                | libc::ATTR_CMN_FLAGS
                | libc::ATTR_CMN_FILEID,
            dir: libc::ATTR_DIR_LINKCOUNT
                | libc::ATTR_DIR_ENTRYCOUNT
                | libc::ATTR_DIR_MOUNTSTATUS
                | libc::ATTR_DIR_ALLOCSIZE
                | libc::ATTR_DIR_DATALENGTH,
            file: libc::ATTR_FILE_LINKCOUNT
                | libc::ATTR_FILE_TOTALSIZE
                | libc::ATTR_FILE_ALLOCSIZE
                | libc::ATTR_FILE_DATALENGTH
                | libc::ATTR_FILE_DATAALLOCSIZE
                | libc::ATTR_FILE_RSRCLENGTH
                | libc::ATTR_FILE_RSRCALLOCSIZE,
            ext: libc::ATTR_CMNEXT_PRIVATESIZE
                | libc::ATTR_CMNEXT_NOFIRMLINKPATH
                | libc::ATTR_CMNEXT_REALDEVID
                | libc::ATTR_CMNEXT_REALFSID
                | libc::ATTR_CMNEXT_CLONEID
                | libc::ATTR_CMNEXT_EXT_FLAGS
                | libc::ATTR_CMNEXT_RECURSIVE_GENCOUNT
                | ATTR_CMNEXT_CLONE_REFCNT,
        }
    }

    fn attrs_one(p: &Path) {
        match fs::symlink_metadata(p) {
            Ok(m) => println!(
                "lstat  {}: dev={} ino={} nlink={} size={} blocks={} (alloc={}B) blksize={} type={} st_flags={}",
                p.display(),
                m.dev(),
                m.ino(),
                m.nlink(),
                m.size(),
                m.blocks(),
                m.blocks() * 512,
                m.blksize(),
                ftype(&m),
                decode_st_flags(m.st_flags())
            ),
            Err(e) => println!("lstat  {}: {}", p.display(), e),
        }
        match getattrs(p, &full_req(), true) {
            Ok(a) => println!(
                "  gattr: objtype={} devid={} fsid=({:#x},{:#x}) fileid={} flags={} dir[link={} entries={} mnt={} alloc={} len={}] file[link={} total={} alloc={} datalen={} dataalloc={} rsrclen={} rsrcalloc={}] ext[private={:?} nofirmlinkpath={:?} realdev={:?} realfsid={:?} cloneid={:?} ext_flags={} gencount={:?} clone_refcnt={:?}]",
                a.objtype,
                a.devid,
                a.fsid.0,
                a.fsid.1,
                a.fileid,
                decode_st_flags(a.flags),
                a.dir_linkcount,
                a.dir_entrycount,
                a.dir_mountstatus,
                a.dir_allocsize,
                a.dir_datalength,
                a.file_linkcount,
                a.file_totalsize,
                a.file_allocsize,
                a.file_datalength,
                a.file_dataallocsize,
                a.file_rsrclength,
                a.file_rsrcallocsize,
                a.privatesize,
                a.nofirmlinkpath,
                a.realdevid,
                a.realfsid,
                a.cloneid,
                a.ext_flags.map(decode_ef).unwrap_or_default(),
                a.recursive_gencount,
                a.clone_refcnt,
            ),
            Err(e) => println!("  gattr: {}", errname(e)),
        }
    }

    fn ftype(m: &fs::Metadata) -> &'static str {
        let t = m.file_type();
        if t.is_symlink() {
            "symlink"
        } else if t.is_dir() {
            "dir"
        } else if t.is_file() {
            "file"
        } else {
            "other"
        }
    }

    // ---- volume capabilities -------------------------------------------------------------
    fn volcaps(p: &Path) {
        let mut al = libc::attrlist {
            bitmapcount: libc::ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: 0,
            volattr: libc::ATTR_VOL_INFO | libc::ATTR_VOL_CAPABILITIES | libc::ATTR_VOL_ATTRIBUTES,
            dirattr: 0,
            fileattr: 0,
            forkattr: 0,
        };
        let mut buf = vec![0u8; 1024];
        let r = unsafe {
            libc::getattrlist(
                cpath(p).as_ptr(),
                &mut al as *mut _ as *mut libc::c_void,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                0,
            )
        };
        if r != 0 {
            println!("volcaps {}: {}", p.display(), errname(errno()));
            return;
        }
        let mut rd = Rd { b: &buf, pos: 4 };
        let caps = [rd.u32(), rd.u32(), rd.u32(), rd.u32()];
        let valid = [rd.u32(), rd.u32(), rd.u32(), rd.u32()];
        let validattr = rd.attrset();
        let nativeattr = rd.attrset();
        let fmt = caps[0] & valid[0];
        let intf = caps[1] & valid[1];
        let names: &[(u32, &str)] = &[
            (VOL_CAP_FMT_HARDLINKS, "HARDLINKS"),
            (VOL_CAP_FMT_SPARSE_FILES, "SPARSE_FILES"),
            (VOL_CAP_FMT_CASE_SENSITIVE, "CASE_SENSITIVE"),
            (VOL_CAP_FMT_DECMPFS_COMPRESSION, "DECMPFS_COMPRESSION"),
            (VOL_CAP_FMT_DIR_HARDLINKS, "DIR_HARDLINKS"),
            (VOL_CAP_FMT_SHARED_SPACE, "SHARED_SPACE"),
            (VOL_CAP_FMT_VOL_GROUPS, "VOL_GROUPS"),
            (VOL_CAP_FMT_SEALED, "SEALED"),
            (VOL_CAP_FMT_CLONE_MAPPING, "CLONE_MAPPING"),
        ];
        let set: Vec<&str> = names.iter().filter(|(b, _)| fmt & b != 0).map(|(_, n)| *n).collect();
        let ext_names: &[(u32, &str)] = &[
            (libc::ATTR_CMNEXT_PRIVATESIZE, "PRIVATESIZE"),
            (libc::ATTR_CMNEXT_LINKID, "LINKID"),
            (libc::ATTR_CMNEXT_NOFIRMLINKPATH, "NOFIRMLINKPATH"),
            (libc::ATTR_CMNEXT_REALDEVID, "REALDEVID"),
            (libc::ATTR_CMNEXT_REALFSID, "REALFSID"),
            (libc::ATTR_CMNEXT_CLONEID, "CLONEID"),
            (libc::ATTR_CMNEXT_EXT_FLAGS, "EXT_FLAGS"),
            (libc::ATTR_CMNEXT_RECURSIVE_GENCOUNT, "RECURSIVE_GENCOUNT"),
            (ATTR_CMNEXT_CLONE_REFCNT, "CLONE_REFCNT"),
        ];
        let ext_valid: Vec<&str> =
            ext_names.iter().filter(|(b, _)| validattr[4] & b != 0).map(|(_, n)| *n).collect();
        println!(
            "volcaps {}: fmt=[{}] clone(int)={} valid_cmnext=[{}] native_cmnext={:#x}",
            p.display(),
            set.join(","),
            intf & VOL_CAP_INT_CLONE != 0,
            ext_valid.join(","),
            nativeattr[4],
        );
    }


    // ---- volume space (what df uses on APFS) ---------------------------------------------
    fn volspace(p: &Path) {
        let mut al = libc::attrlist {
            bitmapcount: libc::ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: 0,
            volattr: libc::ATTR_VOL_INFO
                | libc::ATTR_VOL_SIZE
                | libc::ATTR_VOL_SPACEFREE
                | libc::ATTR_VOL_SPACEAVAIL
                | libc::ATTR_VOL_OBJCOUNT
                | libc::ATTR_VOL_FILECOUNT
                | libc::ATTR_VOL_DIRCOUNT
                | libc::ATTR_VOL_NAME
                | libc::ATTR_VOL_SPACEUSED
                | libc::ATTR_VOL_QUOTA_SIZE
                | libc::ATTR_VOL_RESERVED_SIZE,
            dirattr: 0,
            fileattr: 0,
            forkattr: 0,
        };
        let mut buf = vec![0u8; 4096];
        let r = unsafe {
            libc::getattrlist(
                cpath(p).as_ptr(),
                &mut al as *mut _ as *mut libc::c_void,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                0,
            )
        };
        if r != 0 {
            println!("volspace {}: {}", p.display(), errname(errno()));
            return;
        }
        let mut rd = Rd { b: &buf, pos: 4 };
        // packing order (xnu vfs_attrlist.c): SIZE, SPACEFREE, SPACEAVAIL, SPACEUSED, ...,
        // OBJCOUNT, FILECOUNT, DIRCOUNT, ..., NAME, ..., QUOTA_SIZE, RESERVED_SIZE
        let size = rd.i64();
        let free = rd.i64();
        let avail = rd.i64();
        let used = rd.i64();
        let objs = rd.u32();
        let files = rd.u32();
        let dirs = rd.u32();
        let name = String::from_utf8_lossy(rd.attrref()).into_owned();
        let quota = rd.i64();
        let reserved = rd.i64();
        println!(
            "volspace {:<26} name={:<20} size={:.2}GB free={:.2}GB avail={:.2}GB SPACEUSED={:.2}GB quota={} reserve={} objcount={} filecount={} dircount={}",
            p.display(), name, size as f64 / 1e9, free as f64 / 1e9, avail as f64 / 1e9, used as f64 / 1e9, quota, reserved, objs, files, dirs
        );
    }

    // ---- getattrlistbulk -----------------------------------------------------------------
    fn bulk_req() -> Req {
        // FS_PROBE_EXT=none|flags|all selects which extended attrs are requested (cost test)
        let ext_mode = std::env::var("FS_PROBE_EXT").unwrap_or_else(|_| "all".into());
        let ext = match ext_mode.as_str() {
            "none" => 0,
            "flags" => libc::ATTR_CMNEXT_EXT_FLAGS | libc::ATTR_CMNEXT_CLONEID | ATTR_CMNEXT_CLONE_REFCNT,
            _ => {
                libc::ATTR_CMNEXT_PRIVATESIZE
                    | libc::ATTR_CMNEXT_CLONEID
                    | libc::ATTR_CMNEXT_EXT_FLAGS
                    | ATTR_CMNEXT_CLONE_REFCNT
            }
        };
        Req { ext, ..bulk_req_all() }
    }

    fn bulk_req_all() -> Req {
        Req {
            common: libc::ATTR_CMN_NAME
                | ATTR_CMN_ERROR
                | libc::ATTR_CMN_DEVID
                | libc::ATTR_CMN_OBJTYPE
                | libc::ATTR_CMN_FLAGS
                | libc::ATTR_CMN_FILEID,
            dir: libc::ATTR_DIR_LINKCOUNT | libc::ATTR_DIR_MOUNTSTATUS | libc::ATTR_DIR_ALLOCSIZE,
            file: libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_TOTALSIZE | libc::ATTR_FILE_ALLOCSIZE,
            ext: libc::ATTR_CMNEXT_PRIVATESIZE
                | libc::ATTR_CMNEXT_CLONEID
                | libc::ATTR_CMNEXT_EXT_FLAGS
                | ATTR_CMNEXT_CLONE_REFCNT,
        }
    }

    /// Reads all entries of an open directory fd with getattrlistbulk.
    fn bulk_read(fd: libc::c_int, req: &Req, out: &mut Vec<Attrs>) -> Result<(), i32> {
        thread_local! {
            static BUF: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(vec![0u8; 128 * 1024]);
        }
        BUF.with(|b| bulk_read_buf(fd, req, out, &mut b.borrow_mut()))
    }

    fn bulk_read_buf(fd: libc::c_int, req: &Req, out: &mut Vec<Attrs>, buf: &mut Vec<u8>) -> Result<(), i32> {
        let mut al = make_attrlist(req);
        loop {
            let n = unsafe {
                libc::getattrlistbulk(
                    fd,
                    &mut al as *mut _ as *mut libc::c_void,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                    libc::FSOPT_ATTR_CMN_EXTENDED as u64,
                )
            };
            if n < 0 {
                return Err(errno());
            }
            if n == 0 {
                return Ok(());
            }
            let mut off = 0usize;
            for _ in 0..n {
                let len = u32::from_ne_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
                out.push(parse_attrs(&buf[off..off + len]));
                off += len;
            }
        }
    }

    fn open_dir(p: &Path) -> Result<libc::c_int, i32> {
        let fd = unsafe {
            libc::open(cpath(p).as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC)
        };
        if fd < 0 {
            Err(errno())
        } else {
            Ok(fd)
        }
    }

    fn bulk_list(p: &Path) {
        let fd = match open_dir(p) {
            Ok(fd) => fd,
            Err(e) => {
                println!("bulk {}: open: {}", p.display(), errname(e));
                return;
            }
        };
        let mut v = Vec::new();
        let r = bulk_read(fd, &bulk_req(), &mut v);
        unsafe { libc::close(fd) };
        println!("bulk {}: {} entries, result={:?}", p.display(), v.len(), r.err().map(errname));
        v.sort_by(|a, b| a.name.cmp(&b.name));
        for a in &v {
            let t = match a.objtype {
                VREG => "file",
                VDIR => "dir",
                VLNK => "link",
                _ => "other",
            };
            let interesting = a.flags & (SF_FIRMLINK | SF_DATALESS | libc::UF_COMPRESSED) != 0
                || a.dir_mountstatus != 0
                || a.error != 0;
            if interesting || v.len() <= 40 {
                println!(
                    "  {:<28} {:<5} err={} dev={} fileid={} flags={} mnt={} dlink={} dalloc={} flink={} total={} alloc={} private={:?} cloneid={:?} ef={} refcnt={:?}",
                    String::from_utf8_lossy(&a.name),
                    t,
                    a.error,
                    a.devid,
                    a.fileid,
                    decode_st_flags(a.flags),
                    a.dir_mountstatus,
                    a.dir_linkcount,
                    a.dir_allocsize,
                    a.file_linkcount,
                    a.file_totalsize,
                    a.file_allocsize,
                    a.privatesize,
                    a.cloneid,
                    a.ext_flags.map(decode_ef).unwrap_or_default(),
                    a.clone_refcnt
                );
            }
        }
    }

    // ---- iopolicy ------------------------------------------------------------------------
    fn iopol() {
        let show = |label: &str| {
            let types = [
                ("DISK", IOPOL_TYPE_DISK),
                ("ATIME_UPDATES", IOPOL_TYPE_VFS_ATIME_UPDATES),
                ("MATERIALIZE_DATALESS", IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES),
                ("STATFS_NO_DATA_VOLUME", IOPOL_TYPE_VFS_STATFS_NO_DATA_VOLUME),
                ("TRIGGER_RESOLVE", IOPOL_TYPE_VFS_TRIGGER_RESOLVE),
            ];
            for (n, t) in types {
                let p = unsafe { getiopolicy_np(t, IOPOL_SCOPE_PROCESS) };
                let pe = if p < 0 { errname(errno()) } else { String::new() };
                let th = unsafe { getiopolicy_np(t, IOPOL_SCOPE_THREAD) };
                let te = if th < 0 { errname(errno()) } else { String::new() };
                println!("  [{label}] {n:<22} process={p} {pe} thread={th} {te}");
            }
        };
        show("initial");
        let name = CString::new("vfs.nspace.prevent_materialization").unwrap();
        let read_sysctl = || {
            let mut sp: libc::c_int = -1;
            let mut sz = std::mem::size_of::<libc::c_int>();
            let r = unsafe {
                libc::sysctlbyname(
                    name.as_ptr(),
                    &mut sp as *mut _ as *mut libc::c_void,
                    &mut sz,
                    std::ptr::null_mut(),
                    0,
                )
            };
            println!("  sysctl vfs.nspace.prevent_materialization (read) r={r} value={sp}");
        };
        read_sysctl();
        for (n, t, v, scope) in [
            ("MATERIALIZE_DATALESS=OFF (thread)", IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_MATERIALIZE_DATALESS_FILES_OFF, IOPOL_SCOPE_THREAD),
            ("MATERIALIZE_DATALESS=OFF", IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_MATERIALIZE_DATALESS_FILES_OFF, IOPOL_SCOPE_PROCESS),
            ("ATIME_UPDATES=OFF (thread)", IOPOL_TYPE_VFS_ATIME_UPDATES, IOPOL_ATIME_UPDATES_OFF, IOPOL_SCOPE_THREAD),
            ("TRIGGER_RESOLVE=OFF", IOPOL_TYPE_VFS_TRIGGER_RESOLVE, IOPOL_VFS_TRIGGER_RESOLVE_OFF, IOPOL_SCOPE_PROCESS),
            ("ATIME_UPDATES=OFF", IOPOL_TYPE_VFS_ATIME_UPDATES, IOPOL_ATIME_UPDATES_OFF, IOPOL_SCOPE_PROCESS),
            ("DISK=UTILITY (thread)", IOPOL_TYPE_DISK, IOPOL_UTILITY, IOPOL_SCOPE_THREAD),
        ] {
            let r = unsafe { setiopolicy_np(t, scope, v) };
            println!(
                "  setiopolicy_np({n}) -> {r} {}",
                if r < 0 { errname(errno()) } else { String::new() }
            );
        }
        show("after");
        read_sysctl();
    }

    // ---- opendir errno -------------------------------------------------------------------
    fn try_opendir(p: &Path) {
        let d = unsafe { libc::opendir(cpath(p).as_ptr()) };
        if d.is_null() {
            println!("opendir {:<56} -> {}", p.display(), errname(errno()));
        } else {
            unsafe { *libc::__error() = 0 };
            let mut n = 0;
            loop {
                let e = unsafe { libc::readdir(d) };
                if e.is_null() {
                    break;
                }
                n += 1;
            }
            let e = errno();
            unsafe { libc::closedir(d) };
            println!(
                "opendir {:<56} -> ok, {} entries, errno after readdir loop={}",
                p.display(),
                n,
                if e == 0 { "0".into() } else { errname(e) }
            );
        }
        if let Err(e) = fs::read_dir(p) {
            println!("  std::fs::read_dir -> {e} kind={:?}", e.kind());
        }
    }

    // ---- std walker ----------------------------------------------------------------------
    #[derive(Default)]
    struct Totals {
        files: u64,
        dirs: u64,
        symlinks: u64,
        other: u64,
        apparent: u64,
        alloc: u64,
        alloc_nodedupe: u64,
        dir_own_alloc: u64,
        hardlinked_files: u64,
        hardlink_dupes: u64,
        hardlink_dupe_bytes: u64,
        compressed_files: u64,
        compressed_apparent: u64,
        compressed_alloc: u64,
        dataless: u64,
        sparse_like: u64,
        sparse_saved: u64,
        firmlinks: u64,
        skipped_mounts: Vec<PathBuf>,
        skipped_dev: u64,
        errors: HashMap<String, u64>,
        error_examples: Vec<String>,
        max_path: usize,
        max_depth: usize,
        non_utf8: u64,
    }

    fn walk_std(root: &Path) {
        let t0 = Instant::now();
        let mounts = mount_snapshot();
        let root_meta = fs::symlink_metadata(root).expect("root");
        let root_dev = root_meta.dev();
        let mount_points: HashSet<PathBuf> = mounts.iter().map(|m| m.on.clone()).collect();
        let mut seen: HashSet<(u64, u64)> = HashSet::new();
        let mut t = Totals::default();
        let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
        t.dirs += 1;
        t.alloc += root_meta.blocks() * 512;
        t.dir_own_alloc += root_meta.blocks() * 512;
        while let Some((dir, depth)) = stack.pop() {
            t.max_depth = t.max_depth.max(depth);
            let rd = match fs::read_dir(&dir) {
                Ok(rd) => rd,
                Err(e) => {
                    let k = errname(e.raw_os_error().unwrap_or(0));
                    if t.error_examples.len() < 12 {
                        t.error_examples.push(format!("read_dir {}: {}", dir.display(), k));
                    }
                    *t.errors.entry(format!("read_dir {k}")).or_default() += 1;
                    continue;
                }
            };
            for ent in rd {
                let ent = match ent {
                    Ok(e) => e,
                    Err(e) => {
                        *t
                            .errors
                            .entry(format!("readdir-iter {}", errname(e.raw_os_error().unwrap_or(0))))
                            .or_default() += 1;
                        continue;
                    }
                };
                let path = ent.path();
                t.max_path = t.max_path.max(path.as_os_str().len());
                if std::str::from_utf8(ent.file_name().as_bytes()).is_err() {
                    t.non_utf8 += 1;
                }
                // mount-point check BEFORE stat (avoid touching other filesystems)
                if mount_points.contains(&path) {
                    t.skipped_mounts.push(path);
                    continue;
                }
                let m = match ent.metadata() {
                    Ok(m) => m,
                    Err(e) => {
                        let k = errname(e.raw_os_error().unwrap_or(0));
                        *t.errors.entry(format!("lstat {k}")).or_default() += 1;
                        if t.error_examples.len() < 12 {
                            t.error_examples.push(format!("lstat {}: {}", path.display(), k));
                        }
                        continue;
                    }
                };
                let fl = m.st_flags();
                if fl & SF_FIRMLINK != 0 {
                    t.firmlinks += 1;
                }
                if fl & SF_DATALESS != 0 {
                    t.dataless += 1;
                }
                let alloc = m.blocks() * 512;
                let ft = m.file_type();
                if ft.is_dir() {
                    if m.dev() != root_dev {
                        t.skipped_dev += 1;
                        t.skipped_mounts.push(path);
                        continue;
                    }
                    t.dirs += 1;
                    t.alloc += alloc;
                    t.alloc_nodedupe += alloc;
                    t.dir_own_alloc += alloc;
                    t.apparent += m.size();
                    stack.push((path, depth + 1));
                    continue;
                }
                if ft.is_symlink() {
                    t.symlinks += 1;
                } else if ft.is_file() {
                    t.files += 1;
                } else {
                    t.other += 1;
                }
                t.alloc_nodedupe += alloc;
                if m.nlink() > 1 {
                    t.hardlinked_files += 1;
                    if !seen.insert((m.dev(), m.ino())) {
                        t.hardlink_dupes += 1;
                        t.hardlink_dupe_bytes += alloc;
                        continue;
                    }
                }
                t.apparent += m.size();
                t.alloc += alloc;
                if fl & libc::UF_COMPRESSED != 0 {
                    t.compressed_files += 1;
                    t.compressed_apparent += m.size();
                    t.compressed_alloc += alloc;
                } else if ft.is_file() && alloc + 4096 < m.size() && fl & SF_DATALESS == 0 {
                    t.sparse_like += 1;
                    t.sparse_saved += m.size() - alloc;
                }
            }
        }
        let el = t0.elapsed();
        println!("walk_std {} ({:.2?})", root.display(), el);
        println!(
            "  files={} dirs={} symlinks={} other={} entries/s={:.0}",
            t.files,
            t.dirs,
            t.symlinks,
            t.other,
            (t.files + t.dirs + t.symlinks + t.other) as f64 / el.as_secs_f64()
        );
        println!(
            "  apparent={} alloc(dedupe hardlinks)={} alloc(no dedupe)={} dir_own_alloc={}",
            t.apparent, t.alloc, t.alloc_nodedupe, t.dir_own_alloc
        );
        println!("  KiB: alloc={} apparent={}", t.alloc / 1024, t.apparent / 1024);
        println!(
            "  hardlinked_files={} dupes={} dupe_bytes={} seen_set_len={}",
            t.hardlinked_files,
            t.hardlink_dupes,
            t.hardlink_dupe_bytes,
            seen.len()
        );
        println!(
            "  compressed_files={} compressed_apparent={} compressed_alloc={}",
            t.compressed_files, t.compressed_apparent, t.compressed_alloc
        );
        println!(
            "  dataless={} sparse_like_files={} sparse_saved_bytes={} firmlinks(st_flags)={} non_utf8_names={}",
            t.dataless, t.sparse_like, t.sparse_saved, t.firmlinks, t.non_utf8
        );
        println!(
            "  max_path_len={} max_depth={} skipped_dev_dirs={} skipped_mounts={:?}",
            t.max_path,
            t.max_depth,
            t.skipped_dev,
            t.skipped_mounts.iter().take(10).collect::<Vec<_>>()
        );
        let mut errs: Vec<_> = t.errors.iter().collect();
        errs.sort();
        println!("  errors={:?}", errs);
        for e in &t.error_examples {
            println!("    e.g. {e}");
        }
    }

    // ---- bulk walker ---------------------------------------------------------------------
    fn walk_bulk(root: &Path) {
        let req = bulk_req();
        let t1 = Instant::now();
        let mut acc = BulkAcc::default();
        match open_dir(root) {
            Ok(fd) => bwalk_rec(fd, &req, &mut acc, 0),
            Err(e) => println!("open root: {}", errname(e)),
        }
        let el = t1.elapsed();
        let a = &acc;
        let same_cloneid_groups = a.cloneids.values().filter(|&&n| n > 1).count();
        println!("walk_bulk {} ({:.2?})", root.display(), el);
        println!(
            "  files={} dirs={} symlinks={} hardlink_dupes={} entries/s={:.0}",
            a.files,
            a.dirs,
            a.links,
            a.hl_dupes,
            (a.files + a.dirs + a.links) as f64 / el.as_secs_f64()
        );
        println!(
            "  alloc(ATTR_*_ALLOCSIZE, dirs incl.)={} ({} KiB) logical={} private(PRIVATESIZE sum)={} ({} KiB)",
            a.alloc,
            a.alloc / 1024,
            a.total,
            a.private,
            a.private / 1024
        );
        println!(
            "  EF_MAY_SHARE_BLOCKS files={} alloc={} private={} | EF_SHARES_ALL_BLOCKS files={} alloc={} | clone_refcnt>1 files={} | cloneid groups(>1 member) in scan={}",
            a.may_share, a.may_share_alloc, a.may_share_private, a.shares_all, a.shares_all_alloc, a.refcnt_gt1, same_cloneid_groups
        );
        println!(
            "  EF_IS_PURGEABLE files={} alloc={} | EF_IS_SPARSE files={} | UF_COMPRESSED={} SF_DATALESS={} SF_FIRMLINK={}",
            a.purgeable, a.purgeable_alloc, a.sparse, a.compressed, a.dataless, a.firmlinks
        );
        println!("  mountpoints skipped (DIR_MNTSTATUS_MNTPOINT)={:?}", a.mountpoints);
        let mut errs: Vec<_> = a.errors.iter().collect();
        errs.sort();
        println!("  errors={:?}", errs);
    }

    #[derive(Default)]
    struct BulkAcc {
        alloc: u64,
        total: u64,
        private: u64,
        files: u64,
        dirs: u64,
        links: u64,
        hl_dupes: u64,
        may_share: u64,
        may_share_alloc: u64,
        may_share_private: u64,
        shares_all: u64,
        shares_all_alloc: u64,
        purgeable: u64,
        purgeable_alloc: u64,
        sparse: u64,
        refcnt_gt1: u64,
        compressed: u64,
        dataless: u64,
        firmlinks: u64,
        cloneids: HashMap<u64, u32>,
        mountpoints: Vec<String>,
        errors: HashMap<String, u64>,
        seen: HashSet<u64>,
    }

    /// Depth-first, openat()-relative walk: no full paths are ever built, so PATH_MAX does
    /// not apply. Entries are buffered before recursing; the parent fd stays open while its
    /// children are visited (one fd per depth level).
    fn bwalk_rec(fd: libc::c_int, req: &Req, acc: &mut BulkAcc, depth: usize) {
        acc.dirs += 1;
        let mut v = Vec::new();
        if let Err(e) = bulk_read(fd, req, &mut v) {
            *acc.errors.entry(format!("bulk {}", errname(e))).or_default() += 1;
        }
        for a in v {
            if a.error != 0 {
                *acc.errors.entry(format!("entry {}", errname(a.error as i32))).or_default() += 1;
                continue;
            }
            if a.flags & libc::UF_COMPRESSED != 0 {
                acc.compressed += 1;
            }
            if a.flags & SF_DATALESS != 0 {
                acc.dataless += 1;
            }
            if a.flags & SF_FIRMLINK != 0 {
                acc.firmlinks += 1;
            }
            if a.objtype == VDIR {
                acc.alloc += a.dir_allocsize.max(0) as u64;
                if a.dir_mountstatus & libc::DIR_MNTSTATUS_MNTPOINT != 0 {
                    acc.mountpoints.push(String::from_utf8_lossy(&a.name).into_owned());
                    continue;
                }
                let name = CString::new(a.name.clone()).unwrap();
                let cfd = unsafe {
                    libc::openat(
                        fd,
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                if cfd < 0 {
                    *acc.errors.entry(format!("openat {}", errname(errno()))).or_default() += 1;
                    continue;
                }
                bwalk_rec(cfd, req, acc, depth + 1);
                continue;
            }
            if a.objtype == VLNK {
                acc.links += 1;
            } else {
                acc.files += 1;
            }
            if a.file_linkcount > 1 && !acc.seen.insert(a.fileid) {
                acc.hl_dupes += 1;
                continue;
            }
            let al = a.file_allocsize.max(0) as u64;
            acc.alloc += al;
            acc.total += a.file_totalsize.max(0) as u64;
            let pv = a.privatesize.unwrap_or(0).max(0) as u64;
            acc.private += pv;
            let ef = a.ext_flags.unwrap_or(0);
            if ef & EF_MAY_SHARE_BLOCKS != 0 {
                acc.may_share += 1;
                acc.may_share_alloc += al;
                acc.may_share_private += pv;
                if let Some(c) = a.cloneid {
                    *acc.cloneids.entry(c).or_default() += 1;
                }
            }
            if ef & EF_SHARES_ALL_BLOCKS != 0 {
                acc.shares_all += 1;
                acc.shares_all_alloc += al;
            }
            if ef & EF_IS_PURGEABLE != 0 {
                acc.purgeable += 1;
                acc.purgeable_alloc += al;
            }
            if ef & EF_IS_SPARSE != 0 {
                acc.sparse += 1;
            }
            if a.clone_refcnt.unwrap_or(0) > 1 {
                acc.refcnt_gt1 += 1;
            }
        }
        unsafe { libc::close(fd) };
    }

    // ---- names ---------------------------------------------------------------------------
    fn names(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        // 1. invalid UTF-8
        let bad = OsString::from_vec(vec![b'b', b'a', b'd', 0xff, 0xfe, b'.', b't', b'x', b't']);
        let p = dir.join(&bad);
        match fs::write(&p, b"x") {
            Ok(_) => println!("names: created non-UTF-8 name OK: {:?}", p),
            Err(e) => println!(
                "names: create non-UTF-8 name -> {} ({:?})",
                e,
                e.raw_os_error().map(errname)
            ),
        }
        // 2. case-insensitivity
        let a = dir.join("CaseTest.txt");
        fs::write(&a, b"upper").unwrap();
        let b = dir.join("casetest.TXT");
        match fs::symlink_metadata(&b) {
            Ok(m) => println!(
                "names: 'casetest.TXT' resolves to existing 'CaseTest.txt' ino={} (case-insensitive)",
                m.ino()
            ),
            Err(e) => println!("names: 'casetest.TXT' -> {e} (case-sensitive)"),
        }
        // 3. Unicode normalization: NFC "é" vs NFD "e\u{301}"
        let nfc = dir.join("caf\u{e9}.txt");
        let nfd = dir.join("cafe\u{301}.txt");
        fs::write(&nfd, b"nfd").unwrap();
        match fs::symlink_metadata(&nfc) {
            Ok(m) => println!(
                "names: NFC lookup finds NFD-created file ino={} (normalization-insensitive)",
                m.ino()
            ),
            Err(e) => println!("names: NFC lookup of NFD file -> {e} (normalization-sensitive)"),
        }
        let listed: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let n = e.unwrap().file_name();
                format!("{:?} bytes={:x?}", n, n.as_bytes())
            })
            .filter(|s| s.contains("caf"))
            .collect();
        println!("names: readdir returns {:?}", listed);
        // 4. NAME_MAX
        let long = "a".repeat(255);
        match fs::write(dir.join(&long), b"x") {
            Ok(_) => println!("names: 255-byte name OK"),
            Err(e) => println!("names: 255-byte name -> {e}"),
        }
        let long2 = "a".repeat(256);
        match fs::write(dir.join(&long2), b"x") {
            Ok(_) => println!("names: 256-byte name OK"),
            Err(e) => println!("names: 256-byte name -> {e}"),
        }
        let umlaut = "\u{e4}".repeat(255); // 255 chars, 510 UTF-8 bytes
        match fs::write(dir.join(&umlaut), b"x") {
            Ok(_) => println!("names: 255 x 'ä' (510 bytes UTF-8) OK -> NAME_MAX counts UTF-16/chars, not bytes"),
            Err(e) => println!("names: 255 x 'ä' (510 bytes) -> {e}"),
        }
    }

    // ---- long paths ----------------------------------------------------------------------
    fn longpath(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        let comp = "d".repeat(200);
        // build with *at() calls so that each syscall only sees a short relative path
        let mut fd = open_dir(dir).unwrap();
        let mut full = dir.to_path_buf();
        for _ in 0..8 {
            let c = CString::new(comp.clone()).unwrap();
            unsafe { libc::mkdirat(fd, c.as_ptr(), 0o755) };
            let nfd = unsafe { libc::openat(fd, c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY) };
            unsafe { libc::close(fd) };
            fd = nfd;
            full.push(&comp);
        }
        let leaf = CString::new("leaf.txt").unwrap();
        let ffd = unsafe { libc::openat(fd, leaf.as_ptr(), libc::O_CREAT | libc::O_WRONLY, 0o644) };
        unsafe { libc::close(ffd) };
        full.push("leaf.txt");
        println!(
            "longpath: full path length = {} bytes (PATH_MAX={})",
            full.as_os_str().len(),
            libc::PATH_MAX
        );
        match fs::symlink_metadata(&full) {
            Ok(m) => println!("longpath: std::fs::symlink_metadata(full) OK size={}", m.size()),
            Err(e) => println!("longpath: std::fs::symlink_metadata(full) -> {e}"),
        }
        match fs::read_dir(full.parent().unwrap()) {
            Ok(_) => println!("longpath: std::fs::read_dir(parent) OK"),
            Err(e) => println!("longpath: std::fs::read_dir(parent) -> {e}"),
        }
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let r = unsafe { libc::fstatat(fd, leaf.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) };
        println!(
            "longpath: fstatat(dirfd, \"leaf.txt\") -> {}",
            if r == 0 { "OK".into() } else { errname(errno()) }
        );
        unsafe { libc::close(fd) };
    }
}
