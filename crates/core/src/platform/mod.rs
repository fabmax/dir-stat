//! Everything OS-specific lives here, behind one set of signatures (design §8). `mod.rs`
//! holds the shared types and the pure helpers; the cfg-selected module does the syscalls.

use std::io;
use std::path::{Path, PathBuf};

use crate::scan::ErrorClass;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", test))]
mod mountinfo;
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod other;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
use other as imp;
#[cfg(windows)]
use windows as imp;

/// macOS `st_flags` bits libc lacks (FS §2.11).
pub mod flags {
    pub const SF_RESTRICTED: u32 = 0x0008_0000;
    pub const SF_NOUNLINK: u32 = 0x0010_0000;
    pub const SF_FIRMLINK: u32 = 0x0080_0000;
    /// Contents live in the cloud (iCloud Drive, File Provider); reading may download them.
    pub const SF_DATALESS: u32 = 0x4000_0000;
}

/// Called once at startup, before the first window opens.
pub fn init_process() {
    imp::init_process();
}

/// Called at the start of every thread or blocking closure that touches the scanned
/// filesystem. Thread-scoped policies are not inherited, so each one calls it.
pub fn init_io_thread() {
    imp::init_io_thread();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
    /// Sockets, FIFOs, devices.
    Other,
}

/// The `struct stat` fields the scanner reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RawStat {
    pub mode: u32,
    pub dev: u64,
    pub ino: u64,
    pub nlink: u64,
    pub blocks: u64,
    pub size: u64,
    pub mtime: i64,
    /// `st_flags` (macOS, BSD); 0 elsewhere.
    pub flags: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryMeta {
    pub kind: EntryKind,
    pub dev: u64,
    pub ino: u64,
    pub nlink: u64,
    /// Size on disk: `st_blocks * 512`, what `du` counts.
    pub alloc: u64,
    pub apparent: u64,
    pub mtime: i64,
    /// macOS `SF_DATALESS`: a cloud placeholder that must never be read.
    pub dataless: bool,
    pub flags: u32,
}

const S_IFMT: u32 = 0o170_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFREG: u32 = 0o100_000;
const S_IFLNK: u32 = 0o120_000;

/// Decodes a stat. Pure, so the flag logic is testable without real iCloud files.
pub fn meta_from_raw(raw: &RawStat) -> EntryMeta {
    let kind = match raw.mode & S_IFMT {
        S_IFDIR => EntryKind::Dir,
        S_IFREG => EntryKind::File,
        S_IFLNK => EntryKind::Symlink,
        _ => EntryKind::Other,
    };
    EntryMeta {
        kind,
        dev: raw.dev,
        ino: raw.ino,
        nlink: raw.nlink,
        alloc: raw.blocks * 512,
        apparent: raw.size,
        mtime: raw.mtime,
        dataless: cfg!(target_os = "macos") && raw.flags & flags::SF_DATALESS != 0,
        flags: raw.flags,
    }
}

/// Metadata of an entry that was not followed if it is a symlink.
pub fn entry_meta(m: &std::fs::Metadata) -> EntryMeta {
    imp::entry_meta(m)
}

/// Classifies an I/O error once, in core (FS §0 rule 7).
pub fn classify_error(e: &io::Error) -> ErrorClass {
    #[cfg(unix)]
    {
        match e.raw_os_error() {
            Some(libc::EPERM) if cfg!(target_os = "macos") => ErrorClass::PrivacyBlocked,
            Some(libc::EPERM | libc::EACCES) => ErrorClass::PermissionDenied,
            Some(libc::EDEADLK) => ErrorClass::CloudNotDownloaded,
            Some(libc::ENAMETOOLONG) => ErrorClass::PathTooLong,
            _ => ErrorClass::Other,
        }
    }
    #[cfg(not(unix))]
    {
        match e.kind() {
            io::ErrorKind::PermissionDenied => ErrorClass::PermissionDenied,
            _ => ErrorClass::Other,
        }
    }
}

/// The directory disappeared between being listed by its parent and being read.
pub fn is_vanished_dir(e: &io::Error) -> bool {
    #[cfg(unix)]
    {
        matches!(e.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR))
    }
    #[cfg(not(unix))]
    {
        e.kind() == io::ErrorKind::NotFound
    }
}

/// The entry disappeared or was replaced between readdir and stat.
pub fn is_vanished_entry(e: &io::Error) -> bool {
    #[cfg(unix)]
    {
        matches!(
            e.raw_os_error(),
            Some(libc::ENOENT | libc::ENOTDIR | libc::ELOOP)
        )
    }
    #[cfg(not(unix))]
    {
        e.kind() == io::ErrorKind::NotFound
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Space {
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub inodes_used: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub path: PathBuf,
    pub fs_type: String,
    pub local: bool,
    pub read_only: bool,
    /// macOS: not `MNT_DONTBROWSE` (all of /System/Volumes/*, devfs, autofs are hidden).
    pub browsable: bool,
    pub automounted: bool,
    pub dev: Option<u64>,
    pub space: Option<Space>,
}

/// The mount table. Never blocks: macOS uses cached statfs data, Linux reads mountinfo text.
pub fn mounts() -> Vec<Mount> {
    imp::mounts()
}

pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

/// What users call the system disk: "Macintosh HD" (the `/Volumes/<name> -> /` symlink),
/// "Computer" on Linux, "Local Disk (C:)" on Windows.
pub fn system_disk_name() -> String {
    imp::system_disk_name()
}

/// macOS: whether dir-stat has Full Disk Access (it can list the TCC folder). `None`
/// elsewhere, where there is no such permission.
pub fn full_disk_access() -> Option<bool> {
    imp::full_disk_access()
}

/// The Privacy & Security ▸ Full Disk Access pane (macOS 13+, D6).
pub const PRIVACY_SETTINGS_URL: Option<&str> = if cfg!(target_os = "macos") {
    Some("x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_AllFiles")
} else {
    None
};

/// The app dir-stat was started from, if it was a terminal: macOS grants privacy permissions
/// to that app, so the Full Disk Access hint must name it (UX §10.3). `None` off macOS.
pub fn launched_from_terminal(own_bundle_id: Option<&str>) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    terminal_app_name(
        std::env::var("TERM_PROGRAM").ok().as_deref(),
        std::env::var("__CFBundleIdentifier").ok().as_deref(),
        own_bundle_id,
    )
}

/// `TERM_PROGRAM` first; else `__CFBundleIdentifier` unless it is our own (LaunchServices
/// sets it for every app, including dir-stat.app started from Finder).
pub fn terminal_app_name(
    term_program: Option<&str>,
    bundle_id: Option<&str>,
    own_bundle_id: Option<&str>,
) -> Option<String> {
    let known = |id: &str| -> Option<&'static str> {
        Some(match id {
            "Apple_Terminal" | "com.apple.Terminal" => "Terminal",
            "iTerm.app" | "com.googlecode.iterm2" => "iTerm",
            "vscode" | "com.microsoft.VSCode" => "Visual Studio Code",
            "WarpTerminal" | "dev.warp.Warp-Stable" => "Warp",
            "ghostty" | "com.mitchellh.ghostty" => "Ghostty",
            "WezTerm" | "com.github.wez.wezterm" => "WezTerm",
            "Hyper" | "co.zeit.hyper" => "Hyper",
            "kitty" | "net.kovidgoyal.kitty" => "kitty",
            "alacritty" | "org.alacritty" => "Alacritty",
            _ => return None,
        })
    };
    if let Some(program) = term_program.filter(|p| !p.is_empty()) {
        return Some(known(program).map_or_else(|| program.to_string(), str::to_string));
    }
    let id = bundle_id.filter(|id| !id.is_empty() && Some(*id) != own_bundle_id)?;
    Some(known(id).map_or_else(|| id.to_string(), str::to_string))
}

/// The filesystem type of the mount containing `path` (longest matching mount point).
pub fn fs_type_of(path: &Path, mounts: &[Mount]) -> String {
    mounts
        .iter()
        .filter(|m| path.starts_with(&m.path))
        .max_by_key(|m| m.path.as_os_str().len())
        .map(|m| m.fs_type.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(mode: u32, flags: u32) -> RawStat {
        RawStat {
            mode,
            dev: 7,
            ino: 42,
            nlink: 2,
            blocks: 8,
            size: 1234,
            mtime: 99,
            flags,
        }
    }

    #[test]
    fn decodes_kinds_and_sizes() {
        let m = meta_from_raw(&raw(S_IFREG | 0o644, 0));
        assert_eq!(m.kind, EntryKind::File);
        assert_eq!(
            (m.alloc, m.apparent, m.dev, m.ino, m.nlink, m.mtime),
            (4096, 1234, 7, 42, 2, 99)
        );
        assert_eq!(meta_from_raw(&raw(S_IFDIR | 0o755, 0)).kind, EntryKind::Dir);
        assert_eq!(meta_from_raw(&raw(S_IFLNK, 0)).kind, EntryKind::Symlink);
        assert_eq!(
            meta_from_raw(&raw(0o010_000, 0)).kind,
            EntryKind::Other,
            "FIFO"
        );
    }

    #[test]
    fn dataless_only_on_macos() {
        let m = meta_from_raw(&raw(S_IFDIR, flags::SF_DATALESS | flags::SF_RESTRICTED));
        assert_eq!(m.dataless, cfg!(target_os = "macos"));
        assert_eq!(m.flags, flags::SF_DATALESS | flags::SF_RESTRICTED);
        assert!(!meta_from_raw(&raw(S_IFDIR, flags::SF_RESTRICTED)).dataless);
    }

    #[cfg(unix)]
    #[test]
    fn classifies_errors() {
        let e = |errno| io::Error::from_raw_os_error(errno);
        let eperm = if cfg!(target_os = "macos") {
            ErrorClass::PrivacyBlocked
        } else {
            ErrorClass::PermissionDenied
        };
        assert_eq!(classify_error(&e(libc::EPERM)), eperm);
        assert_eq!(
            classify_error(&e(libc::EACCES)),
            ErrorClass::PermissionDenied
        );
        assert_eq!(
            classify_error(&e(libc::EDEADLK)),
            ErrorClass::CloudNotDownloaded
        );
        assert_eq!(
            classify_error(&e(libc::ENAMETOOLONG)),
            ErrorClass::PathTooLong
        );
        assert_eq!(classify_error(&e(libc::EIO)), ErrorClass::Other);
        assert!(is_vanished_dir(&e(libc::ENOENT)) && is_vanished_dir(&e(libc::ENOTDIR)));
        assert!(!is_vanished_dir(&e(libc::ELOOP)) && is_vanished_entry(&e(libc::ELOOP)));
    }

    #[test]
    fn terminal_names() {
        assert_eq!(
            terminal_app_name(Some("Apple_Terminal"), None, None).as_deref(),
            Some("Terminal")
        );
        assert_eq!(
            terminal_app_name(Some("iTerm.app"), Some("com.googlecode.iterm2"), None).as_deref(),
            Some("iTerm")
        );
        assert_eq!(
            terminal_app_name(Some("SomeTerm"), None, None).as_deref(),
            Some("SomeTerm")
        );
        assert_eq!(
            terminal_app_name(None, Some("com.apple.Terminal"), None).as_deref(),
            Some("Terminal")
        );
        // Started from Finder: LaunchServices sets our own bundle id.
        let own = Some("com.example.dir-stat");
        assert_eq!(terminal_app_name(None, own, own), None);
        assert_eq!(terminal_app_name(None, None, None), None);
    }

    #[test]
    fn fs_type_uses_the_longest_mount() {
        let mount = |path: &str, fs: &str| Mount {
            path: path.into(),
            fs_type: fs.into(),
            local: true,
            read_only: false,
            browsable: true,
            automounted: false,
            dev: None,
            space: None,
        };
        let mounts = [mount("/", "apfs"), mount("/Volumes/Stick", "msdos")];
        assert_eq!(fs_type_of(Path::new("/Volumes/Stick/a"), &mounts), "msdos");
        assert_eq!(fs_type_of(Path::new("/Volumes/Stickers"), &mounts), "apfs");
        assert_eq!(fs_type_of(Path::new("relative"), &mounts), "");
    }
}
