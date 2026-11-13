//! The classification context of a scan root (design §5.5), and the other additions to the
//! ported prototype: canonical resolver paths, file matches, cleanup hints.
//!
//! The engine only sees what the scan walks, and a root's parent and siblings are outside
//! the scan. So the root's parent context is computed explicitly, on the scan thread.

use std::path::{Path, PathBuf};

use super::engine::{Ctx, Engine, Resolver, RuleIdx, Verdict, is_valid_cachedir_tag};
use super::model::{Kind, m, marker_of};
use crate::scan::FileMatch;

#[derive(Clone, Debug, Default)]
pub struct RootContext {
    /// Context of the root's parent directory.
    pub parent_ctx: Ctx,
    /// Name of the root's parent directory ("" for a child of "/").
    pub parent_name: String,
    /// Nearest ancestor of the root that contains `.git`.
    pub git_root_above: Option<PathBuf>,
}

impl RootContext {
    /// As if the root's parent were an ordinary folder such as ~/dev. For tests: fixtures in
    /// $TMPDIR (/private/var/folders/… on macOS) sit in a zone where no heuristic applies.
    pub fn detached(engine: &Engine) -> RootContext {
        let parent = if cfg!(windows) {
            r"C:\Users\someone\dev"
        } else {
            "/home/someone/dev"
        };
        RootContext {
            parent_ctx: engine.enter(parent).1,
            parent_name: "dev".into(),
            git_root_above: None,
        }
    }
}

/// For a fresh, canonical scan root. Does a little I/O (one names-only listing of the parent,
/// one lstat per ancestor), so it runs on the scan thread, never on the UI thread.
pub fn root_context(engine: &Engine, root: &Path) -> RootContext {
    let git_root_above = root
        .ancestors()
        .skip(1)
        .find(|a| a.join(".git").symlink_metadata().is_ok())
        .map(Path::to_path_buf);
    let Some(parent) = root.parent() else {
        return RootContext {
            git_root_above,
            ..RootContext::default()
        };
    };
    let parent_markers = names_only_markers(parent);
    let parent_name = name_of(parent);
    let parent_ctx = match parent.parent() {
        // The parent is "/" (or a drive root).
        None => {
            let mut ctx = engine.root().1;
            ctx.markers = parent_markers;
            ctx
        }
        Some(grandparent) => {
            let (_, gp_ctx) = engine.enter(&grandparent.to_string_lossy());
            engine
                .classify_dir(&gp_ctx, &name_of(grandparent), &parent_name, parent_markers)
                .1
        }
    };
    RootContext {
        parent_ctx,
        parent_name,
        git_root_above,
    }
}

/// Classifies the scan root itself with the markers of its entries.
pub fn classify_root(
    engine: &Engine,
    ctx: &RootContext,
    root: &Path,
    markers: u64,
) -> (Verdict, Ctx) {
    if root.parent().is_none() {
        let (verdict, mut root_ctx) = engine.root();
        root_ctx.markers = markers;
        return (verdict, root_ctx);
    }
    engine.classify_dir(&ctx.parent_ctx, &ctx.parent_name, &name_of(root), markers)
}

/// The name the engine sees for a directory: its last component, lossy ("" for "/").
pub fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Marker bits of a directory's entries from one listing, using d_type only.
fn names_only_markers(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut markers = 0;
    for entry in entries.flatten() {
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        let mut bits = marker_of(&entry.file_name().to_string_lossy(), is_dir);
        if bits & m::CACHEDIR_TAG != 0 && !is_valid_cachedir_tag(&entry.path()) {
            bits &= !m::CACHEDIR_TAG;
        }
        markers |= bits;
    }
    markers
}

impl Resolver {
    /// Tree paths are built from the canonical scan root, so anchored rules must use
    /// canonical bases too: with `HOME=/home/u` and `/home/u -> /data/u`, `~` rules would
    /// otherwise never match. Canonicalizes home and every variable that names an existing
    /// absolute path.
    pub fn canonicalized(mut self) -> Resolver {
        let canonical = |value: &str| -> Option<String> {
            let path = Path::new(value);
            if !path.is_absolute() {
                return None;
            }
            std::fs::canonicalize(path)
                .ok()?
                .into_os_string()
                .into_string()
                .ok()
        };
        if let Some(home) = canonical(&self.home) {
            self.home = home;
        }
        for value in self.vars.values_mut() {
            if let Some(c) = canonical(value) {
                *value = c;
            }
        }
        self
    }
}

impl Engine {
    /// A file rule match, telling candidates (Target) from protected files.
    pub fn match_file(&self, dir: &Ctx, name: &str) -> Option<FileMatch> {
        let rule = self.classify_file(dir, name)?;
        Some(match self.rule(rule).kind {
            Kind::Protect(_) => FileMatch::Protected(rule),
            _ => FileMatch::Target(rule),
        })
    }

    /// The rule's preferred way to clean up, as one sentence: "Or run `cargo clean`." for a
    /// command, "Prefer Xcode ▸ Settings ▸ Components." for a UI path.
    pub fn cleanup_hint(&self, rule: RuleIdx, name: &str, path: &str) -> Option<String> {
        let first = self.cleanup_for(rule, name, path).into_iter().next()?;
        Some(if first.contains('▸') {
            format!("Prefer {first}.")
        } else {
            format!("Or run `{first}`.")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::model::Os;
    use std::collections::HashMap;

    fn engine() -> Engine {
        Engine::new(&Resolver {
            os: Os::current(),
            home: "/nonexistent/home".into(),
            vars: HashMap::new(),
        })
    }

    #[test]
    fn root_context_reads_parent_markers_and_git() {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let proj = base.join("proj");
        std::fs::create_dir_all(proj.join("sub")).unwrap();
        std::fs::create_dir(proj.join(".git")).unwrap();
        std::fs::write(proj.join("Cargo.toml"), "[package]").unwrap();

        let engine = engine();
        let ctx = root_context(&engine, &proj.join("sub"));
        assert_eq!(ctx.parent_name, "proj");
        assert_eq!(
            ctx.parent_ctx.markers & (m::CARGO_TOML | m::GIT),
            m::CARGO_TOML | m::GIT
        );
        assert_eq!(ctx.git_root_above.as_deref(), Some(proj.as_path()));

        let ctx = root_context(&engine, &proj);
        assert_eq!(
            ctx.git_root_above, None,
            "the root's own .git is seen by the scan"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_filesystem_root_has_no_parent() {
        let engine = engine();
        let ctx = root_context(&engine, Path::new("/"));
        assert_eq!(ctx.parent_name, "");
        assert_eq!(
            classify_root(&engine, &ctx, Path::new("/"), 0).0,
            engine.root().0
        );
        let below = root_context(&engine, Path::new("/usr"));
        assert_eq!(below.parent_name, "");
        assert_ne!(below.parent_ctx.markers, u64::MAX);
    }

    #[cfg(unix)]
    #[test]
    fn canonicalized_resolves_a_symlinked_home() {
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap().join("real-home");
        std::fs::create_dir(&real).unwrap();
        let link = real.with_file_name("link-home");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let resolver = Resolver {
            os: Os::current(),
            home: link.to_str().unwrap().into(),
            vars: HashMap::from([
                ("CARGO_HOME".into(), link.join(".").to_str().unwrap().into()),
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("TERM".into(), "xterm".into()),
            ]),
        }
        .canonicalized();
        assert_eq!(resolver.home, real.to_str().unwrap());
        assert_eq!(resolver.vars["CARGO_HOME"], real.to_str().unwrap());
        assert_eq!(resolver.vars["PATH"], "/usr/bin:/bin");
        assert_eq!(resolver.vars["TERM"], "xterm");
    }

    #[test]
    fn protected_files_are_not_targets() {
        let windows = Engine::new(&Resolver {
            os: Os::WIN,
            home: r"C:\Users\u".into(),
            vars: HashMap::from([("SystemDrive".into(), "C:".into())]),
        });
        let (_, drive) = windows.enter("C:");
        let id = |m: Option<FileMatch>| {
            m.map(|m| match m {
                FileMatch::Target(r) | FileMatch::Protected(r) => windows.rule(r).id,
            })
        };
        let pagefile = windows.match_file(&drive, "pagefile.sys");
        assert!(
            matches!(pagefile, Some(FileMatch::Protected(_))),
            "{pagefile:?}"
        );
        assert_eq!(id(pagefile), Some("win-system-files"));
        assert_eq!(windows.match_file(&drive, "notes.txt"), None);
    }

    #[test]
    fn cleanup_hints() {
        let engine = engine();
        let rule = |id| engine.rule_by_id(id).unwrap();
        assert_eq!(
            engine.cleanup_hint(rule("mac-trash"), "x", "/x").as_deref(),
            Some("Prefer Finder ▸ Empty Trash.")
        );
        let cargo = engine
            .cleanup_hint(rule("cargo-target"), "target", "/p/target")
            .unwrap();
        assert!(
            cargo.starts_with("Or run `") && cargo.ends_with("`."),
            "{cargo}"
        );
    }
}
