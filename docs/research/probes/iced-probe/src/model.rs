//! Minimal tree model for the probe: aggregates scanner records into recursive totals and
//! flattens the expanded part of the tree into rows (done in `update`, never in `view`).
use crate::scan::DirRecord;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Sort {
    #[default]
    SizeDesc,
    SizeAsc,
    Name,
}

impl Sort {
    pub const ALL: [Sort; 3] = [Sort::SizeDesc, Sort::SizeAsc, Sort::Name];
}

impl std::fmt::Display for Sort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Sort::SizeDesc => "Size ↓ (largest first)",
            Sort::SizeAsc => "Size ↑",
            Sort::Name => "Name",
        })
    }
}

#[derive(Debug, Default)]
struct Node {
    parent: Option<PathBuf>,
    children: Vec<PathBuf>,
    total_bytes: u64,
    total_files: u64,
}

#[derive(Debug, Default)]
pub struct Tree {
    root: Option<PathBuf>,
    nodes: HashMap<PathBuf, Node>,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub path: PathBuf,
    pub name: String,
    pub depth: u16,
    pub bytes: u64,
    pub files: u64,
    pub has_children: bool,
    pub expanded: bool,
    /// Fraction of the parent's size (0..=1), for inline bars.
    pub share: f32,
}

impl Tree {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: Some(root),
            nodes: HashMap::new(),
        }
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn total(&self) -> u64 {
        self.root
            .as_ref()
            .and_then(|root| self.nodes.get(root))
            .map_or(0, |node| node.total_bytes)
    }

    pub fn apply(&mut self, records: &[DirRecord]) {
        for record in records {
            let node = self.nodes.entry(record.path.clone()).or_default();
            node.parent.clone_from(&record.parent);

            if let Some(parent) = &record.parent {
                self.nodes
                    .entry(parent.clone())
                    .or_default()
                    .children
                    .push(record.path.clone());
            }

            // Propagate this directory's own bytes to itself and all ancestors.
            let mut current = Some(record.path.clone());
            while let Some(path) = current {
                let Some(node) = self.nodes.get_mut(&path) else {
                    break;
                };
                node.total_bytes += record.own_bytes;
                node.total_files += record.own_files;
                current = node.parent.clone();
            }
        }
    }

    /// Children of `dir` as (path, bytes), largest first. Used by the treemap.
    pub fn children_by_size(&self, dir: &Path) -> Vec<(PathBuf, u64)> {
        let Some(node) = self.nodes.get(dir) else {
            return Vec::new();
        };
        let mut children: Vec<(PathBuf, u64)> = node
            .children
            .iter()
            .filter_map(|child| Some((child.clone(), self.nodes.get(child)?.total_bytes)))
            .collect();
        children.sort_by(|a, b| b.1.cmp(&a.1));
        children
    }

    #[allow(dead_code)]
    pub fn flatten(&self, expanded: &HashSet<PathBuf>, sort: Sort) -> Vec<Row> {
        self.flatten_with(expanded, false, sort)
    }

    /// `expand_all` = stress mode for the virtualized list (every directory becomes a row).
    pub fn flatten_with(&self, expanded: &HashSet<PathBuf>, expand_all: bool, sort: Sort) -> Vec<Row> {
        let mut rows = Vec::new();
        if let Some(root) = &self.root {
            self.push_rows(root, 0, 1.0, expanded, expand_all, sort, &mut rows);
        }
        rows
    }

    #[allow(clippy::too_many_arguments)]
    fn push_rows(
        &self,
        path: &Path,
        depth: u16,
        share: f32,
        expanded: &HashSet<PathBuf>,
        expand_all: bool,
        sort: Sort,
        rows: &mut Vec<Row>,
    ) {
        let Some(node) = self.nodes.get(path) else {
            return;
        };
        let is_expanded = expand_all || expanded.contains(path);

        rows.push(Row {
            path: path.to_path_buf(),
            name: display_name(path),
            depth,
            bytes: node.total_bytes,
            files: node.total_files,
            has_children: !node.children.is_empty(),
            expanded: is_expanded,
            share,
        });

        if !is_expanded {
            return;
        }

        let mut children: Vec<(&PathBuf, &Node)> = node
            .children
            .iter()
            .filter_map(|child| Some((child, self.nodes.get(child)?)))
            .collect();

        match sort {
            Sort::SizeDesc => children.sort_by(|a, b| b.1.total_bytes.cmp(&a.1.total_bytes)),
            Sort::SizeAsc => children.sort_by(|a, b| a.1.total_bytes.cmp(&b.1.total_bytes)),
            Sort::Name => children.sort_by(|a, b| a.0.cmp(b.0)),
        }

        let parent_total = node.total_bytes.max(1) as f32;
        for (child, child_node) in children {
            let share = child_node.total_bytes as f32 / parent_total;
            self.push_rows(child, depth + 1, share, expanded, expand_all, sort, rows);
        }
    }
}

pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Human readable size (base 1024 / "KiB"-style but shown as KB like Finder would be base 1000;
/// pick one and stay consistent in the real app).
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
