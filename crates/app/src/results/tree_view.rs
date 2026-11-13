//! State of the tree pane: child orders, the row list, scroll position and selection
//! (design §6.2, §6.4–6.7). Every method that changes the rows returns the offset to scroll
//! to when anchoring moved it.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use dirstat_core::tree::{ApplyEffect, NodeFlags, NodeId, Tree};

use super::order::{self, ChildOrder, Column, ReorderClock, Sort};
use super::rows::{self, Row, RowKey, Viewport};

/// Scan-driven row rebuilds happen at most this often.
const SCAN_REBUILD: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub struct TreeView {
    pub zoom: NodeId,
    pub sort: Sort,
    /// Cached child order of the zoom root and every expanded folder below it.
    pub orders: HashMap<NodeId, ChildOrder>,
    pub rows: Vec<Row>,
    pub open_aggregates: HashSet<NodeId>,
    scan_rows_dirty: bool,
    last_rows_build: Option<Instant>,
    pub reorder: ReorderClock,
    pub viewport: Viewport,
    pub selection: Option<RowKey>,
}

impl Default for TreeView {
    fn default() -> Self {
        TreeView {
            zoom: NodeId::ROOT,
            sort: Sort::default(),
            orders: HashMap::new(),
            rows: Vec::new(),
            open_aggregates: HashSet::new(),
            scan_rows_dirty: false,
            last_rows_build: None,
            reorder: ReorderClock::default(),
            viewport: Viewport::default(),
            selection: None,
        }
    }
}

impl TreeView {
    /// Rebuilds the rows with the anchor kept still. `Some(offset)` when the view must scroll.
    pub fn rebuild(&mut self, tree: &Tree, now: Instant) -> Option<f32> {
        let anchor = rows::anchor(&self.rows, self.viewport, self.selection);
        self.rows = rows::build(tree, self.zoom, &self.orders, &self.open_aggregates);
        self.scan_rows_dirty = false;
        self.last_rows_build = Some(now);
        let offset = match anchor {
            Some(anchor) => rows::restore(tree, &self.rows, self.viewport, anchor),
            None => self
                .viewport
                .offset_y
                .clamp(0.0, self.viewport.max_offset(self.rows.len())),
        };
        let moved = (offset - self.viewport.offset_y).abs() > 0.5;
        self.viewport.offset_y = offset;
        moved.then_some(offset)
    }

    /// A listing was applied. A folder's children all arrive in its own listing, so an
    /// expanded folder (or the zoom root) gets its exact order right away.
    pub fn on_listing(&mut self, tree: &Tree, id: NodeId, effect: ApplyEffect) {
        if id == self.zoom || effect.touched_expanded {
            self.orders.insert(id, order::exact(tree, id, self.sort));
            self.scan_rows_dirty = true;
        } else if self
            .orders
            .contains_key(&tree.node(id).parent().unwrap_or(NodeId::NONE))
        {
            // A visible folder changed state (pending -> scanning -> done).
            self.scan_rows_dirty = true;
        }
    }

    pub fn toggle(&mut self, tree: &mut Tree, key: RowKey, now: Instant) -> Option<f32> {
        self.reorder.activity(now);
        match key {
            RowKey::Dir(id) if rows::expandable(tree, id) => {
                let expand = !tree.node(id).has_flag(NodeFlags::EXPANDED);
                tree.set_flag(id, NodeFlags::EXPANDED, expand);
                let mut stack = vec![id];
                while let Some(dir) = stack.pop() {
                    if expand {
                        self.orders.insert(dir, order::exact(tree, dir, self.sort));
                    } else {
                        self.orders.remove(&dir);
                    }
                    stack.extend(
                        tree.children(dir)
                            .filter(|&c| tree.node(c).has_flag(NodeFlags::EXPANDED)),
                    );
                }
            }
            RowKey::Aggregate(dir) => {
                if !self.open_aggregates.remove(&dir) {
                    self.open_aggregates.insert(dir);
                }
            }
            _ => return None,
        }
        self.rebuild(tree, now)
    }

    pub fn select(&mut self, key: RowKey, now: Instant) {
        self.reorder.activity(now);
        self.selection = Some(key);
    }

    /// A header click: an exact sort, anchored to the selection.
    pub fn sort_by(&mut self, tree: &Tree, column: Column, now: Instant) -> Option<f32> {
        self.sort = self.sort.clicked(column);
        self.settle(tree, now)
    }

    /// Exact order everywhere: completion, sort changes.
    pub fn settle(&mut self, tree: &Tree, now: Instant) -> Option<f32> {
        let ids: Vec<NodeId> = self.orders.keys().copied().collect();
        for id in ids {
            self.orders.insert(id, order::exact(tree, id, self.sort));
        }
        self.rebuild(tree, now)
    }

    /// The 50 ms tick: throttled rebuilds for scan changes, and the calm re-order.
    pub fn tick(
        &mut self,
        tree: &Tree,
        now: Instant,
        scanning: bool,
        button_held: bool,
    ) -> Option<f32> {
        let reorder =
            (scanning || self.reorder.post_trash) && self.reorder.due(now, false, button_held);
        if reorder {
            let ids: Vec<NodeId> = self.orders.keys().copied().collect();
            for id in ids {
                let calm = order::calm(tree, id, &self.orders[&id], self.sort);
                self.orders.insert(id, calm);
            }
            self.reorder.post_trash = false;
            return self.rebuild(tree, now);
        }
        let throttled = self
            .last_rows_build
            .is_some_and(|t| now.saturating_duration_since(t) < SCAN_REBUILD);
        if self.scan_rows_dirty && !throttled {
            return self.rebuild(tree, now);
        }
        None
    }

    /// A viewport from the scrollable. Only a real scroll counts as activity: a content
    /// height change publishes a viewport too.
    pub fn scrolled(&mut self, offset_y: f32, height: f32, now: Instant) {
        if (offset_y - self.viewport.offset_y).abs() > 0.5 {
            self.reorder.activity(now);
        }
        self.viewport = Viewport { offset_y, height };
    }

    /// The layout's height, for anchoring before the first scroll event.
    pub fn set_height(&mut self, height: f32) {
        self.viewport.height = height.max(rows::ROW_H);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dirstat_core::tree::testing::{TreeBuilder, dir};

    #[test]
    fn listings_and_throttled_rebuilds() {
        let builder = TreeBuilder::new(
            "/r",
            dir("r").child(dir("a").child(dir("a1"))).child(dir("b")),
        );
        let mut tree = builder.started();
        let mut view = TreeView::default();
        let t0 = Instant::now();
        let mut listings = builder.listings().into_iter();
        let root = listings.next().unwrap();
        let effect = tree.apply(root);
        view.on_listing(&tree, NodeId::ROOT, effect);
        assert!(view.orders.contains_key(&NodeId::ROOT));
        view.tick(&tree, t0, true, false);
        assert_eq!(view.rows.len(), 2);

        // Expanding a pending folder shows its placeholder; its listing replaces it at once
        // in the order, and in the rows on the next unthrottled tick.
        view.toggle(&mut tree, RowKey::Dir(NodeId(1)), t0);
        assert_eq!(view.rows[1].key, RowKey::Placeholder(NodeId(1)));
        let a = listings.next().unwrap();
        let effect = tree.apply(a);
        view.on_listing(&tree, NodeId(1), effect);
        assert!(
            view.tick(&tree, t0 + Duration::from_millis(50), false, false)
                .is_none()
        );
        assert_eq!(
            view.rows[1].key,
            RowKey::Placeholder(NodeId(1)),
            "throttled"
        );
        view.tick(&tree, t0 + Duration::from_millis(120), false, false);
        assert_eq!(view.rows[1].key, RowKey::Dir(NodeId(3)));

        // Collapsing drops the orders of the subtree.
        view.toggle(&mut tree, RowKey::Dir(NodeId(1)), t0);
        assert_eq!(view.rows.len(), 2);
        assert!(!view.orders.contains_key(&NodeId(1)));
    }

    #[test]
    fn real_scrolls_count_as_activity() {
        let mut view = TreeView::default();
        let t0 = Instant::now();
        assert!(view.reorder.due(t0, false, false));
        view.scrolled(0.0, 500.0, t0 + Duration::from_millis(1500));
        assert!(
            view.reorder
                .due(t0 + Duration::from_millis(1600), false, false),
            "a height change alone isn't"
        );
        view.scrolled(300.0, 500.0, t0 + Duration::from_millis(3200));
        assert!(
            !view
                .reorder
                .due(t0 + Duration::from_millis(3300), false, false)
        );
    }
}
