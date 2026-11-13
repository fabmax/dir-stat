//! State of the tree pane: child orders, the row list, scroll position, selection, zoom and
//! history (design §6.2, §6.4–6.7). Methods that change the rows return the offset to scroll
//! to when it moved.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use dirstat_core::smallfiles::SmallFile;
use dirstat_core::tree::{ApplyEffect, NodeFlags, NodeId, Tree};

use super::order::{self, ChildOrder, Column, ReorderClock, Sort};
use super::rows::{self, FILE_BATCH, OpenAggregate, ROW_H, Row, RowKey, SmallFiles, Viewport};
use crate::keys::KeyAction;

/// Scan-driven row rebuilds happen at most this often.
const SCAN_REBUILD: Duration = Duration::from_millis(100);
/// ⌥→ expands the largest path this deep, through children of at least 1%.
const EXPAND_LARGEST_LEVELS: usize = 3;

/// Where the view was: zoom, selection and scroll.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub zoom: NodeId,
    pub selection: Option<RowKey>,
    pub offset_y: f32,
}

#[derive(Debug, Default)]
pub struct History {
    back: Vec<Place>,
    forward: Vec<Place>,
}

impl History {
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }
    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

/// What a key or click asks the rest of the app to do.
#[derive(Debug, Default, PartialEq)]
pub struct Effect {
    pub scroll: Option<f32>,
    /// Start the small-file listing of this folder.
    pub list_small: Option<NodeId>,
    pub reveal: Option<RowKey>,
    pub copy: Option<RowKey>,
}

#[derive(Debug)]
pub struct TreeView {
    pub zoom: NodeId,
    pub sort: Sort,
    /// Cached child order of the zoom root and every expanded folder below it.
    pub orders: HashMap<NodeId, ChildOrder>,
    pub rows: Vec<Row>,
    pub open_aggregates: HashMap<NodeId, OpenAggregate>,
    scan_rows_dirty: bool,
    last_rows_build: Option<Instant>,
    pub reorder: ReorderClock,
    pub viewport: Viewport,
    pub selection: Option<RowKey>,
    pub history: History,
}

impl Default for TreeView {
    fn default() -> Self {
        TreeView {
            zoom: NodeId::ROOT,
            sort: Sort::default(),
            orders: HashMap::new(),
            rows: Vec::new(),
            open_aggregates: HashMap::new(),
            scan_rows_dirty: false,
            last_rows_build: None,
            reorder: ReorderClock::default(),
            viewport: Viewport::default(),
            selection: None,
            history: History::default(),
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
        self.set_offset(offset)
    }

    fn set_offset(&mut self, offset: f32) -> Option<f32> {
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

    /// Orders for the zoom root and every expanded folder below it, nothing else.
    fn sync_orders(&mut self, tree: &Tree) {
        let mut wanted = HashMap::new();
        let mut stack = vec![self.zoom];
        while let Some(dir) = stack.pop() {
            let order = self
                .orders
                .remove(&dir)
                .unwrap_or_else(|| order::exact(tree, dir, self.sort));
            wanted.insert(dir, order);
            stack.extend(
                tree.children(dir)
                    .filter(|&c| tree.node(c).has_flag(NodeFlags::EXPANDED)),
            );
        }
        self.orders = wanted;
    }

    pub fn toggle(&mut self, tree: &mut Tree, key: RowKey, now: Instant) -> Effect {
        self.reorder.activity(now);
        let mut effect = Effect::default();
        match key {
            RowKey::Dir(id) if rows::expandable(tree, id) => {
                let expand = !tree.node(id).has_flag(NodeFlags::EXPANDED);
                tree.set_flag(id, NodeFlags::EXPANDED, expand);
                self.sync_orders(tree);
            }
            RowKey::Aggregate(dir) => {
                if self.open_aggregates.remove(&dir).is_none() {
                    self.open_aggregates.insert(dir, OpenAggregate::default());
                    effect.list_small = Some(dir);
                }
            }
            _ => return effect,
        }
        effect.scroll = self.rebuild(tree, now);
        effect
    }

    /// The small-file listing of `dir` arrived.
    pub fn small_files_listed(
        &mut self,
        tree: &Tree,
        dir: NodeId,
        result: Result<Vec<SmallFile>, String>,
        now: Instant,
    ) -> Option<f32> {
        let open = self.open_aggregates.get_mut(&dir)?;
        open.small = match result {
            Ok(files) => SmallFiles::Loaded(files),
            Err(error) => SmallFiles::Failed(error),
        };
        self.rebuild(tree, now)
    }

    pub fn select(&mut self, key: RowKey, now: Instant) {
        self.reorder.activity(now);
        self.selection = Some(key);
    }

    /// A click on a row; "Show 50 more" reveals the next batch.
    pub fn press(&mut self, tree: &Tree, key: RowKey, now: Instant) -> Effect {
        self.select(key, now);
        let mut effect = Effect::default();
        if let RowKey::MoreFiles(dir) = key
            && let Some(open) = self.open_aggregates.get_mut(&dir)
            && rows::aggregate_entries(
                tree,
                dir,
                &OpenAggregate {
                    shown: usize::MAX,
                    ..open.clone()
                },
            )
            .0
            .len()
                > open.shown
        {
            open.shown += FILE_BATCH;
            effect.scroll = self.rebuild(tree, now);
        }
        effect
    }

    /// Double-click and Return: zoom into folders, reveal files, open the aggregate.
    pub fn activate(&mut self, tree: &mut Tree, key: RowKey, now: Instant) -> Effect {
        self.select(key, now);
        match key {
            RowKey::Dir(id) if rows::expandable(tree, id) => Effect {
                scroll: self.zoom_to(tree, id, None, now),
                ..Effect::default()
            },
            RowKey::File(..) | RowKey::SmallFile(..) => Effect {
                reveal: Some(key),
                ..Effect::default()
            },
            RowKey::Aggregate(_) => self.toggle(tree, key, now),
            _ => Effect::default(),
        }
    }

    // ---- zoom and history ----

    fn place(&self) -> Place {
        Place {
            zoom: self.zoom,
            selection: self.selection,
            offset_y: self.viewport.offset_y,
        }
    }

    /// Makes `id` the zoom root (pushes history). Zooming never rescans anything.
    pub fn zoom_to(
        &mut self,
        tree: &Tree,
        id: NodeId,
        select: Option<RowKey>,
        now: Instant,
    ) -> Option<f32> {
        if id == self.zoom || tree.get(id).is_none() {
            return None;
        }
        self.history.back.push(self.place());
        self.history.forward.clear();
        self.go(
            tree,
            Place {
                zoom: id,
                selection: select,
                offset_y: 0.0,
            },
            now,
        )
    }

    /// One level up; the folder we came from is selected, as in Finder.
    pub fn zoom_out(&mut self, tree: &Tree, now: Instant) -> Option<f32> {
        let parent = tree.node(self.zoom).parent()?;
        let from = RowKey::Dir(self.zoom);
        let scroll = self.zoom_to(tree, parent, Some(from), now);
        self.reveal_selection().or(scroll)
    }

    pub fn back(&mut self, tree: &Tree, now: Instant) -> Option<f32> {
        let place = self.history.back.pop()?;
        self.history.forward.push(self.place());
        self.go(tree, place, now)
    }

    pub fn forward(&mut self, tree: &Tree, now: Instant) -> Option<f32> {
        let place = self.history.forward.pop()?;
        self.history.back.push(self.place());
        self.go(tree, place, now)
    }

    fn go(&mut self, tree: &Tree, place: Place, now: Instant) -> Option<f32> {
        self.reorder.activity(now);
        self.zoom = place.zoom;
        self.selection = place.selection;
        self.sync_orders(tree);
        self.rows = rows::build(tree, self.zoom, &self.orders, &self.open_aggregates);
        self.last_rows_build = Some(now);
        let offset = place
            .offset_y
            .clamp(0.0, self.viewport.max_offset(self.rows.len()));
        self.viewport.offset_y = offset;
        // Always scroll: the scrollable still holds the old position.
        Some(offset)
    }

    /// Scrolls the selection into view if it isn't.
    fn reveal_selection(&mut self) -> Option<f32> {
        let index = self
            .rows
            .iter()
            .position(|r| Some(r.key) == self.selection)?;
        let y = index as f32 * ROW_H;
        let offset = if y < self.viewport.offset_y {
            y
        } else if y + ROW_H > self.viewport.offset_y + self.viewport.height {
            y + ROW_H - self.viewport.height
        } else {
            return None;
        };
        self.set_offset(offset.clamp(0.0, self.viewport.max_offset(self.rows.len())))
    }

    // ---- keyboard ----

    pub fn key(&mut self, tree: &mut Tree, action: KeyAction, now: Instant) -> Effect {
        use KeyAction::*;
        self.reorder.activity(now);
        let index = self.rows.iter().position(|r| Some(r.key) == self.selection);
        let page = ((self.viewport.height / ROW_H) as usize)
            .saturating_sub(1)
            .max(1);
        let last = self.rows.len().saturating_sub(1);
        let target = match (action, index) {
            _ if self.rows.is_empty() => None,
            (Down, None) | (Home, _) => Some(0),
            (Up, None) => Some(0),
            (Down, Some(i)) => Some((i + 1).min(last)),
            (Up, Some(i)) => Some(i.saturating_sub(1)),
            (PageDown, i) => Some(i.map_or(0, |i| (i + page).min(last))),
            (PageUp, i) => Some(i.map_or(0, |i| i.saturating_sub(page))),
            (End, _) => Some(last),
            _ => None,
        };
        if let Some(target) = target {
            self.selection = Some(self.rows[target].key);
            return Effect {
                scroll: self.reveal_selection(),
                ..Effect::default()
            };
        }
        let selected = self.selection;
        let mut effect = match (action, selected) {
            (Right, Some(key)) => self.right(tree, key, index, now),
            (Left, Some(key)) => self.left(tree, key, now),
            (ExpandLargest, Some(RowKey::Dir(id))) => self.expand_largest(tree, id, now),
            (CollapseSubtree, Some(RowKey::Dir(id))) => self.collapse_subtree(tree, id, now),
            (Enter, Some(key)) => self.activate(tree, key, now),
            (ZoomOut, _) => Effect {
                scroll: self.zoom_out(tree, now),
                ..Effect::default()
            },
            (Back, _) => Effect {
                scroll: self.back(tree, now),
                ..Effect::default()
            },
            (Forward, _) => Effect {
                scroll: self.forward(tree, now),
                ..Effect::default()
            },
            (Reveal, Some(key)) => Effect {
                reveal: Some(key),
                ..Effect::default()
            },
            (CopyPath, Some(key)) => Effect {
                copy: Some(key),
                ..Effect::default()
            },
            _ => Effect::default(),
        };
        if effect.scroll.is_none() {
            effect.scroll = self.reveal_selection();
        }
        effect
    }

    /// → expands, or goes to the first child when already expanded.
    fn right(
        &mut self,
        tree: &mut Tree,
        key: RowKey,
        index: Option<usize>,
        now: Instant,
    ) -> Effect {
        let open = match key {
            RowKey::Dir(id) => {
                if !rows::expandable(tree, id) {
                    return Effect::default();
                }
                tree.node(id).has_flag(NodeFlags::EXPANDED)
            }
            RowKey::Aggregate(dir) => self.open_aggregates.contains_key(&dir),
            _ => return Effect::default(),
        };
        if !open {
            return self.toggle(tree, key, now);
        }
        let depth = index.map(|i| self.rows[i].depth);
        if let Some(next) = index.and_then(|i| self.rows.get(i + 1))
            && Some(next.depth) > depth
        {
            self.selection = Some(next.key);
        }
        Effect::default()
    }

    /// ← collapses, or goes to the parent row.
    fn left(&mut self, tree: &mut Tree, key: RowKey, now: Instant) -> Effect {
        match key {
            RowKey::Dir(id)
                if tree.node(id).has_flag(NodeFlags::EXPANDED) && rows::expandable(tree, id) =>
            {
                return self.toggle(tree, key, now);
            }
            RowKey::Aggregate(dir) if self.open_aggregates.contains_key(&dir) => {
                return self.toggle(tree, key, now);
            }
            _ => {}
        }
        // Rows below an open aggregate belong to it; everything else to its folder row.
        let index = self.rows.iter().position(|r| r.key == key);
        let depth = index.map(|i| self.rows[i].depth).unwrap_or(0);
        if let Some(i) = index
            && depth > 0
            && let Some(parent) = self.rows[..i].iter().rev().find(|r| r.depth < depth)
        {
            self.selection = Some(parent.key);
        }
        Effect::default()
    }

    /// ⌥→: expand the path of largest children, up to 3 levels, children of ≥ 1% only.
    fn expand_largest(&mut self, tree: &mut Tree, id: NodeId, now: Instant) -> Effect {
        let mut current = id;
        for _ in 0..EXPAND_LARGEST_LEVELS {
            if !rows::expandable(tree, current) {
                break;
            }
            tree.set_flag(current, NodeFlags::EXPANDED, true);
            let total = tree.node(current).total().alloc;
            let largest = tree
                .children(current)
                .max_by_key(|&c| tree.node(c).total().alloc);
            match largest {
                Some(child)
                    if tree.node(child).total().alloc.saturating_mul(100) >= total && total > 0 =>
                {
                    current = child
                }
                _ => break,
            }
        }
        self.sync_orders(tree);
        Effect {
            scroll: self.rebuild(tree, now),
            ..Effect::default()
        }
    }

    /// ⌥←: collapse the folder and everything expanded below it.
    fn collapse_subtree(&mut self, tree: &mut Tree, id: NodeId, now: Instant) -> Effect {
        let mut stack = vec![id];
        while let Some(dir) = stack.pop() {
            tree.set_flag(dir, NodeFlags::EXPANDED, false);
            stack.extend(
                tree.children(dir)
                    .filter(|&c| tree.node(c).has_flag(NodeFlags::EXPANDED)),
            );
        }
        self.sync_orders(tree);
        Effect {
            scroll: self.rebuild(tree, now),
            ..Effect::default()
        }
    }

    // ---- sorting and ticks ----

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
        self.viewport.height = height.max(ROW_H);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dirstat_core::tree::testing::{TreeBuilder, dir, large};

    fn ready(builder: &TreeBuilder) -> (Tree, TreeView) {
        let tree = builder.build();
        let mut view = TreeView::default();
        view.orders.insert(
            NodeId::ROOT,
            order::exact(&tree, NodeId::ROOT, Sort::default()),
        );
        view.rebuild(&tree, Instant::now());
        (tree, view)
    }

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
        let effect = tree.apply(listings.next().unwrap());
        view.on_listing(&tree, NodeId::ROOT, effect);
        view.tick(&tree, t0, true, false);
        assert_eq!(view.rows.len(), 2);

        view.toggle(&mut tree, RowKey::Dir(NodeId(1)), t0);
        assert_eq!(view.rows[1].key, RowKey::Placeholder(NodeId(1)));
        let effect = tree.apply(listings.next().unwrap());
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

    #[test]
    fn arrow_keys_move_expand_and_collapse() {
        let builder = TreeBuilder::new(
            "/r",
            dir("r")
                .child(
                    dir("big")
                        .small_files(1, 9000)
                        .child(dir("inner").small_files(1, 5000)),
                )
                .child(dir("small")),
        );
        let (mut tree, mut view) = ready(&builder);
        let now = Instant::now();
        let (big, inner, small) = (
            RowKey::Dir(NodeId(1)),
            RowKey::Dir(NodeId(3)),
            RowKey::Dir(NodeId(2)),
        );
        view.key(&mut tree, KeyAction::Down, now);
        assert_eq!(view.selection, Some(big), "the first ↓ selects row 1");
        view.key(&mut tree, KeyAction::Right, now);
        assert!(tree.node(NodeId(1)).has_flag(NodeFlags::EXPANDED));
        view.key(&mut tree, KeyAction::Right, now);
        assert_eq!(
            view.selection,
            Some(inner),
            "→ on an expanded folder goes to its first child"
        );
        view.key(&mut tree, KeyAction::Left, now);
        assert_eq!(view.selection, Some(big), "← goes to the parent");
        view.key(&mut tree, KeyAction::Left, now);
        assert!(
            !tree.node(NodeId(1)).has_flag(NodeFlags::EXPANDED),
            "← collapses"
        );
        view.key(&mut tree, KeyAction::End, now);
        assert_eq!(view.selection, Some(small));
        view.key(&mut tree, KeyAction::Home, now);
        assert_eq!(view.selection, Some(big));
        view.key(&mut tree, KeyAction::ExpandLargest, now);
        assert!(tree.node(NodeId(1)).has_flag(NodeFlags::EXPANDED));
        view.key(&mut tree, KeyAction::CollapseSubtree, now);
        assert!(!tree.node(NodeId(1)).has_flag(NodeFlags::EXPANDED));
    }

    #[test]
    fn zoom_out_and_back_restore_selection_and_scroll() {
        let mut root = dir("r");
        for i in 0..60 {
            let mut d = dir(format!("d{i:02}")).small_files(1, 100_000 - i * 10);
            if i == 40 {
                for j in 0..30 {
                    d = d.child(dir(format!("c{j:02}")).small_files(1, 1000 - j));
                }
            }
            root = root.child(d);
        }
        let builder = TreeBuilder::new("/r", root);
        let (mut tree, mut view) = ready(&builder);
        view.set_height(10.0 * ROW_H);
        let now = Instant::now();
        // Scroll down and select d40, then zoom into it.
        view.scrolled(35.0 * ROW_H, 10.0 * ROW_H, now);
        let d40 = RowKey::Dir(tree.find_path(std::path::Path::new("/r/d40")).unwrap());
        view.select(d40, now);
        let scroll = view.key(&mut tree, KeyAction::Enter, now).scroll;
        assert_eq!(scroll, Some(0.0));
        assert_eq!(view.zoom, d40.dir());
        assert_eq!(view.rows.len(), 31, "30 folders and its own file");
        view.key(&mut tree, KeyAction::End, now);
        let c_last = view.selection;

        // Back restores the root view as it was.
        let scroll = view.key(&mut tree, KeyAction::Back, now).scroll;
        assert_eq!(
            (view.zoom, view.selection, scroll),
            (NodeId::ROOT, Some(d40), Some(35.0 * ROW_H))
        );
        // Forward returns into the folder with its selection.
        view.key(&mut tree, KeyAction::Forward, now);
        assert_eq!((view.zoom, view.selection), (d40.dir(), c_last));
        // Zooming out selects the folder we came from and keeps it visible.
        let scroll = view.key(&mut tree, KeyAction::ZoomOut, now).scroll;
        assert_eq!((view.zoom, view.selection), (NodeId::ROOT, Some(d40)));
        let index = view.rows.iter().position(|r| r.key == d40).unwrap();
        let y = index as f32 * ROW_H - scroll.unwrap_or(view.viewport.offset_y);
        assert!(
            y >= 0.0 && y + ROW_H <= view.viewport.height,
            "the folder is visible"
        );
        assert!(view.history.can_go_back() && !view.history.can_go_forward());
    }

    #[test]
    fn aggregate_lists_files_in_batches_of_fifty() {
        // 3,000 small files in one folder, plus a few kept large ones under 1% of it.
        let mut folder = dir("many").small_files(3000, 3_000_000_000);
        for i in 0..3 {
            folder = folder.large(large(format!("kept{i}"), 2_000_000 + i));
        }
        let builder = TreeBuilder::new("/r", dir("r").child(folder));
        let (mut tree, mut view) = ready(&builder);
        let now = Instant::now();
        let many = NodeId(1);
        view.toggle(&mut tree, RowKey::Dir(many), now);
        let effect = view.toggle(&mut tree, RowKey::Aggregate(many), now);
        assert_eq!(effect.list_small, Some(many), "opening starts the listing");
        let more = |view: &TreeView| {
            view.rows
                .iter()
                .filter(|r| r.key == RowKey::MoreFiles(many))
                .count()
        };
        assert_eq!(more(&view), 1, "Listing files…");

        let small: Vec<SmallFile> = (0..3000)
            .map(|i| SmallFile {
                name: std::ffi::OsStr::new(&format!("f{i:04}")).into(),
                alloc: 1000,
                apparent: 1000,
                mtime: 0,
                ino: i,
                kind: dirstat_core::platform::EntryKind::File,
                nlink: 1,
                flags: Default::default(),
            })
            .collect();
        view.small_files_listed(&tree, many, Ok(small), now);
        let file_rows = |view: &TreeView| {
            view.rows
                .iter()
                .filter(|r| matches!(r.key, RowKey::File(..) | RowKey::SmallFile(..)))
                .count()
        };
        assert_eq!(file_rows(&view), 50);
        view.press(&tree, RowKey::MoreFiles(many), now);
        assert_eq!(file_rows(&view), 100);
        for _ in 0..100 {
            view.press(&tree, RowKey::MoreFiles(many), now);
        }
        assert_eq!(file_rows(&view), 3003, "all kept and small files");
        assert_eq!(more(&view), 0, "nothing left behind a More row");
    }
}
