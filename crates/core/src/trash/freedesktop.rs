//! A rename-only freedesktop Trash (design §7.5, D14). The `trash` crate falls back to a
//! recursive cross-device copy plus `remove_dir_all` when a disk has no usable Trash; this one
//! only ever renames, so nothing can be copied across devices or deleted.

use std::path::{Path, PathBuf};

/// What a probed path is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    Missing,
    Dir {
        dev: u64,
        symlink: bool,
        sticky: bool,
    },
    Other,
}

/// Where the item goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    /// `$XDG_DATA_HOME/Trash`; `Path=` in the info file is absolute.
    Home(PathBuf),
    /// A per-volume trash; `Path=` is relative to `topdir`.
    Volume {
        trash: PathBuf,
        topdir: PathBuf,
    },
    NoTrash,
}

/// The spec's choice, as a pure function of the probes:
/// 1. the home trash if it is on the target's device;
/// 2. `$topdir/.Trash/$uid` if `$topdir/.Trash` is a real, sticky directory on that device;
/// 3. else `$topdir/.Trash-$uid` (the caller creates it with mode 0700 and must verify its
///    device afterwards).
pub fn choose(
    target_dev: u64,
    home_trash: Option<(PathBuf, u64)>,
    topdir: &Path,
    uid: u32,
    probe: impl Fn(&Path) -> Probe,
) -> Choice {
    if let Some((home, dev)) = home_trash
        && dev == target_dev
    {
        return Choice::Home(home);
    }
    let shared = topdir.join(".Trash");
    if let Probe::Dir {
        dev,
        symlink: false,
        sticky: true,
    } = probe(&shared)
        && dev == target_dev
    {
        return Choice::Volume {
            trash: shared.join(uid.to_string()),
            topdir: topdir.into(),
        };
    }
    match probe(&topdir.join(format!(".Trash-{uid}"))) {
        Probe::Missing => Choice::Volume {
            trash: topdir.join(format!(".Trash-{uid}")),
            topdir: topdir.into(),
        },
        Probe::Dir {
            dev,
            symlink: false,
            ..
        } if dev == target_dev => Choice::Volume {
            trash: topdir.join(format!(".Trash-{uid}")),
            topdir: topdir.into(),
        },
        _ => Choice::NoTrash,
    }
}

/// `Path=` value: percent-encoded bytes, except unreserved characters and '/'.
pub fn encode_path(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(target_os = "linux")]
pub fn trash(path: &Path) -> Result<(), super::TrashError> {
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

    use super::TrashError;

    let os = |e: std::io::Error| TrashError::Os(e.to_string());
    let meta = std::fs::symlink_metadata(path).map_err(|_| TrashError::Changed)?;
    let target_dev = meta.dev();
    let probe = |p: &Path| match std::fs::symlink_metadata(p) {
        Err(_) => Probe::Missing,
        Ok(m) if m.file_type().is_symlink() => Probe::Dir {
            dev: m.dev(),
            symlink: true,
            sticky: false,
        },
        Ok(m) if m.is_dir() => Probe::Dir {
            dev: m.dev(),
            symlink: false,
            sticky: m.permissions().mode() & 0o1000 != 0,
        },
        Ok(_) => Probe::Other,
    };

    // The home trash, created if missing.
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| crate::platform::home_dir().map(|h| h.join(".local/share")));
    let home_trash = data_home.map(|d| d.join("Trash")).and_then(|t| {
        std::fs::create_dir_all(t.join("files")).ok()?;
        std::fs::create_dir_all(t.join("info")).ok()?;
        Some((t.clone(), std::fs::metadata(&t).ok()?.dev()))
    });

    // The target's mount: the longest mount point it lies under, confirmed by device.
    let canonical = std::fs::canonicalize(path.parent().unwrap_or(path))
        .map_err(os)?
        .join(path.file_name().unwrap_or_default());
    let topdir = crate::platform::mounts()
        .into_iter()
        .filter(|m| {
            canonical.starts_with(&m.path)
                && std::fs::metadata(&m.path).is_ok_and(|md| md.dev() == target_dev)
        })
        .max_by_key(|m| m.path.as_os_str().len())
        .map(|m| m.path)
        .ok_or(TrashError::NoTrash)?;

    let uid = unsafe { libc::getuid() };
    let (trash, relative_to) = match choose(target_dev, home_trash, &topdir, uid, probe) {
        Choice::Home(trash) => (trash, None),
        Choice::Volume { trash, topdir } => (trash, Some(topdir)),
        Choice::NoTrash => return Err(TrashError::NoTrash),
    };
    for sub in ["files", "info"] {
        let dir = trash.join(sub);
        if !dir.exists() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&dir)
                .map_err(|_| TrashError::NoTrash)?;
        }
    }
    let files = trash.join("files");
    if std::fs::metadata(&files).map(|m| m.dev()).ok() != Some(target_dev) {
        return Err(TrashError::NoTrash);
    }

    let original = match &relative_to {
        Some(topdir) => canonical
            .strip_prefix(topdir)
            .unwrap_or(&canonical)
            .to_path_buf(),
        None => canonical.clone(),
    };
    let date = deletion_date();
    let base = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    // A unique name: O_EXCL on the info file reserves it.
    for n in 0..10_000 {
        let name = if n == 0 {
            base.clone()
        } else {
            format!("{base}.{n}")
        };
        let info_path = trash.join("info").join(format!("{name}.trashinfo"));
        let mut info = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&info_path)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(os(e)),
        };
        let body = format!(
            "[Trash Info]\nPath={}\nDeletionDate={date}\n",
            encode_path(original.as_os_str().as_bytes())
        );
        info.write_all(body.as_bytes()).map_err(os)?;
        drop(info);
        // Rename only, never copy: EXDEV and every other error leave the item in place.
        return match std::fs::rename(path, files.join(&name)) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(&info_path);
                Err(os(e))
            }
        };
    }
    Err(TrashError::NoTrash)
}

/// Local time as `YYYY-MM-DDThh:mm:ss`, as the spec asks.
#[cfg(target_os = "linux")]
fn deletion_date() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(dev: u64, sticky: bool) -> Probe {
        Probe::Dir {
            dev,
            symlink: false,
            sticky,
        }
    }

    #[test]
    fn trash_directory_choice() {
        let home = Some((PathBuf::from("/home/u/.local/share/Trash"), 1));
        let top = Path::new("/mnt/disk");
        // Same device as the home trash.
        assert_eq!(
            choose(1, home.clone(), top, 1000, |_| Probe::Missing),
            Choice::Home("/home/u/.local/share/Trash".into())
        );
        // Another disk with a proper shared .Trash.
        let shared = |p: &Path| {
            if p.ends_with(".Trash") {
                dir(2, true)
            } else {
                Probe::Missing
            }
        };
        assert_eq!(
            choose(2, home.clone(), top, 1000, shared),
            Choice::Volume {
                trash: "/mnt/disk/.Trash/1000".into(),
                topdir: top.into()
            }
        );
        // .Trash without the sticky bit, or a symlink: use .Trash-$uid instead.
        let unsticky = |p: &Path| {
            if p.ends_with(".Trash") {
                dir(2, false)
            } else {
                Probe::Missing
            }
        };
        assert_eq!(
            choose(2, home.clone(), top, 1000, unsticky),
            Choice::Volume {
                trash: "/mnt/disk/.Trash-1000".into(),
                topdir: top.into()
            }
        );
        let symlinked = |p: &Path| {
            if p.ends_with(".Trash") {
                Probe::Dir {
                    dev: 2,
                    symlink: true,
                    sticky: true,
                }
            } else {
                Probe::Missing
            }
        };
        assert!(
            matches!(choose(2, home.clone(), top, 1000, symlinked), Choice::Volume { trash, .. } if trash.ends_with(".Trash-1000"))
        );
        // An existing .Trash-$uid on another device (a mount on top), or a file: no trash.
        let wrong_dev = |p: &Path| {
            if p.ends_with(".Trash-1000") {
                dir(9, false)
            } else {
                Probe::Missing
            }
        };
        assert_eq!(
            choose(2, home.clone(), top, 1000, wrong_dev),
            Choice::NoTrash
        );
        assert_eq!(
            choose(2, home, top, 1000, |p| if p.ends_with(".Trash-1000") {
                Probe::Other
            } else {
                Probe::Missing
            }),
            Choice::NoTrash
        );
        // No home trash at all (no home): still the volume trash.
        assert!(matches!(
            choose(2, None, top, 1000, |_| Probe::Missing),
            Choice::Volume { .. }
        ));
    }

    #[test]
    fn info_paths_are_percent_encoded() {
        assert_eq!(
            encode_path(b"/home/u/my file%.txt"),
            "/home/u/my%20file%25.txt"
        );
        assert_eq!(encode_path(b"a/\xff"), "a/%FF");
    }
}
