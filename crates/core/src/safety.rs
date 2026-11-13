//! Delete guards (design §7.2). They decide what may be moved to the Trash, independent of
//! the catalog: a catalog bug can't open a path to system folders, and these guards can't hide
//! suggestions.
//!
//! Path rules (`check`) compare exact components of canonical paths. Case and symlink variants
//! that slip past them are caught by the `(dev, ino)` check in `check_live`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::platform::{self, flags};
use crate::trash::{Identity, TrashError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// G3/G4: a protected system or home folder; the label names it.
    Protected(&'static str),
    /// G2.
    ScanRootOrAncestor,
    MountPoint,
    ContainsOtherVolume,
    ReadOnlyVolume,
    ImmutableFlag,
    SystemVolumes,
}

impl Refusal {
    /// The refusal dialog's text (UX §9).
    pub fn message(self, name: &str) -> String {
        match self {
            Refusal::Protected(_) | Refusal::ScanRootOrAncestor => {
                format!(
                    "dir-stat won't move “{name}” to the Trash because macOS or your apps need it."
                )
            }
            Refusal::MountPoint => {
                format!("“{name}” is a separate volume. dir-stat won't move it to the Trash.")
            }
            Refusal::ContainsOtherVolume => {
                format!("“{name}” contains another volume. dir-stat won't move it to the Trash.")
            }
            Refusal::ReadOnlyVolume => format!("“{name}” is on a read-only volume."),
            Refusal::ImmutableFlag => format!("“{name}” is locked or protected by the system."),
            Refusal::SystemVolumes => format!("“{name}” is part of the macOS system volumes."),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Guards {
    home: PathBuf,
    /// G3: the folder and everything inside it.
    protected_tree: Vec<(PathBuf, &'static str)>,
    /// Exceptions inside protected trees (`/usr/local`).
    allowed_inside: Vec<PathBuf>,
    /// G4: only the folder itself.
    protected_self: Vec<(PathBuf, &'static str)>,
    /// G4 by identity, for symlinked or differently cased paths.
    protected_ids: HashSet<(u64, u64)>,
    /// Cached mount points (G5 path rule).
    mounts: HashSet<PathBuf>,
}

impl Guards {
    /// The real guards: canonical home, the OS lists, the mount table. Once, at startup.
    pub fn new() -> Guards {
        let home = platform::home_dir().unwrap_or_else(|| PathBuf::from("/nonexistent-home"));
        let home = std::fs::canonicalize(&home).unwrap_or(home);
        let mounts = platform::mounts().into_iter().map(|m| m.path).collect();
        let mut guards = Guards::for_home(home, mounts);
        guards.canonicalize();
        guards
    }

    /// The OS lists for `home`, without touching the disk (tests).
    pub fn for_home(home: PathBuf, mounts: HashSet<PathBuf>) -> Guards {
        let (tree, allowed, own) = os_lists(&home);
        Guards {
            protected_tree: tree
                .into_iter()
                .map(|(p, l)| (PathBuf::from(p), l))
                .collect(),
            allowed_inside: allowed.into_iter().map(PathBuf::from).collect(),
            protected_self: own,
            protected_ids: HashSet::new(),
            mounts,
            home,
        }
    }

    /// Resolves stored paths that exist and records the identity of every G4 folder.
    fn canonicalize(&mut self) {
        let canonical = |p: &PathBuf| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
        for (path, _) in self
            .protected_tree
            .iter_mut()
            .chain(self.protected_self.iter_mut())
        {
            *path = canonical(path);
        }
        // Children of the iCloud and File Provider roots are protected too (G4).
        for root in ["Library/Mobile Documents", "Library/CloudStorage"] {
            if let Ok(entries) = std::fs::read_dir(self.home.join(root)) {
                for entry in entries.flatten() {
                    self.protected_self
                        .push((canonical(&entry.path()), "iCloud and cloud storage"));
                }
            }
        }
        for (path, _) in &self.protected_self {
            if let Ok(meta) = std::fs::symlink_metadata(path) {
                let meta = platform::entry_meta(&meta);
                self.protected_ids.insert((meta.dev, meta.ino));
            }
        }
    }

    pub fn home(&self) -> &Path {
        &self.home
    }
}

impl Default for Guards {
    fn default() -> Self {
        Guards::new()
    }
}

type Lists = (
    Vec<(&'static str, &'static str)>,
    Vec<&'static str>,
    Vec<(PathBuf, &'static str)>,
);

fn os_lists(home: &Path) -> Lists {
    let in_home = |rel: &str, label: &'static str| (home.join(rel), label);
    if cfg!(target_os = "macos") {
        let tree = vec![
            ("/System", "macOS system"),
            ("/usr", "macOS system"),
            ("/bin", "macOS system"),
            ("/sbin", "macOS system"),
            ("/private/etc", "macOS system"),
            ("/dev", "devices"),
        ];
        let mut own: Vec<(PathBuf, &'static str)> = [
            "/",
            "/Library",
            "/Applications",
            "/Users",
            "/Volumes",
            "/private",
            "/private/var",
            "/private/tmp",
            "/opt",
            "/cores",
            "/etc",
            "/var",
            "/tmp",
        ]
        .into_iter()
        .map(|p| (PathBuf::from(p), "system folder"))
        .collect();
        own.push((home.to_path_buf(), "home folder"));
        for rel in [
            "Library",
            "Desktop",
            "Documents",
            "Downloads",
            "Movies",
            "Music",
            "Pictures",
            "Public",
            "Applications",
            ".Trash",
        ] {
            own.push(in_home(rel, "home folder"));
        }
        own.push(in_home("Library/Mobile Documents", "iCloud Drive"));
        own.push(in_home("Library/CloudStorage", "cloud storage"));
        (tree, vec!["/usr/local"], own)
    } else if cfg!(windows) {
        let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let tree = vec![("C:\\Windows", "Windows")];
        let mut own: Vec<(PathBuf, &'static str)> = vec![
            (PathBuf::from(format!("{drive}\\")), "drive"),
            (PathBuf::from(format!("{drive}\\Program Files")), "programs"),
            (
                PathBuf::from(format!("{drive}\\Program Files (x86)")),
                "programs",
            ),
            (PathBuf::from(format!("{drive}\\ProgramData")), "programs"),
            (PathBuf::from(format!("{drive}\\Users")), "users"),
            (PathBuf::from(windir), "Windows"),
            (home.to_path_buf(), "home folder"),
        ];
        for file in ["pagefile.sys", "hiberfil.sys", "swapfile.sys"] {
            own.push((PathBuf::from(format!("{drive}\\{file}")), "system file"));
        }
        (tree, Vec::new(), own)
    } else {
        let tree = vec![
            ("/bin", "system"),
            ("/boot", "system"),
            ("/dev", "devices"),
            ("/etc", "system"),
            ("/lib", "system"),
            ("/lib32", "system"),
            ("/lib64", "system"),
            ("/libx32", "system"),
            ("/proc", "system"),
            ("/run", "system"),
            ("/sbin", "system"),
            ("/sys", "system"),
            ("/usr", "system"),
        ];
        let mut own: Vec<(PathBuf, &'static str)> = ["/", "/home", "/root", "/srv", "/var", "/opt"]
            .into_iter()
            .map(|p| (PathBuf::from(p), "system folder"))
            .collect();
        own.push((home.to_path_buf(), "home folder"));
        for dir in [
            dirs::desktop_dir(),
            dirs::document_dir(),
            dirs::download_dir(),
            dirs::audio_dir(),
            dirs::picture_dir(),
            dirs::video_dir(),
            dirs::public_dir(),
            dirs::template_dir(),
        ]
        .into_iter()
        .flatten()
        {
            own.push((dir, "home folder"));
        }
        (tree, vec!["/usr/local"], own)
    }
}

/// Exact component comparison; case-insensitive on Windows.
fn same(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

fn starts_with(path: &Path, base: &Path) -> bool {
    if cfg!(windows) {
        path.to_string_lossy()
            .to_lowercase()
            .starts_with(&base.to_string_lossy().to_lowercase())
    } else {
        path.starts_with(base)
    }
}

/// G2–G5 path rules. Pure: no syscalls.
pub fn check(guards: &Guards, scan_root: &Path, path: &Path) -> Result<(), Refusal> {
    // G2: the scan root and its ancestors.
    if scan_root.ancestors().any(|a| same(a, path)) {
        return Err(Refusal::ScanRootOrAncestor);
    }
    // G3: whole subtrees, with their exceptions.
    let allowed = guards.allowed_inside.iter().any(|a| starts_with(path, a));
    if !allowed
        && let Some((_, label)) = guards
            .protected_tree
            .iter()
            .find(|(p, _)| starts_with(path, p))
    {
        return Err(Refusal::Protected(label));
    }
    // G4: the folder itself.
    if let Some((_, label)) = guards.protected_self.iter().find(|(p, _)| same(p, path)) {
        return Err(Refusal::Protected(label));
    }
    // G5: mount points and the macOS system volumes.
    if cfg!(target_os = "macos") && path.starts_with("/System/Volumes") {
        return Err(Refusal::SystemVolumes);
    }
    if guards.mounts.iter().any(|m| same(m, path)) {
        return Err(Refusal::MountPoint);
    }
    Ok(())
}

/// G4 by identity, G5 live, G6 identity, G7 flags. Syscalls; call right before moving.
pub fn check_live(guards: &Guards, path: &Path, expect: &Identity) -> Result<(), TrashError> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| TrashError::Changed)?;
    let now = Identity::of(&meta);
    if !expect.matches(&now) {
        return Err(TrashError::Changed);
    }
    let raw = platform::entry_meta(&meta);
    if guards.protected_ids.contains(&(raw.dev, raw.ino)) {
        return Err(TrashError::Refused(Refusal::Protected("protected folder")));
    }
    const IMMUTABLE: u32 = 0x0000_0002 | 0x0002_0000; // UF_IMMUTABLE | SF_IMMUTABLE
    if raw.flags & (flags::SF_RESTRICTED | flags::SF_NOUNLINK | IMMUTABLE) != 0 {
        return Err(TrashError::Refused(Refusal::ImmutableFlag));
    }
    if platform::is_read_only(path) {
        return Err(TrashError::Refused(Refusal::ReadOnlyVolume));
    }
    // Mounts created after the scan.
    for mount in platform::mounts() {
        if same(&mount.path, path) {
            return Err(TrashError::Refused(Refusal::MountPoint));
        }
        if raw.kind == platform::EntryKind::Dir && mount.path.starts_with(path) {
            return Err(TrashError::Refused(Refusal::ContainsOtherVolume));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guards() -> Guards {
        let home = if cfg!(windows) {
            r"C:\Users\u"
        } else if cfg!(target_os = "macos") {
            "/Users/u"
        } else {
            "/home/u"
        };
        let mounts = HashSet::from([PathBuf::from("/Volumes/Backup"), PathBuf::from("/mnt/data")]);
        Guards::for_home(home.into(), mounts)
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_paths() {
        let g = guards();
        let root = Path::new("/Users/u/dev");
        let refused = |p: &str| check(&g, root, Path::new(p)).is_err();
        // G3 whole subtrees, with /usr/local allowed.
        for p in [
            "/System",
            "/System/Library/Fonts",
            "/usr",
            "/usr/lib/x",
            "/bin/ls",
            "/private/etc/hosts",
            "/dev/null",
        ] {
            assert!(refused(p), "{p}");
        }
        assert!(!refused("/usr/local/x"));
        assert!(!refused("/usr/local"));
        // G4 the folder itself; things inside are fine.
        for p in [
            "/",
            "/Applications",
            "/Library",
            "/Users",
            "/Volumes",
            "/private/var",
            "/private/tmp",
            "/Users/u",
            "/Users/u/Library",
            "/Users/u/Documents",
            "/Users/u/.Trash",
            "/Users/u/Library/Mobile Documents",
        ] {
            assert!(refused(p), "{p}");
        }
        for p in [
            "/Applications/Foo.app",
            "/Users/u/Library/Caches/x",
            "/Users/u/Documents/old",
            "/Users/u/dev/proj/target",
            "/Library/Caches/x",
        ] {
            assert!(!refused(p), "{p}");
        }
        // G2 the scan root and its ancestors.
        assert_eq!(check(&g, root, root), Err(Refusal::ScanRootOrAncestor));
        assert_eq!(
            check(
                &g,
                Path::new("/Users/u/dev/a/b"),
                Path::new("/Users/u/dev/a")
            ),
            Err(Refusal::ScanRootOrAncestor)
        );
        // G5 mounts and the system volumes.
        assert_eq!(
            check(&g, Path::new("/"), Path::new("/Volumes/Backup")),
            Err(Refusal::MountPoint)
        );
        assert_eq!(
            check(&g, Path::new("/"), Path::new("/System/Volumes/Data/x")),
            Err(Refusal::Protected("macOS system"))
        );
        assert_eq!(
            check(&g, Path::new("/x"), Path::new("/x/System/Volumes")),
            Ok(()),
            "exact components"
        );
        // No case folding: "/system" is a different folder on a case-sensitive volume.
        assert!(!refused("/Users/u/dev/system"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_paths() {
        let g = guards();
        let root = Path::new("/home/u/dev");
        let refused = |p: &str| check(&g, root, Path::new(p)).is_err();
        for p in [
            "/bin",
            "/boot/x",
            "/etc/x",
            "/lib64/x",
            "/proc/1",
            "/usr/share",
            "/",
            "/home",
            "/home/u",
            "/var",
            "/opt",
            "/srv",
        ] {
            assert!(refused(p), "{p}");
        }
        for p in [
            "/usr/local/x",
            "/var/cache/x",
            "/opt/x",
            "/home/u/dev/proj/target",
        ] {
            assert!(!refused(p), "{p}");
        }
        assert_eq!(
            check(&g, Path::new("/"), Path::new("/mnt/data")),
            Err(Refusal::MountPoint)
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths() {
        let g = guards();
        let root = Path::new(r"C:\Users\u\dev");
        let refused = |p: &str| check(&g, root, Path::new(p)).is_err();
        assert!(refused(r"C:\Windows\System32"));
        assert!(refused(r"c:\windows"), "case-insensitive");
        assert!(refused(r"C:\Users\u"));
        assert!(!refused(r"C:\Users\u\dev\proj\target"));
    }

    #[cfg(unix)]
    #[test]
    fn live_checks_identity() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        let g = guards();
        let expect = Identity::of(&std::fs::symlink_metadata(&file).unwrap());
        assert_eq!(check_live(&g, &file, &expect), Ok(()));
        // Replaced by a symlink after the scan: Changed.
        std::fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink("/etc", &file).unwrap();
        assert_eq!(check_live(&g, &file, &expect), Err(TrashError::Changed));
        std::fs::remove_file(&file).unwrap();
        assert_eq!(
            check_live(&g, &file, &expect),
            Err(TrashError::Changed),
            "gone"
        );
    }

    #[test]
    fn refusal_wording() {
        assert_eq!(
            Refusal::Protected("x").message("Library"),
            "dir-stat won't move “Library” to the Trash because macOS or your apps need it."
        );
        assert!(
            Refusal::ContainsOtherVolume
                .message("data")
                .contains("contains another volume")
        );
    }
}
