//! Git verification of cleanup candidates (design §5.6, D10). A build folder inside a work
//! tree is only suggested once git confirms it is ignored and has no tracked files.
//!
//! Only read-only commands run: `check-ignore` and `ls-files`, with optional locks off, so
//! dir-stat never writes to a repository's index.

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::catalog::m;
use crate::platform;
use crate::tree::{CandidateRef, NodeId, Tree};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GitStatus {
    /// Ignored, and no tracked files inside.
    Clean,
    /// Not ignored: it may hold work that isn't committed.
    NotIgnored,
    /// Contains tracked files: never suggested.
    Tracked,
    /// No git, a timeout, a git error, or a work tree in iCloud that must not be read.
    Unavailable,
}

/// A git executable that can run without side effects.
#[derive(Clone, Debug)]
pub struct GitBinary(pub PathBuf);

/// Candidates of one work tree, with their paths relative to it.
#[derive(Clone, Debug)]
pub struct GitJob {
    pub worktree: PathBuf,
    pub items: Vec<(CandidateRef, PathBuf)>,
}

/// One git command may take this long; then it is killed and the job is `Unavailable`.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Pathspecs per `ls-files` call, to stay far below the argument length limit.
const LS_FILES_CHUNK: usize = 200;

/// macOS: Homebrew's git, else /usr/bin/git only if the developer tools are installed
/// (otherwise /usr/bin/git opens the install dialog, CAT §5.6). Elsewhere: git in PATH.
pub fn find_git() -> Option<GitBinary> {
    if cfg!(target_os = "macos") {
        for path in ["/opt/homebrew/bin/git", "/usr/local/bin/git"] {
            if Path::new(path).is_file() {
                return Some(GitBinary(path.into()));
            }
        }
        let tools = Command::new("/usr/bin/xcode-select")
            .arg("-p")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        return tools
            .is_ok_and(|s| s.success())
            .then(|| GitBinary("/usr/bin/git".into()));
    }
    let exe = if cfg!(windows) { "git.exe" } else { "git" };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(exe))
        .find(|p| p.is_file())
        .map(GitBinary)
}

/// The work tree a candidate belongs to: the nearest folder holding `.git` (for a folder
/// candidate, strictly above it), else the one above the scan root.
pub fn worktree_of(tree: &Tree, candidate: CandidateRef) -> Option<PathBuf> {
    let mut dirs: Box<dyn Iterator<Item = NodeId>> = match candidate {
        CandidateRef::Dir(id) => Box::new(tree.ancestors(id)),
        CandidateRef::File(dir, _) => Box::new(std::iter::once(dir).chain(tree.ancestors(dir))),
    };
    match dirs.find(|&d| tree.node(d).markers() & m::GIT != 0) {
        Some(d) => Some(tree.path_of(d)),
        None => tree.root_context().git_root_above.clone(),
    }
}

/// Groups candidates by work tree. Candidates outside any work tree come back separately:
/// they need no check (`GitState::NotInRepo`).
pub fn plan_jobs(
    tree: &Tree,
    candidates: impl IntoIterator<Item = CandidateRef>,
) -> (Vec<CandidateRef>, Vec<GitJob>) {
    let mut outside = Vec::new();
    let mut by_worktree: BTreeMap<PathBuf, Vec<(CandidateRef, PathBuf)>> = BTreeMap::new();
    for candidate in candidates {
        let path = match candidate {
            CandidateRef::Dir(id) => tree.path_of(id),
            CandidateRef::File(dir, index) => {
                match tree.node(dir).large_files().get(index as usize) {
                    Some(file) => tree.path_of(dir).join(&*file.name),
                    None => continue,
                }
            }
        };
        let Some(worktree) = worktree_of(tree, candidate) else {
            outside.push(candidate);
            continue;
        };
        let rel = path
            .strip_prefix(&worktree)
            .map(Path::to_path_buf)
            .unwrap_or(path);
        by_worktree
            .entry(worktree)
            .or_default()
            .push((candidate, rel));
    }
    let jobs = by_worktree
        .into_iter()
        .map(|(worktree, items)| GitJob { worktree, items })
        .collect();
    (outside, jobs)
}

/// Checks every item of one work tree. Runs git; call it on a blocking thread.
pub fn verify(git: &GitBinary, job: &GitJob) -> Vec<(CandidateRef, GitStatus)> {
    let all = |status| job.items.iter().map(|&(c, _)| (c, status)).collect();
    // git runs with the process default (materialization on): never point it at iCloud.
    let dataless =
        |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| platform::entry_meta(&m).dataless);
    if dataless(&job.worktree) || dataless(&job.worktree.join(".git")) {
        return all(GitStatus::Unavailable);
    }
    let (Some(ignored), Some(tracked)) = (ignored_set(git, job), tracked_set(git, job)) else {
        return all(GitStatus::Unavailable);
    };
    job.items
        .iter()
        .map(|&(candidate, ref rel)| {
            let key = git_path(rel);
            let status = if tracked.iter().any(|t| is_same_or_below(t, &key)) {
                GitStatus::Tracked
            } else if ignored.contains(&key) {
                GitStatus::Clean
            } else {
                GitStatus::NotIgnored
            };
            (candidate, status)
        })
        .collect()
}

/// Items `check-ignore` reports as ignored, as git paths without a trailing slash.
fn ignored_set(git: &GitBinary, job: &GitJob) -> Option<HashSet<Vec<u8>>> {
    let mut input = Vec::new();
    for (candidate, rel) in &job.items {
        input.extend(git_path(rel));
        // A trailing slash makes directory-only patterns like `target/` apply.
        if matches!(candidate, CandidateRef::Dir(_)) {
            input.push(b'/');
        }
        input.push(0);
    }
    let (code, out) = run(
        git,
        &job.worktree,
        &["check-ignore", "-z", "--stdin"].map(OsString::from),
        Some(input),
    )?;
    // 0: some are ignored, 1: none are.
    if !matches!(code, Some(0 | 1)) {
        return None;
    }
    Some(
        split_nul(&out)
            .map(|p| p.strip_suffix(b"/").unwrap_or(p).to_vec())
            .collect(),
    )
}

/// Tracked files below any of the items.
fn tracked_set(git: &GitBinary, job: &GitJob) -> Option<Vec<Vec<u8>>> {
    let mut tracked = Vec::new();
    for chunk in job.items.chunks(LS_FILES_CHUNK) {
        let mut args: Vec<OsString> = ["ls-files", "-z", "--"].map(OsString::from).to_vec();
        args.extend(chunk.iter().map(|(_, rel)| literal_pathspec(rel)));
        let (code, out) = run(git, &job.worktree, &args, None)?;
        if code != Some(0) {
            return None;
        }
        tracked.extend(split_nul(&out).map(<[u8]>::to_vec));
    }
    Some(tracked)
}

/// Runs git in `worktree` with a timeout. `None` if it can't start, times out or dies.
fn run(
    git: &GitBinary,
    worktree: &Path,
    args: &[OsString],
    input: Option<Vec<u8>>,
) -> Option<(Option<i32>, Vec<u8>)> {
    let mut child = Command::new(&git.0)
        .arg("-C")
        .arg(worktree)
        .args(["-c", "core.fsmonitor=false"])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Feed stdin and drain stdout on their own threads, so neither pipe can fill up.
    let writer = input.zip(child.stdin.take()).map(|(bytes, mut stdin)| {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        })
    });
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    Some((status.code(), reader.join().ok()?))
}

fn split_nul(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.split(|&b| b == 0).filter(|p| !p.is_empty())
}

/// A relative path as git writes it: components joined with '/'.
fn git_path(rel: &Path) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, component) in rel.components().enumerate() {
        if i > 0 {
            out.push(b'/');
        }
        out.extend(os_bytes(component.as_os_str()));
    }
    out
}

fn literal_pathspec(rel: &Path) -> OsString {
    let mut spec = OsString::from(":(literal)");
    spec.push(bytes_to_os(git_path(rel)));
    spec
}

#[cfg(unix)]
fn os_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    s.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn os_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
    s.to_string_lossy().into_owned().into_bytes()
}

#[cfg(unix)]
fn bytes_to_os(bytes: Vec<u8>) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes)
}

#[cfg(not(unix))]
fn bytes_to_os(bytes: Vec<u8>) -> OsString {
    String::from_utf8_lossy(&bytes).into_owned().into()
}

/// `path` is `base` or lies below it (byte paths with '/' separators).
fn is_same_or_below(path: &[u8], base: &[u8]) -> bool {
    path == base || (path.starts_with(base) && path.get(base.len()) == Some(&b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_as_git_writes_them() {
        assert_eq!(git_path(Path::new("a/b/target")), b"a/b/target");
        assert_eq!(
            literal_pathspec(Path::new("a/*")),
            OsString::from(":(literal)a/*")
        );
        assert!(is_same_or_below(b"a/target/x.o", b"a/target"));
        assert!(is_same_or_below(b"a/target", b"a/target"));
        assert!(!is_same_or_below(b"a/targets/x", b"a/target"));
        let parts: Vec<&[u8]> = split_nul(b"a\0b/\0\0").collect();
        assert_eq!(parts, [&b"a"[..], &b"b/"[..]]);
    }
}
