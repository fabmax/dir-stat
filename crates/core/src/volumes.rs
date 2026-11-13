//! Start-screen locations and the progress estimate of a volume scan (design §8, UX §4).
//! Everything here reads only the cached mount table, so the UI thread may call it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::platform::{self, Mount, Space};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardKind {
    Home,
    System,
    External,
    Network,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeCard {
    pub path: PathBuf,
    pub name: String,
    pub kind: CardKind,
    /// `None` for Home and for network volumes, which are never statfs'ed.
    pub space: Option<Space>,
}

/// Home, the system disk, then other volumes a user would recognize.
pub fn start_cards() -> Vec<VolumeCard> {
    cards_from(
        platform::home_dir().as_deref(),
        &platform::mounts(),
        &platform::system_disk_name(),
    )
}

/// Linux mount roots that hold user-visible volumes.
const LINUX_VOLUME_ROOTS: &[&str] = &["/home", "/media", "/mnt", "/run/media"];
const LINUX_REAL_TYPES: &[&str] = &[
    "ext2", "ext3", "ext4", "xfs", "btrfs", "f2fs", "vfat", "exfat", "ntfs", "ntfs3", "zfs",
    "fuseblk",
];

pub fn cards_from(home: Option<&Path>, mounts: &[Mount], system_name: &str) -> Vec<VolumeCard> {
    let mut cards = Vec::new();
    if let Some(home) = home {
        cards.push(VolumeCard {
            path: home.into(),
            name: "Home".into(),
            kind: CardKind::Home,
            space: None,
        });
    }
    if cfg!(windows) {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let path = PathBuf::from(format!("{drive}\\"));
        cards.push(VolumeCard {
            path,
            name: system_name.into(),
            kind: CardKind::System,
            space: None,
        });
        return cards;
    }
    let root = mounts.iter().find(|m| m.path == Path::new("/"));
    cards.push(VolumeCard {
        path: "/".into(),
        name: system_name.into(),
        kind: CardKind::System,
        space: root.and_then(|m| m.space),
    });
    let mut seen_devs = HashSet::new();
    for mount in mounts {
        let recognizable = if cfg!(target_os = "macos") {
            mount.path.starts_with("/Volumes")
                && mount.path != Path::new("/Volumes")
                && mount.browsable
        } else {
            LINUX_VOLUME_ROOTS.iter().any(|r| mount.path.starts_with(r))
                && (LINUX_REAL_TYPES.contains(&mount.fs_type.as_str()) || !mount.local)
        };
        if !recognizable || mount.dev.is_some_and(|dev| !seen_devs.insert(dev)) {
            continue;
        }
        let name = mount.path.file_name().map_or_else(
            || mount.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let (kind, space) = if mount.local {
            (CardKind::External, mount.space)
        } else {
            (CardKind::Network, None)
        };
        cards.push(VolumeCard {
            path: mount.path.clone(),
            name,
            kind,
            space,
        });
    }
    cards
}

/// What a volume-root scan will roughly find (D3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanEstimate {
    /// Inodes in use: progress = scanned items / items.
    pub items: Option<u64>,
    /// Fallback when there are no inode counts (btrfs).
    pub bytes: Option<u64>,
    /// The container's "used", for the Unaccounted row.
    pub container_used: Option<u64>,
}

impl ScanEstimate {
    /// Progress in 0..=0.99; it only reaches 1 when the scan completes.
    pub fn fraction(&self, items_scanned: u64, bytes_scanned: u64) -> Option<f64> {
        let raw = match (self.items, self.bytes) {
            (Some(items), _) if items > 0 => items_scanned as f64 / items as f64,
            (_, Some(bytes)) if bytes > 0 => bytes_scanned as f64 / bytes as f64,
            _ => return None,
        };
        Some(raw.min(0.99))
    }
}

/// `Some` only if `root` is a volume root. macOS "/" adds the Data volume, which the scan
/// traverses through firmlinks.
pub fn estimate(root: &Path, mounts: &[Mount]) -> Option<ScanEstimate> {
    let space_at = |p: &Path| mounts.iter().find(|m| m.path == p).and_then(|m| m.space);
    let own = space_at(root)?;
    let mut parts = vec![own];
    if cfg!(target_os = "macos") && root == Path::new("/") {
        parts.extend(space_at(Path::new("/System/Volumes/Data")));
    }
    let items = parts.iter().map(|s| s.inodes_used).sum::<Option<u64>>();
    Some(ScanEstimate {
        items,
        bytes: Some(parts.iter().map(|s| s.used).sum()),
        container_used: Some(own.used),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mount(
        path: &str,
        fs_type: &str,
        local: bool,
        browsable: bool,
        dev: u64,
        used: u64,
    ) -> Mount {
        Mount {
            path: path.into(),
            fs_type: fs_type.into(),
            local,
            read_only: false,
            browsable,
            automounted: false,
            dev: Some(dev),
            space: Some(Space {
                total: 1000,
                used,
                free: 1000 - used,
                inodes_used: Some(used / 10),
            }),
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_cards() {
        let mounts = [
            mount("/", "apfs", true, true, 1, 100),
            mount("/System/Volumes/Data", "apfs", true, false, 2, 800),
            mount("/System/Volumes/VM", "apfs", true, false, 3, 10),
            mount("/Volumes/Backup", "hfs", true, true, 4, 600),
            mount("/Volumes/share", "smbfs", false, true, 5, 300),
            mount("/Volumes/Recovery", "apfs", true, false, 6, 1),
        ];
        let cards = cards_from(Some(Path::new("/Users/u")), &mounts, "Macintosh HD");
        let summary: Vec<_> = cards
            .iter()
            .map(|c| (c.name.as_str(), c.kind, c.space.is_some()))
            .collect();
        assert_eq!(
            summary,
            [
                ("Home", CardKind::Home, false),
                ("Macintosh HD", CardKind::System, true),
                ("Backup", CardKind::External, true),
                ("share", CardKind::Network, false),
            ]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_cards() {
        let mounts = [
            mount("/", "ext4", true, true, 1, 100),
            mount("/proc", "proc", true, true, 2, 0),
            mount("/home", "ext4", true, true, 3, 500),
            mount("/mnt/bind-of-home", "ext4", true, true, 3, 500),
            mount("/run/media/u/stick", "vfat", true, true, 4, 10),
            mount("/mnt/tmp", "tmpfs", true, true, 5, 1),
            mount("/mnt/nas", "nfs4", false, true, 6, 0),
        ];
        let cards = cards_from(Some(Path::new("/home/u")), &mounts, "Computer");
        let names: Vec<_> = cards.iter().map(|c| (c.name.as_str(), c.kind)).collect();
        assert_eq!(
            names,
            [
                ("Home", CardKind::Home),
                ("Computer", CardKind::System),
                ("home", CardKind::External),
                ("stick", CardKind::External),
                ("nas", CardKind::Network),
            ]
        );
    }

    #[test]
    fn estimates_only_volume_roots() {
        let mounts = [
            mount("/", "apfs", true, true, 1, 100),
            mount("/System/Volumes/Data", "apfs", true, false, 2, 800),
        ];
        assert_eq!(estimate(Path::new("/Users"), &mounts), None);
        let e = estimate(Path::new("/"), &mounts).unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(
                (e.items, e.bytes),
                (Some(90), Some(900)),
                "/ and the Data volume"
            );
        } else {
            assert_eq!((e.items, e.bytes), (Some(10), Some(100)));
        }
        assert_eq!(e.container_used, Some(100));
        assert_eq!(
            e.fraction(45, 0).map(|f| (f * 1000.0).round()),
            Some(if cfg!(target_os = "macos") {
                500.0
            } else {
                990.0
            })
        );
        let bytes_only = ScanEstimate {
            items: None,
            bytes: Some(200),
            container_used: None,
        };
        assert_eq!(bytes_only.fraction(0, 50), Some(0.25));
        assert_eq!(ScanEstimate::default().fraction(10, 10), None);
    }
}
