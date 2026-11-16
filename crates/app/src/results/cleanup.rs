//! The cleanup pane's model (UX §7, design §5.3): the zoom root's candidates grouped by
//! section and rule. Pure: built from the tree, the catalog and the git states; every section
//! decision comes from `display_section`.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use dirstat_core::catalog::model::Removal;
use dirstat_core::catalog::{
    Display, Engine, GitState, Risk, RuleIdx, Section, Verdict, Why, display_section,
};
use dirstat_core::scan::FileMatch;
use dirstat_core::tree::{CandidateRef, FileFlags, NodeFlags, NodeId, NodeState, Tree};

use crate::format;

/// Groups below this go behind "Show N smaller suggestions".
pub const SMALL_GROUP: u64 = 10_000_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub candidate: CandidateRef,
    /// "~/dev/shop/node_modules", or relative to the scan root outside home.
    pub path: String,
    pub size: u64,
    /// Not finished scanning: the size is a lower bound.
    pub growing: bool,
    /// "8 mo": since anything inside was last modified.
    pub age: Option<String>,
    /// Why git put it here, when it did.
    pub note: Option<&'static str>,
    /// Moved to the Trash: struck through until the next re-order.
    pub trashed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub section: Section,
    pub rule: RuleIdx,
    /// The folder name for a single match, else the rule's name.
    pub title: String,
    pub items: Vec<Item>,
    /// Without trashed items.
    pub total: u64,
    pub reason: &'static str,
    pub caution: Option<&'static str>,
    /// "Or run `cargo clean`." / "Prefer Xcode ▸ …".
    pub hint: Option<String>,
    /// Low: restorable by re-downloading or rebuilding (shown as a tag).
    pub redownloadable: bool,
    /// ToolOnly rules: the command to use instead of the Trash (D8).
    pub command: Option<String>,
}

impl Group {
    pub fn key(&self) -> GroupKey {
        (self.section, self.rule)
    }
}

pub type GroupKey = (Section, RuleIdx);

#[derive(Clone, Debug, PartialEq)]
pub struct SectionModel {
    pub section: Section,
    pub groups: Vec<Group>,
    /// Groups under 10 MB, behind "Show N smaller suggestions".
    pub small: Vec<Group>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CleanupModel {
    /// "In Library".
    pub scope: String,
    pub safe_total: u64,
    pub review_total: u64,
    /// Still scanning: totals are lower bounds and more will appear.
    pub scanning: bool,
    pub sections: Vec<SectionModel>,
    /// Some folders couldn't be read, so suggestions may be incomplete.
    pub incomplete: bool,
    /// One line instead of suggestions (/System/Volumes, or the whole scope is a match).
    pub info: Option<String>,
}

impl CleanupModel {
    pub fn is_empty(&self) -> bool {
        self.sections
            .iter()
            .all(|s| s.groups.is_empty() && s.small.is_empty())
    }
}

/// Candidates of `scope`, grouped. `keep_trashed` are trashed items that still show, struck
/// through, until the next re-order.
#[allow(clippy::too_many_arguments)]
pub fn build(
    tree: &Tree,
    engine: &Engine,
    git: impl Fn(CandidateRef) -> GitState,
    scope: NodeId,
    home: Option<&Path>,
    now: i64,
    scanning: bool,
    keep_trashed: &HashSet<CandidateRef>,
) -> CleanupModel {
    let mut model = CleanupModel {
        scanning,
        ..CleanupModel::default()
    };
    if !tree.is_started() || tree.get(scope).is_none() {
        return model;
    }
    let scope_node = tree.node(scope);
    model.scope = format!("In {}", scope_node.name().to_string_lossy());
    model.incomplete = scope_node.total().error_dirs > 0;
    if cfg!(target_os = "macos") && tree.path_of(scope).starts_with("/System/Volumes") {
        model.info = Some(
            "Suggestions are off under /System/Volumes. Scan / or your home folder instead.".into(),
        );
        return model;
    }
    // The whole scope is a match: nothing below it is a separate candidate.
    if let Verdict::Target { rule, .. } | Verdict::Inside(rule) = scope_node.verdict() {
        let rule = engine.rule(rule);
        model.info = Some(format!(
            "This whole folder is {}: {}",
            rule.name, rule.reason
        ));
        return model;
    }

    let mut groups: HashMap<GroupKey, Group> = HashMap::new();
    for &candidate in tree.candidates() {
        let dir = candidate.dir();
        if tree.get(dir).is_none() || !tree.is_within(dir, scope) {
            continue;
        }
        let Some(fact) = facts(tree, candidate) else {
            continue;
        };
        if fact.trashed && !keep_trashed.contains(&candidate) {
            continue;
        }
        let rule = engine.rule(fact.rule);
        let risk = rule.risk().unwrap_or(Risk::Review);
        let Display::Shown { section, why } = display_section(risk, git(candidate)) else {
            continue; // waiting for git, or tracked: not suggested
        };
        let note = match why {
            Why::Catalog => None,
            Why::GitNotIgnored => Some("not ignored by git"),
            Why::GitUnavailable => Some("couldn't check with git"),
        };
        let age = (fact.mtime != i64::MIN).then(|| {
            format::relative_compact(Duration::from_secs(
                now.saturating_sub(fact.mtime).max(0) as u64
            ))
        });
        let item = Item {
            candidate,
            path: display_path(&fact.path, tree.root_path(), home),
            size: fact.size,
            growing: fact.growing,
            age,
            note,
            trashed: fact.trashed,
        };
        let group = groups.entry((section, fact.rule)).or_insert_with(|| Group {
            section,
            rule: fact.rule,
            title: rule.name.to_string(),
            items: Vec::new(),
            total: 0,
            reason: rule.reason,
            caution: rule.caution,
            hint: None,
            redownloadable: risk == Risk::Low,
            command: None,
        });
        if !item.trashed {
            group.total += item.size;
        }
        group.items.push(item);
    }

    for group in groups.values_mut() {
        group
            .items
            .sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
        if group.items.len() == 1 {
            group.title = Path::new(&group.items[0].path)
                .file_name()
                .map_or_else(|| group.title.clone(), |n| n.to_string_lossy().into_owned());
        }
        let first = &group.items[0];
        let first_path = tree_path(tree, first.candidate);
        let (name, path) = (
            first_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            first_path.to_string_lossy().into_owned(),
        );
        if engine.rule(group.rule).removal == Removal::ToolOnly {
            group.command = engine
                .cleanup_for(group.rule, &name, &path)
                .into_iter()
                .next();
        } else {
            group.hint = engine.cleanup_hint(group.rule, &name, &path);
        }
    }

    for section in [Section::SafeToDelete, Section::ReviewFirst] {
        let mut in_section: Vec<Group> = groups
            .values()
            .filter(|g| g.section == section)
            .cloned()
            .collect();
        if in_section.is_empty() {
            continue;
        }
        in_section.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.title.cmp(&b.title)));
        let total: u64 = in_section.iter().map(|g| g.total).sum();
        match section {
            Section::SafeToDelete => model.safe_total = total,
            Section::ReviewFirst => model.review_total = total,
        }
        let (groups, small): (Vec<Group>, Vec<Group>) = in_section
            .into_iter()
            .partition(|g| g.total >= SMALL_GROUP || g.items.iter().any(|i| i.trashed));
        model.sections.push(SectionModel {
            section,
            groups,
            small,
        });
    }
    model
}

struct Facts {
    rule: RuleIdx,
    size: u64,
    mtime: i64,
    growing: bool,
    trashed: bool,
    path: std::path::PathBuf,
}

fn facts(tree: &Tree, candidate: CandidateRef) -> Option<Facts> {
    match candidate {
        CandidateRef::Dir(id) => {
            let node = tree.node(id);
            let Verdict::Target { rule, .. } = node.verdict() else {
                return None;
            };
            Some(Facts {
                rule,
                size: node.total().alloc,
                mtime: node.newest_mtime(),
                growing: node.state() != NodeState::Done,
                trashed: node.has_flag(NodeFlags::TRASHED),
                path: tree.path_of(id),
            })
        }
        CandidateRef::File(dir, index) => {
            let file = tree.node(dir).large_files().get(index as usize)?;
            let Some(FileMatch::Target(rule)) = file.matched else {
                return None;
            };
            Some(Facts {
                rule,
                size: file.alloc,
                mtime: file.mtime,
                growing: false,
                trashed: file.flags.contains(FileFlags::TRASHED),
                path: tree.path_of(dir).join(&*file.name),
            })
        }
    }
}

pub fn tree_path(tree: &Tree, candidate: CandidateRef) -> std::path::PathBuf {
    match candidate {
        CandidateRef::Dir(id) => tree.path_of(id),
        CandidateRef::File(dir, index) => {
            let name = tree
                .node(dir)
                .large_files()
                .get(index as usize)
                .map(|f| f.name.clone());
            tree.path_of(dir).join(name.as_deref().unwrap_or_default())
        }
    }
}

/// "~/…" inside home, else relative to the scan root's parent ("dev/x/target").
fn display_path(path: &Path, root: &Path, home: Option<&Path>) -> String {
    if let Some(rest) = home.and_then(|h| path.strip_prefix(h).ok()) {
        return format!("~/{}", rest.display());
    }
    let base = root.parent().unwrap_or(root);
    path.strip_prefix(base)
        .map_or_else(|_| path.display().to_string(), |p| p.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dirstat_core::catalog::Resolver;
    use dirstat_core::git::GitStatus;
    use dirstat_core::scan::ErrorClass;
    use dirstat_core::tree::testing::{Dir, TreeBuilder, dir};
    use std::sync::OnceLock;

    fn engine() -> &'static Engine {
        static ENGINE: OnceLock<Engine> = OnceLock::new();
        ENGINE.get_or_init(|| Engine::new(&Resolver::from_env()))
    }

    fn target(id: &str) -> Verdict {
        Verdict::Target {
            rule: engine().rule_by_id(id).unwrap(),
            by_default: false,
        }
    }

    fn sample() -> Tree {
        let proj = |name: &str, target_mb: u64, modules_mb: u64| -> Dir {
            dir(name)
                .child(
                    dir("target")
                        .verdict(target("cargo-target"))
                        .small_files(1, target_mb * 1_000_000)
                        .mtime(0),
                )
                .child(
                    dir("node_modules")
                        .verdict(target("node-modules"))
                        .small_files(1, modules_mb * 1_000_000),
                )
        };
        TreeBuilder::new(
            "/Users/u/dev",
            dir("dev")
                .child(proj("a", 500, 40))
                .child(proj("b", 200, 30))
                .child(
                    dir("tiny").child(
                        dir("target")
                            .verdict(target("cargo-target"))
                            .small_files(1, 2_000_000),
                    ),
                ),
        )
        .build()
    }

    fn model(tree: &Tree, git: impl Fn(CandidateRef) -> GitState) -> CleanupModel {
        build(
            tree,
            engine(),
            git,
            NodeId::ROOT,
            Some(Path::new("/Users/u")),
            30 * 86_400 * 8,
            false,
            &HashSet::new(),
        )
    }

    #[test]
    fn sections_totals_and_tags() {
        let tree = sample();
        let m = model(&tree, |_| GitState::NotInRepo);
        assert_eq!(m.scope, "In dev");
        let safe = &m.sections[0];
        assert_eq!(safe.section, Section::SafeToDelete);
        assert_eq!(safe.groups[0].title, "Cargo target");
        assert_eq!(safe.groups[0].items.len(), 3, "a, b and tiny");
        assert_eq!(safe.groups[0].items[0].path, "~/dev/a/target");
        assert_eq!(safe.groups[0].items[0].age.as_deref(), Some("8 mo"));
        let review = &m.sections[1];
        assert_eq!(
            review.section,
            Section::ReviewFirst,
            "Low goes under Review first"
        );
        assert!(review.groups[0].redownloadable);
        assert_eq!(review.groups[0].title, "node_modules");
        // The stat block counts Safe only.
        let safe_sizes: u64 = safe
            .groups
            .iter()
            .flat_map(|g| &g.items)
            .map(|i| i.size)
            .sum();
        assert_eq!(m.safe_total, safe_sizes);
        assert!(m.review_total > 0 && m.review_total < m.safe_total);
        assert!(
            safe.groups[0]
                .hint
                .as_deref()
                .is_some_and(|h| h.contains("cargo clean"))
        );
    }

    #[test]
    fn pending_and_tracked_are_absent_and_unavailable_goes_to_review() {
        let tree = sample();
        let a_target =
            CandidateRef::Dir(tree.find_path(Path::new("/Users/u/dev/a/target")).unwrap());
        let b_target =
            CandidateRef::Dir(tree.find_path(Path::new("/Users/u/dev/b/target")).unwrap());
        let m = model(&tree, |c| match c {
            c if c == a_target => GitState::Pending,
            c if c == b_target => GitState::Known(GitStatus::Tracked),
            _ => GitState::Known(GitStatus::Unavailable),
        });
        let all: Vec<&Item> = m
            .sections
            .iter()
            .flat_map(|s| s.groups.iter().chain(&s.small))
            .flat_map(|g| &g.items)
            .collect();
        assert!(
            !all.iter()
                .any(|i| i.candidate == a_target || i.candidate == b_target)
        );
        assert!(
            m.sections.iter().all(|s| s.section == Section::ReviewFirst),
            "unchecked repos: Review first"
        );
        assert!(
            all.iter()
                .all(|i| i.note == Some("couldn't check with git"))
        );
        assert_eq!(m.safe_total, 0);
    }

    #[test]
    fn small_groups_fold_and_single_matches_use_the_folder_name() {
        let tree = TreeBuilder::new(
            "/Users/u/x",
            dir("x").child(
                dir("p").child(
                    dir("target")
                        .verdict(target("cargo-target"))
                        .small_files(1, 3_000_000),
                ),
            ),
        )
        .build();
        let m = model(&tree, |_| GitState::NotInRepo);
        assert!(m.sections[0].groups.is_empty());
        assert_eq!(
            m.sections[0].small[0].title, "target",
            "a single match is named by its folder"
        );
    }

    #[test]
    fn states() {
        let tree = TreeBuilder::new(
            "/Users/u/x",
            dir("x").child(dir("bad").error(ErrorClass::PermissionDenied)),
        )
        .build();
        let m = model(&tree, |_| GitState::NotInRepo);
        assert!(m.is_empty() && m.incomplete);
        let whole = TreeBuilder::new(
            "/Users/u/target",
            dir("target").verdict(target("cargo-target")),
        )
        .build();
        assert!(
            model(&whole, |_| GitState::NotInRepo)
                .info
                .unwrap()
                .starts_with("This whole folder is Cargo target")
        );
    }

    #[test]
    fn trashed_items_stay_struck_until_released() {
        let mut tree = sample();
        let a = tree.find_path(Path::new("/Users/u/dev/a/target")).unwrap();
        tree.mark_trashed(a);
        let keep = HashSet::from([CandidateRef::Dir(a)]);
        let m = build(
            &tree,
            engine(),
            |_| GitState::NotInRepo,
            NodeId::ROOT,
            None,
            0,
            false,
            &keep,
        );
        let item = m.sections[0].groups[0]
            .items
            .iter()
            .find(|i| i.candidate == CandidateRef::Dir(a))
            .unwrap();
        assert!(item.trashed);
        assert_eq!(
            m.sections[0].groups[0].total,
            200_004_096 + 2_004_096,
            "not counted any more"
        );
        let gone = build(
            &tree,
            engine(),
            |_| GitState::NotInRepo,
            NodeId::ROOT,
            None,
            0,
            false,
            &HashSet::new(),
        );
        assert!(
            !gone.sections[0].groups[0]
                .items
                .iter()
                .any(|i| i.candidate == CandidateRef::Dir(a))
        );
    }
}
