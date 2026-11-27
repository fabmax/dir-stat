//! Space by kind of content: the segmented bar under the path bar.
//!
//! Every byte of a subtree goes to exactly one [`Kind`]. A folder's own bytes take the kind
//! of the folder. That kind comes from, strongest first:
//!
//! 1. the catalog verdict of a cleanup match (`Target`, `Inside`, `Container`): its category
//!    (a `node_modules` inside `~/Documents` is Developer, not Documents);
//! 2. a well-known location ([`Places`]: `~/Documents`, `/Applications`, `/opt`, ...);
//! 3. a well-known name (`steamapps`, `Epic Games`, ...);
//! 4. a catalog `Protect` rule that names a kind (app bundles, OS folders);
//! 5. otherwise the parent's kind.
//!
//! Large files matched by a catalog file rule (installers, ...) take that rule's category.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::PathBuf;

use crate::catalog::Engine;
use crate::catalog::engine::Resolver;
use crate::catalog::model::{Base, Category, Kind as RuleKind, Os};
use crate::catalog::{RuleIdx, Verdict};
use crate::scan::FileMatch;
use crate::tree::{NodeFlags, NodeId, Tree};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Documents,
    Applications,
    Games,
    Developer,
    Caches,
    Logs,
    Trash,
    System,
    /// Everything else.
    Other,
}

impl Kind {
    pub const COUNT: usize = 9;
    /// Bar and legend order. It is also the categorical color order (UX §11.4).
    pub const ALL: [Kind; Kind::COUNT] = [
        Kind::Documents,
        Kind::Applications,
        Kind::Games,
        Kind::Developer,
        Kind::Caches,
        Kind::Logs,
        Kind::Trash,
        Kind::System,
        Kind::Other,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn label(self) -> &'static str {
        match self {
            Kind::Documents => "Documents",
            Kind::Applications => "Applications",
            Kind::Games => "Games",
            Kind::Developer => "Developer",
            Kind::Caches => "Caches",
            Kind::Logs => "Logs",
            Kind::Trash => "Trash",
            Kind::System => "System",
            Kind::Other => "Other",
        }
    }

    /// One line for the tooltip: what counts as this kind.
    pub const fn hint(self) -> &'static str {
        match self {
            Kind::Documents => "Desktop, Documents, Downloads, media folders, mail",
            Kind::Applications => "App bundles and installed software",
            Kind::Games => "Steam, Epic and GOG game libraries",
            Kind::Developer => "Build output, dependencies, SDKs, VMs and containers",
            Kind::Caches => "App, package-manager and temporary caches",
            Kind::Logs => "Log files",
            Kind::Trash => "Items in the Trash",
            Kind::System => "Operating system files",
            Kind::Other => "Everything not recognized",
        }
    }

    /// The kind of a cleanup match. `None` for `Protected`, which says nothing on its own.
    pub const fn of_category(c: Category) -> Option<Kind> {
        Some(match c {
            Category::Cache | Category::PackageCache | Category::Temporary => Kind::Caches,
            Category::BuildArtifacts
            | Category::Dependencies
            | Category::DeveloperTools
            | Category::VirtualMachines
            | Category::Models => Kind::Developer,
            Category::Logs => Kind::Logs,
            Category::Trash => Kind::Trash,
            Category::Installers => Kind::Documents,
            Category::SystemUpdates => Kind::System,
            Category::Backups => Kind::Other,
            Category::Protected => return None,
        })
    }

    /// The kind a `Protect` rule implies for everything below it, by rule id.
    fn of_protect(id: &str) -> Option<Kind> {
        Some(match id {
            "app-bundles" => Kind::Applications,
            "mac-system"
            | "linux-system"
            | "win-system"
            | "win-winsxs"
            | "win-installer-cache"
            | "win-system-files" => Kind::System,
            "media-libraries" | "mail-messages" | "icloud-drive" => Kind::Documents,
            "homebrew-prefix" | "python-tools" | "node-global-packages" => Kind::Developer,
            _ => return None,
        })
    }

    /// Game libraries recognized by folder name, wherever they are.
    fn of_name(name: &OsStr) -> Option<Kind> {
        let name = name.to_str()?;
        matches!(
            name,
            "steamapps" | "SteamLibrary" | "Epic Games" | "GOG Games" | "GOG Galaxy"
        )
        .then_some(Kind::Games)
    }
}

/// Well-known locations, resolved once from the environment.
#[derive(Clone, Debug, Default)]
pub struct Places {
    /// Absolute path and its kind. Longer paths win over their prefixes.
    anchors: Vec<(PathBuf, Kind)>,
}

impl Places {
    pub fn new(r: &Resolver) -> Places {
        use Kind::*;
        let mut anchors = Vec::new();
        let mut at = |base: Base, rel: &[&str], kind: Kind| {
            if let Some(b) = r.base(base) {
                let mut p = PathBuf::from(b);
                p.extend(rel);
                anchors.push((p, kind));
            }
        };
        // Personal folders. Media folders count as Documents, like macOS's own breakdown.
        for name in [
            "Desktop",
            "Documents",
            "Downloads",
            "Pictures",
            "Movies",
            "Music",
            "Videos",
        ] {
            at(Base::Home, &[name], Documents);
        }
        at(Base::Home, &["Games"], Games);
        at(Base::Home, &["Applications"], Applications);
        if r.os.has(Os::MAC) {
            at(Base::Root, &["Applications"], Applications);
        }
        if r.os.has(Os::LINUX) {
            for rel in [
                &["opt"][..],
                &["snap"],
                &["var", "lib", "snapd"],
                &["var", "lib", "flatpak"],
                &["usr", "local"],
            ] {
                at(Base::Root, rel, Applications);
            }
            at(Base::XdgData, &["flatpak"], Applications);
            at(Base::XdgData, &["Trash"], Trash);
            at(Base::Root, &["var", "log"], Logs);
        }
        if r.os.has(Os::WIN) {
            at(Base::SystemDrive, &["Program Files"], Applications);
            at(Base::SystemDrive, &["Program Files (x86)"], Applications);
        }
        anchors.sort_by_key(|(p, _)| p.components().count());
        Places { anchors }
    }

    #[cfg(test)]
    fn with(anchors: Vec<(PathBuf, Kind)>) -> Places {
        Places { anchors }
    }

    /// The anchors at or below the scan root, by node, and the kind the root inherits from
    /// an anchor above it (scanning `~/Documents/thesis` is all Documents).
    fn resolve(&self, tree: &Tree) -> (HashMap<NodeId, Kind>, Kind) {
        let root = tree.root_path();
        let mut nodes = HashMap::new();
        let mut above = Kind::Other;
        for (path, kind) in &self.anchors {
            if root.starts_with(path) {
                above = *kind;
            } else if let Some(id) = tree.find_path(path) {
                nodes.insert(id, *kind);
            }
        }
        (nodes, above)
    }
}

/// Bytes per kind (allocated, hard links once, like the Size column).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Breakdown {
    pub bytes: [u64; Kind::COUNT],
}

impl Breakdown {
    pub fn get(&self, kind: Kind) -> u64 {
        self.bytes[kind.index()]
    }
    pub fn total(&self) -> u64 {
        self.bytes.iter().sum()
    }
    /// Non-empty kinds in bar order.
    pub fn parts(&self) -> impl Iterator<Item = (Kind, u64)> + '_ {
        Kind::ALL
            .into_iter()
            .map(|k| (k, self.get(k)))
            .filter(|&(_, b)| b > 0)
    }
}

struct Classifier<'a> {
    tree: &'a Tree,
    engine: &'a Engine,
    anchors: HashMap<NodeId, Kind>,
}

impl Classifier<'_> {
    fn rule_kind(&self, rule: RuleIdx) -> Option<Kind> {
        Kind::of_category(self.engine.rule(rule).category)
    }

    fn kind_of(&self, id: NodeId, inherited: Kind) -> Kind {
        let node = self.tree.node(id);
        match node.verdict() {
            Verdict::Target { rule, .. } | Verdict::Inside(rule) | Verdict::Container(rule) => {
                if let Some(k) = self.rule_kind(rule) {
                    return k;
                }
            }
            _ => {}
        }
        if let Some(&k) = self.anchors.get(&id) {
            return k;
        }
        if let Some(k) = Kind::of_name(node.name()) {
            return k;
        }
        if let Verdict::Protected(rule) = node.verdict() {
            let rule = self.engine.rule(rule);
            if matches!(rule.kind, RuleKind::Protect(_))
                && let Some(k) = Kind::of_protect(rule.id)
            {
                return k;
            }
        }
        inherited
    }
}

/// The breakdown of `zoom`'s subtree. Visits every node below `zoom` once.
pub fn compute(tree: &Tree, engine: &Engine, places: &Places, zoom: NodeId) -> Breakdown {
    let mut out = Breakdown::default();
    if !tree.is_started() || tree.get(zoom).is_none() {
        return out;
    }
    let (anchors, above) = places.resolve(tree);
    let c = Classifier {
        tree,
        engine,
        anchors,
    };
    // The zoom root inherits along its ancestors, from the scan root down.
    let mut chain: Vec<NodeId> = tree.ancestors(zoom).collect();
    chain.reverse();
    let inherited = chain.into_iter().fold(above, |k, id| c.kind_of(id, k));

    let mut stack = vec![(zoom, inherited)];
    while let Some((id, parent_kind)) = stack.pop() {
        let node = tree.node(id);
        let kind = c.kind_of(id, parent_kind);
        let mut own = node.total().alloc;
        for child in tree.children(id) {
            let n = tree.node(child);
            // A trashed folder keeps its numbers but is already out of its ancestors'.
            if n.has_flag(NodeFlags::TRASHED) {
                continue;
            }
            own = own.saturating_sub(n.total().alloc);
            stack.push((child, kind));
        }
        for f in node.large_files() {
            if f.flags.contains(crate::tree::FileFlags::TRASHED) {
                continue;
            }
            if let Some(FileMatch::Target(rule)) = f.matched
                && let Some(k) = c.rule_kind(rule)
                && k != kind
            {
                let moved = f.alloc.min(own);
                own -= moved;
                out.bytes[k.index()] += moved;
            }
        }
        out.bytes[kind.index()] += own;
    }
    out
}

#[cfg(test)]
mod tests;
