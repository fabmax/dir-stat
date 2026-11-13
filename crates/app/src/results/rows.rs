//! The flattened row list and its virtualization (design §6.6). Rows are keys only; values
//! are read live from the tree when visible rows are drawn.

use std::collections::{HashMap, HashSet};

use dirstat_core::tree::{NodeFlags, NodeId, NodeState, Tree};

use super::order::{ChildKey, ChildOrder, split_files};

pub const ROW_H: f32 = 28.0;
const OVERSCAN: usize = 6;
/// Kept files listed per expanded aggregate row.
pub const FILE_BATCH: usize = 50;

/// What a row shows. Stable across re-orders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RowKey {
    Dir(NodeId),
    /// A folder and an index into its large files.
    File(NodeId, u32),
    /// "≡ 1,234 files".
    Aggregate(NodeId),
    /// "and 1,190 smaller files".
    MoreFiles(NodeId),
    /// "Waiting to be scanned…" / "This folder is empty."
    Placeholder(NodeId),
}

impl RowKey {
    /// The folder the row belongs to (itself for folder rows).
    pub fn dir(self) -> NodeId {
        match self {
            RowKey::Dir(id)
            | RowKey::File(id, _)
            | RowKey::Aggregate(id)
            | RowKey::MoreFiles(id)
            | RowKey::Placeholder(id) => id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub key: RowKey,
    pub depth: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub offset_y: f32,
    pub height: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            offset_y: 0.0,
            height: 700.0,
        }
    }
}

impl Viewport {
    pub fn max_offset(&self, rows: usize) -> f32 {
        (rows as f32 * ROW_H - self.height).max(0.0)
    }
}

/// Rows `first..last` to materialize, with overscan. Valid for any offset, also one left
/// over from a longer list.
pub fn range(offset: f32, height: f32, len: usize) -> (usize, usize) {
    let offset = offset.clamp(0.0, (len as f32 * ROW_H - height).max(0.0));
    let first = ((offset / ROW_H).floor() as usize)
        .saturating_sub(OVERSCAN)
        .min(len);
    let count = (height / ROW_H).ceil() as usize + 2 * OVERSCAN;
    (first, (first + count).min(len))
}

/// Whether a folder row has a disclosure chevron.
pub fn expandable(tree: &Tree, id: NodeId) -> bool {
    let node = tree.node(id);
    match node.state() {
        NodeState::Pending => true,
        // Empty and unreadable folders have nothing to show.
        NodeState::Scanning | NodeState::Done => node.child_count() > 0 || node.files().count > 0,
        NodeState::Skipped(_) | NodeState::Removed => false,
    }
}

/// Depth-first over expanded folders, each in its cached order.
pub fn build(
    tree: &Tree,
    zoom: NodeId,
    orders: &HashMap<NodeId, ChildOrder>,
    open_aggregates: &HashSet<NodeId>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    if tree.get(zoom).is_some() {
        push_children(tree, zoom, 0, orders, open_aggregates, &mut rows);
    }
    rows
}

fn push_children(
    tree: &Tree,
    dir: NodeId,
    depth: u16,
    orders: &HashMap<NodeId, ChildOrder>,
    open_aggregates: &HashSet<NodeId>,
    rows: &mut Vec<Row>,
) {
    let node = tree.node(dir);
    if node.state() == NodeState::Pending {
        rows.push(Row {
            key: RowKey::Placeholder(dir),
            depth,
        });
        return;
    }
    let Some(order) = orders.get(&dir) else {
        return;
    };
    if order.keys.is_empty() {
        rows.push(Row {
            key: RowKey::Placeholder(dir),
            depth,
        });
        return;
    }
    for &key in &order.keys {
        match key {
            ChildKey::Dir(child) => {
                rows.push(Row {
                    key: RowKey::Dir(child),
                    depth,
                });
                if tree.node(child).has_flag(NodeFlags::EXPANDED) && expandable(tree, child) {
                    push_children(tree, child, depth + 1, orders, open_aggregates, rows);
                }
            }
            ChildKey::File(index) => rows.push(Row {
                key: RowKey::File(dir, index),
                depth,
            }),
            ChildKey::Aggregate => {
                rows.push(Row {
                    key: RowKey::Aggregate(dir),
                    depth,
                });
                if open_aggregates.contains(&dir) {
                    let split = split_files(node);
                    for &index in split.kept_rest.iter().take(FILE_BATCH) {
                        rows.push(Row {
                            key: RowKey::File(dir, index),
                            depth: depth + 1,
                        });
                    }
                    let listed = split.kept_rest.len().min(FILE_BATCH) as u64;
                    if split.aggregate_count > listed {
                        rows.push(Row {
                            key: RowKey::MoreFiles(dir),
                            depth: depth + 1,
                        });
                    }
                }
            }
        }
    }
}

/// The row to keep still across a rebuild: the selection if visible, else the first fully
/// visible row (none at the very top); with its y inside the viewport.
pub fn anchor(
    rows: &[Row],
    viewport: Viewport,
    selection: Option<RowKey>,
) -> Option<(RowKey, f32)> {
    let visible = |index: usize| {
        let y = index as f32 * ROW_H - viewport.offset_y;
        (y >= 0.0 && y + ROW_H <= viewport.height).then_some(y)
    };
    if let Some(selected) = selection
        && let Some(index) = rows.iter().position(|r| r.key == selected)
        && let Some(y) = visible(index)
    {
        return Some((selected, y));
    }
    // At the very top, stay there: the biggest rows are what one wants to see.
    if viewport.offset_y < 0.5 {
        return None;
    }
    let first = (viewport.offset_y / ROW_H).ceil() as usize;
    rows.get(first)
        .map(|r| (r.key, first as f32 * ROW_H - viewport.offset_y))
}

/// The offset that puts the anchor (or its nearest shown ancestor) back at its y.
pub fn restore(tree: &Tree, rows: &[Row], viewport: Viewport, anchor: (RowKey, f32)) -> f32 {
    let (key, y) = anchor;
    let index = rows.iter().position(|r| r.key == key).or_else(|| {
        let dir = key.dir();
        let start = if matches!(key, RowKey::Dir(_)) {
            tree.ancestors(dir).collect::<Vec<_>>()
        } else {
            std::iter::once(dir).chain(tree.ancestors(dir)).collect()
        };
        start
            .into_iter()
            .find_map(|a| rows.iter().position(|r| r.key == RowKey::Dir(a)))
    });
    match index {
        Some(index) => (index as f32 * ROW_H - y).clamp(0.0, viewport.max_offset(rows.len())),
        None => viewport
            .offset_y
            .clamp(0.0, viewport.max_offset(rows.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::results::order::{Sort, exact};
    use dirstat_core::tree::testing::{TreeBuilder, dir};

    fn orders_for(tree: &Tree, ids: &[NodeId]) -> HashMap<NodeId, ChildOrder> {
        ids.iter()
            .map(|&id| (id, exact(tree, id, Sort::default())))
            .collect()
    }

    #[test]
    fn range_after_the_rows_shrink() {
        assert_eq!(range(0.0, 280.0, 100), (0, 22));
        assert_eq!(range(28.0 * 50.0, 280.0, 100), (44, 66));
        // A stale offset from a long list must not produce an empty slice of a short one.
        assert_eq!(range(28.0 * 5000.0, 280.0, 3), (0, 3));
        assert_eq!(range(28.0 * 90.0, 280.0, 100), (84, 100));
        assert_eq!(range(0.0, 280.0, 0), (0, 0));
    }

    #[test]
    fn expanded_folders_placeholders_and_aggregates() {
        let builder = TreeBuilder::new(
            "/r",
            dir("r")
                .child(dir("a").small_files(2, 100).child(dir("a1")))
                .child(dir("b"))
                .child(dir("m").skip(dirstat_core::scan::Skip::MountPoint)),
        );
        let mut tree = builder.build();
        let (a, a1, b) = (NodeId(1), NodeId(4), NodeId(2));
        tree.set_flag(a, NodeFlags::EXPANDED, true);
        tree.set_flag(a1, NodeFlags::EXPANDED, true);
        tree.set_flag(b, NodeFlags::EXPANDED, true);
        let orders = orders_for(&tree, &[NodeId::ROOT, a, a1, b]);
        let rows = build(&tree, NodeId::ROOT, &orders, &HashSet::from([a]));
        let keys: Vec<(RowKey, u16)> = rows.iter().map(|r| (r.key, r.depth)).collect();
        assert_eq!(
            keys,
            [
                (RowKey::Dir(a), 0),
                (RowKey::Dir(a1), 1),
                (RowKey::Aggregate(a), 1),
                (RowKey::MoreFiles(a), 2),
                (RowKey::Dir(b), 0),
                (RowKey::Dir(NodeId(3)), 0),
            ]
        );
        assert!(
            !expandable(&tree, NodeId(3)),
            "mount points have no chevron"
        );
        assert!(
            !expandable(&tree, a1) && !expandable(&tree, b),
            "empty folders neither"
        );
    }

    #[test]
    fn the_anchor_stays_put_when_listings_arrive_above_it() {
        // `a-top` is expanded but pending (a placeholder row); 40 siblings follow. All are pending
        // with size 0, so names decide the order.
        let mut root = dir("r").child(
            dir("a-top")
                .child(dir("t1"))
                .child(dir("t2"))
                .child(dir("t3")),
        );
        for i in 0..40 {
            root = root.child(dir(format!("s{i:02}")).small_files(1, 1000 - i));
        }
        let builder = TreeBuilder::new("/r", root);
        let mut tree = builder.started();
        let mut listings = builder.listings().into_iter();
        tree.apply(listings.next().unwrap()); // root only
        let top = NodeId(1);
        tree.set_flag(top, NodeFlags::EXPANDED, true);
        let mut orders = orders_for(&tree, &[NodeId::ROOT]);
        let rows = build(&tree, NodeId::ROOT, &orders, &HashSet::new());
        let viewport = Viewport {
            offset_y: 10.0 * ROW_H,
            height: 10.0 * ROW_H,
        };
        let anchor = anchor(&rows, viewport, None).unwrap();
        assert_eq!(anchor.1, 0.0);

        // `top`'s listing arrives: the placeholder turns into three rows above the viewport.
        let top_listing = listings.find(|l| l.id == top).unwrap();
        tree.apply(top_listing);
        orders.insert(top, exact(&tree, top, Sort::default()));
        let rebuilt = build(&tree, NodeId::ROOT, &orders, &HashSet::new());
        assert_eq!(rebuilt.len(), rows.len() + 2);
        let offset = restore(&tree, &rebuilt, viewport, anchor);
        assert_eq!(offset, 12.0 * ROW_H, "the anchored row keeps its y");
        let index = rebuilt.iter().position(|r| r.key == anchor.0).unwrap();
        assert_eq!(index as f32 * ROW_H - offset, anchor.1);
    }

    #[test]
    fn collapsing_a_long_list_never_leaves_a_blank_tree() {
        let mut big = dir("big");
        for i in 0..300 {
            big = big.child(dir(format!("c{i}")));
        }
        let mut tree = TreeBuilder::new("/r", dir("r").child(big).child(dir("other"))).build();
        let big = NodeId(1);
        tree.set_flag(big, NodeFlags::EXPANDED, true);
        let orders = orders_for(&tree, &[NodeId::ROOT, big]);
        let long = build(&tree, NodeId::ROOT, &orders, &HashSet::new());
        let viewport = Viewport {
            offset_y: 250.0 * ROW_H,
            height: 20.0 * ROW_H,
        };
        let anchor = anchor(&long, viewport, Some(RowKey::Dir(NodeId(260)))).unwrap();

        tree.set_flag(big, NodeFlags::EXPANDED, false);
        let short = build(&tree, NodeId::ROOT, &orders, &HashSet::new());
        let offset = restore(&tree, &short, viewport, anchor);
        assert_eq!(offset, 0.0, "clamped to the short list");
        let (first, last) = range(offset, viewport.height, short.len());
        assert_eq!((first, last), (0, 2));
    }

    #[test]
    fn the_selection_is_the_preferred_anchor() {
        let mut root = dir("r");
        for i in 0..50 {
            root = root.child(dir(format!("d{i}")));
        }
        let tree = TreeBuilder::new("/r", root).build();
        let rows = build(
            &tree,
            NodeId::ROOT,
            &orders_for(&tree, &[NodeId::ROOT]),
            &HashSet::new(),
        );
        let viewport = Viewport {
            offset_y: 5.0 * ROW_H,
            height: 10.0 * ROW_H,
        };
        let selected = rows[8].key;
        assert_eq!(
            anchor(&rows, viewport, Some(selected)),
            Some((selected, 3.0 * ROW_H))
        );
        let at_top = Viewport {
            offset_y: 0.0,
            ..viewport
        };
        assert_eq!(anchor(&rows, at_top, None), None, "the top stays the top");
        assert_eq!(
            anchor(&rows, at_top, Some(rows[3].key)),
            Some((rows[3].key, 3.0 * ROW_H))
        );
        let off_screen = rows[40].key;
        assert_eq!(
            anchor(&rows, viewport, Some(off_screen)).map(|a| a.0),
            Some(rows[5].key)
        );
    }
}
