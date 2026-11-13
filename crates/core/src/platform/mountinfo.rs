//! Parser for Linux `/proc/self/mountinfo`. Pure, so it is tested on every OS.
//!
//! Line format: `36 35 98:0 /mnt1 /mnt/parent rw,noatime master:1 - ext3 /dev/root rw`.
//! Field 5 is the mount point and field 6 the per-mount options; the filesystem type follows
//! the ` - ` separator. Space, tab, newline and backslash in paths are escaped as `\ooo`.

use std::ffi::OsString;
use std::path::PathBuf;

use super::Mount;

/// Network filesystems by type; everything else counts as local.
const NETWORK_TYPES: &[&str] = &[
    "nfs",
    "nfs4",
    "cifs",
    "smb3",
    "smbfs",
    "9p",
    "ceph",
    "glusterfs",
    "lustre",
    "afs",
    "sshfs",
];

pub fn parse(text: &[u8]) -> Vec<Mount> {
    text.split(|&b| b == b'\n').filter_map(parse_line).collect()
}

fn parse_line(line: &[u8]) -> Option<Mount> {
    let fields: Vec<&[u8]> = line.split(|&b| b == b' ').collect();
    let separator = fields.iter().position(|&f| f == b"-")?;
    let dev = parse_dev(fields.get(2)?);
    let path = PathBuf::from(os_string(unescape(fields.get(4)?)));
    let read_only = fields.get(5)?.split(|&b| b == b',').any(|o| o == b"ro");
    let fs_type = String::from_utf8_lossy(fields.get(separator + 1)?).into_owned();
    let local = !(NETWORK_TYPES.contains(&fs_type.as_str()) || fs_type.starts_with("fuse."));
    Some(Mount {
        path,
        automounted: fs_type == "autofs",
        fs_type,
        local,
        read_only,
        browsable: true,
        dev,
        space: None,
    })
}

/// `maj:min` as a `dev_t` (glibc encoding).
fn parse_dev(field: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(field).ok()?;
    let (major, minor) = text.split_once(':')?;
    let (major, minor): (u64, u64) = (major.parse().ok()?, minor.parse().ok()?);
    Some(
        ((major & 0xffff_f000) << 32)
            | ((major & 0xfff) << 8)
            | ((minor & 0xffff_ff00) << 12)
            | (minor & 0xff),
    )
}

pub fn unescape(field: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(field.len());
    let mut i = 0;
    while i < field.len() {
        let octal = field
            .get(i + 1..i + 4)
            .filter(|d| d.iter().all(|c| (b'0'..=b'7').contains(c)));
        match octal {
            Some(d) if field[i] == b'\\' => {
                out.push((d[0] - b'0') * 64 + (d[1] - b'0') * 8 + (d[2] - b'0'));
                i += 4;
            }
            _ => {
                out.push(field[i]);
                i += 1;
            }
        }
    }
    out
}

#[cfg(unix)]
fn os_string(bytes: Vec<u8>) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes)
}

#[cfg(not(unix))]
fn os_string(bytes: Vec<u8>) -> OsString {
    String::from_utf8_lossy(&bytes).into_owned().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
23 22 0:21 / /proc rw,nosuid shared:12 - proc proc rw
40 22 259:3 / /mnt/my\\040disk ro,relatime shared:30 - ext4 /dev/nvme0n1p3 ro
41 22 0:45 / /home/u/net rw shared:31 - nfs4 server:/export rw
42 22 0:46 / /home/u/sshfs rw shared:32 - fuse.sshfs u@h: rw
43 22 0:47 / /mnt/auto rw shared:33 - autofs systemd-1 rw
garbage line
";

    #[test]
    fn parses_mount_points_types_and_flags() {
        let mounts = parse(SAMPLE);
        let paths: Vec<_> = mounts.iter().map(|m| m.path.to_str().unwrap()).collect();
        assert_eq!(
            paths,
            [
                "/",
                "/proc",
                "/mnt/my disk",
                "/home/u/net",
                "/home/u/sshfs",
                "/mnt/auto"
            ]
        );
        assert_eq!(mounts[0].fs_type, "ext4");
        assert!(mounts[0].local && !mounts[0].read_only);
        assert!(mounts[2].read_only);
        assert!(!mounts[3].local && !mounts[4].local);
        assert!(mounts[5].automounted);
        assert_eq!(mounts[0].dev, Some((259 << 8) | 2));
    }

    #[test]
    fn unescapes_octal_sequences() {
        assert_eq!(unescape(br"/a\040b\011c\012d\134e"), b"/a b\tc\nd\\e");
        assert_eq!(unescape(br"/trailing\04"), br"/trailing\04");
        assert_eq!(unescape(br"/not\999octal"), br"/not\999octal");
    }
}
