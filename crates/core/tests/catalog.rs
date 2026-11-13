//! Catalog and git verification on fixture trees (design §10.2). Every scan passes
//! `RootContext::detached`: on macOS $TMPDIR is /private/var/folders/…, a zone where the
//! catalog applies no heuristics, so the real root context would yield no candidates.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

use dirstat_core::catalog::{
    self, Display, Engine, GitState, Resolver, RootContext, Section, Verdict, display_section,
};
use dirstat_core::git::{self, GitStatus};
use dirstat_core::scan::{self, NodeId, Options, ScanRequest, Update};
use dirstat_core::tree::{CandidateRef, Tree};

fn engine() -> Arc<Engine> {
    static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();
    ENGINE
        .get_or_init(|| Arc::new(Engine::new(&Resolver::from_env().canonicalized())))
        .clone()
}

fn scan_tree(root: &Path) -> Tree {
    let engine = engine();
    let req = ScanRequest {
        context: Some(RootContext::detached(&engine)),
        ..ScanRequest::new(root)
    };
    let opts = Options {
        threads: 3,
        batch_interval: Duration::from_millis(5),
        ..Options::new(engine)
    };
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = scan::start(req, opts, tx);
    let mut tree = Tree::opening(root.to_path_buf());
    loop {
        match rx.recv().unwrap() {
            Update::Started(info) => tree.start(info),
            Update::Batch(batch) => batch.listings.into_iter().for_each(|l| {
                tree.apply(l);
            }),
            Update::Finished(summary) => {
                tree.finish(&summary);
                break;
            }
            Update::Failed(e) => panic!("{}", e.message),
        }
    }
    handle.join();
    tree
}

fn fixture() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    (dir, root)
}

fn put(path: &Path, content: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// A Rust project whose `target` carries the real cache-directory markers.
fn rust_project(dir: &Path, valid_tag: bool) {
    put(&dir.join("Cargo.toml"), b"[package]\nname = \"x\"\n");
    let tag: &[u8] = if valid_tag {
        catalog::model::CACHEDIR_SIGNATURE
    } else {
        b"Signature: not the real one"
    };
    put(&dir.join("target/CACHEDIR.TAG"), tag);
    put(&dir.join("target/.rustc_info.json"), b"{}");
    put(
        &dir.join("target/debug/deps/libx.rlib"),
        &vec![0u8; 2 << 20],
    );
}

fn rule_id(tree: &Tree, id: NodeId) -> Option<&'static str> {
    match tree.node(id).verdict() {
        Verdict::Target { rule, .. } => Some(engine().rule(rule).id),
        _ => None,
    }
}

fn candidate_paths(tree: &Tree, root: &Path) -> Vec<String> {
    let mut paths: Vec<String> = tree
        .candidates()
        .iter()
        .map(|c| {
            tree.path_of(c.dir())
                .strip_prefix(root)
                .unwrap()
                .display()
                .to_string()
        })
        .collect();
    paths.sort();
    paths
}

#[test]
fn cargo_targets_are_safe_and_node_modules_review_first() {
    let (_dir, root) = fixture();
    rust_project(&root.join("proj"), true);
    // No Cargo.toml next to it: only the markers inside count, and the tag is invalid.
    rust_project(&root.join("fake"), false);
    fs::remove_file(root.join("fake/Cargo.toml")).unwrap();
    // The same without Cargo.toml but with a valid tag does match.
    rust_project(&root.join("tagged"), true);
    fs::remove_file(root.join("tagged/Cargo.toml")).unwrap();
    put(&root.join("proj/package.json"), b"{}");
    put(
        &root.join("proj/node_modules/left-pad/index.js"),
        b"module.exports = 1;",
    );
    put(&root.join("loose/node_modules/x.js"), b"");

    let tree = scan_tree(&root);
    // Not `fake/target` (invalid tag), not `loose/node_modules` (no package.json next to it),
    // nothing inside a candidate.
    assert_eq!(
        candidate_paths(&tree, &root),
        ["proj/node_modules", "proj/target", "tagged/target"]
    );

    let target = tree.find_path(&root.join("proj/target")).unwrap();
    assert_eq!(rule_id(&tree, target), Some("cargo-target"));
    let deps = tree
        .find_path(&root.join("proj/target/debug/deps"))
        .unwrap();
    assert!(matches!(tree.node(deps).verdict(), Verdict::Inside(_)));
    assert!(tree.node(deps).large_files()[0].matched.is_none());

    let section = |id| {
        let Verdict::Target { rule, .. } = tree.node(id).verdict() else {
            panic!()
        };
        display_section(engine().rule(rule).risk().unwrap(), GitState::NotInRepo)
    };
    let node_modules = tree.find_path(&root.join("proj/node_modules")).unwrap();
    assert_eq!(rule_id(&tree, node_modules), Some("node-modules"));
    assert!(matches!(
        section(target),
        Display::Shown {
            section: Section::SafeToDelete,
            ..
        }
    ));
    assert!(matches!(
        section(node_modules),
        Display::Shown {
            section: Section::ReviewFirst,
            ..
        }
    ));
}

#[test]
fn context_for_reproduces_the_scan_time_verdicts() {
    let (_dir, root) = fixture();
    rust_project(&root.join("ws/a"), true);
    put(&root.join("ws/b/package.json"), b"{}");
    put(&root.join("ws/b/node_modules/m/index.js"), b"");
    put(&root.join("ws/c/.venv/pyvenv.cfg"), b"home = /usr");
    put(&root.join("ws/c/__pycache__/x.pyc"), b"");
    let tree = scan_tree(&root);
    assert!(
        tree.candidates().len() >= 3,
        "{:?}",
        candidate_paths(&tree, &root)
    );

    let engine = engine();
    let mut checked = 0;
    for id in (1..tree.len()).map(NodeId) {
        let node = tree.node(id);
        let ctx = tree.context_for(&engine, id);
        let own = node.name().to_string_lossy();
        let (verdict, _) =
            engine.classify_dir(&ctx.parent_ctx, &ctx.parent_name, &own, node.markers());
        assert_eq!(verdict, node.verdict(), "{}", tree.path_of(id).display());
        checked += 1;
    }
    assert!(checked > 10);
}

fn git_cmd(git: &git::GitBinary, dir: &Path, args: &[&str]) {
    let status = Command::new(&git.0)
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn git_verification() {
    let Some(git) = git::find_git() else {
        eprintln!("git not available, skipping");
        return;
    };
    let (_dir, root) = fixture();
    let repo = root.join("repo");
    put(&repo.join(".gitignore"), b"/target/\n/b/target/\n");
    rust_project(&repo, true); // ignored, nothing tracked: Clean
    rust_project(&repo.join("b"), true);
    put(&repo.join("b/target/tracked.txt"), b"committed by mistake");
    put(&repo.join("web/package.json"), b"{}"); // node_modules not ignored: NotIgnored
    put(&repo.join("web/node_modules/m/index.js"), b"");
    rust_project(&root.join("outside"), true); // no work tree: no check needed
    git_cmd(&git, &repo, &["init", "-q"]);
    git_cmd(&git, &repo, &["add", "-f", "b/target/tracked.txt"]);

    let tree = scan_tree(&root);
    assert_eq!(
        candidate_paths(&tree, &root),
        [
            "outside/target",
            "repo/b/target",
            "repo/target",
            "repo/web/node_modules"
        ]
    );
    let (outside, jobs) = git::plan_jobs(&tree, tree.candidates().iter().copied());
    assert_eq!(outside.len(), 1);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].worktree, repo);

    let mut statuses: Vec<(String, GitStatus)> = git::verify(&git, &jobs[0])
        .into_iter()
        .map(|(c, status)| {
            let CandidateRef::Dir(id) = c else { panic!() };
            (
                tree.path_of(id)
                    .strip_prefix(&repo)
                    .unwrap()
                    .display()
                    .to_string(),
                status,
            )
        })
        .collect();
    statuses.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        statuses,
        [
            ("b/target".into(), GitStatus::Tracked),
            ("target".into(), GitStatus::Clean),
            ("web/node_modules".into(), GitStatus::NotIgnored),
        ]
    );
    // The check is read-only: no index lock was left behind.
    assert!(!repo.join(".git/index.lock").exists());
}

#[test]
fn git_failures_are_unavailable() {
    let (_dir, root) = fixture();
    rust_project(&root.join("proj"), true);
    let tree = scan_tree(&root);
    let target = CandidateRef::Dir(tree.find_path(&root.join("proj/target")).unwrap());
    let job = git::GitJob {
        worktree: root.join("proj"),
        items: vec![(target, "target".into())],
    };
    let missing = git::GitBinary("/nonexistent/git".into());
    assert_eq!(
        git::verify(&missing, &job),
        [(target, GitStatus::Unavailable)]
    );
    if let Some(git) = git::find_git() {
        // Not a repository: git exits with 128.
        assert_eq!(git::verify(&git, &job), [(target, GitStatus::Unavailable)]);
    }
}
