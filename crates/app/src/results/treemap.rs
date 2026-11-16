//! The treemap model (UX §8, design §6.10): squarified layout of the zoom root up to three
//! levels, the sticky color slots, and hit testing. Pure; `treemap_view` draws it.

use std::time::{Duration, Instant};

use dirstat_core::tree::{FileFlags, NodeFlags, NodeId, NodeState, Tree};
use iced::{Point, Rectangle, Size};

use super::order::split_files;
use super::rows::RowKey;

/// Deeper levels aren't laid out.
pub const MAX_LEVEL: u8 = 3;
/// At most this many children per level; the rest become one "N more" block.
pub const MAX_CHILDREN: usize = 200;
/// A block is subdivided only from this size on.
const SUBDIVIDE: Size = Size::new(48.0, 32.0);
/// Smaller children aren't drawn; their parent's fill covers them.
const MIN_SIDE: f32 = 4.0;
/// Level-1 blocks from this size get a header strip.
const HEADER_FROM: Size = Size::new(80.0, 40.0);
pub const HEADER_H: f32 = 18.0;
/// Between siblings and around nested blocks.
const GAP: f32 = 2.0;
/// While scanning, the map re-lays out at most this often...
pub const RELAYOUT_EVERY: Duration = Duration::from_secs(1);
/// ...and waits for the pointer to leave it at most this long.
pub const POINTER_DEFER: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Dir(NodeId),
    /// A listed large file: its folder and index.
    File(NodeId, u32),
    /// The "N files" aggregate of a folder.
    Files(NodeId),
    /// "N more" children of a folder.
    More(NodeId, u32),
}

impl BlockKind {
    /// The tree row a click selects.
    pub fn row_key(self) -> RowKey {
        match self {
            BlockKind::Dir(id) | BlockKind::More(id, _) => RowKey::Dir(id),
            BlockKind::File(dir, index) => RowKey::File(dir, index),
            BlockKind::Files(dir) => RowKey::Aggregate(dir),
        }
    }

    /// The folder the block is or belongs to.
    pub fn dir(self) -> NodeId {
        match self {
            BlockKind::Dir(id)
            | BlockKind::More(id, _)
            | BlockKind::File(id, _)
            | BlockKind::Files(id) => id,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub kind: BlockKind,
    /// The filled area, gaps already taken off.
    pub rect: Rectangle,
    /// 1 = child of the zoom root.
    pub level: u8,
    /// The top-level group's color slot (0..3), `None` = "Other" gray.
    pub slot: Option<u8>,
    /// Level 1 with an 18 px "Name  Size" strip.
    pub header: bool,
    pub name: String,
    pub bytes: u64,
    /// Index of the enclosing block, if any.
    pub parent: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    /// Parents before their children, so drawing in order nests correctly and the last
    /// block containing a point is the deepest.
    pub blocks: Vec<Block>,
}

impl Layout {
    /// The deepest block under `point`.
    pub fn hit(&self, point: Point) -> Option<usize> {
        self.blocks.iter().rposition(|b| b.rect.contains(point))
    }

    pub fn find(&self, key: RowKey) -> Option<usize> {
        self.blocks.iter().position(|b| b.kind.row_key() == key)
    }
}

/// The three color slots of the zoom root's largest children. A folder keeps its slot while
/// it stays in the top three; a newcomer takes a freed slot (UX §8.4).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Slots {
    zoom: Option<NodeId>,
    pub ids: [Option<NodeId>; 3],
}

impl Slots {
    pub fn update(&mut self, tree: &Tree, zoom: NodeId) {
        if self.zoom != Some(zoom) {
            *self = Slots {
                zoom: Some(zoom),
                ids: [None; 3],
            };
        }
        let mut dirs = visible_dirs(tree, zoom);
        dirs.sort_by_key(|&(id, bytes)| (std::cmp::Reverse(bytes), id));
        let top: Vec<NodeId> = dirs.iter().take(3).map(|&(id, _)| id).collect();
        for slot in &mut self.ids {
            if slot.is_some_and(|id| !top.contains(&id)) {
                *slot = None;
            }
        }
        for id in top {
            if self.ids.contains(&Some(id)) {
                continue;
            }
            if let Some(free) = self.ids.iter_mut().find(|s| s.is_none()) {
                *free = Some(id);
            }
        }
    }

    pub fn slot_of(&self, id: NodeId) -> Option<u8> {
        self.ids
            .iter()
            .position(|&s| s == Some(id))
            .map(|i| i as u8)
    }
}

/// Subfolders that take space: not trashed, not skipped, not empty.
fn visible_dirs(tree: &Tree, dir: NodeId) -> Vec<(NodeId, u64)> {
    tree.children(dir)
        .filter_map(|c| {
            let node = tree.get(c)?;
            let gone =
                node.has_flag(NodeFlags::TRASHED) || matches!(node.state(), NodeState::Skipped(_));
            let bytes = node.total().alloc;
            (!gone && bytes > 0).then_some((c, bytes))
        })
        .collect()
}

/// What a folder's area is split into, largest first, capped at `MAX_CHILDREN`.
fn entries(tree: &Tree, dir: NodeId) -> Vec<(BlockKind, u64, String)> {
    let name = |id: NodeId| tree.node(id).name().to_string_lossy().into_owned();
    let mut out: Vec<(BlockKind, u64, String)> = visible_dirs(tree, dir)
        .into_iter()
        .map(|(id, bytes)| (BlockKind::Dir(id), bytes, name(id)))
        .collect();
    let node = tree.node(dir);
    let split = split_files(node);
    for &index in &split.shown {
        let file = &node.large_files()[index as usize];
        if !file.flags.contains(FileFlags::TRASHED) && file.alloc > 0 {
            out.push((
                BlockKind::File(dir, index),
                file.alloc,
                file.name.to_string_lossy().into_owned(),
            ));
        }
    }
    if split.aggregate_alloc > 0 {
        out.push((
            BlockKind::Files(dir),
            split.aggregate_alloc,
            crate::format::plural(split.aggregate_count, "file", "files"),
        ));
    }
    out.sort_by_key(|e| std::cmp::Reverse(e.1));
    if out.len() > MAX_CHILDREN {
        let rest = out.split_off(MAX_CHILDREN - 1);
        let bytes = rest.iter().map(|e| e.1).sum();
        let count = rest.len() as u32;
        out.push((
            BlockKind::More(dir, count),
            bytes,
            format!("{} more", crate::format::thousands(u64::from(count))),
        ));
    }
    out
}

/// Lays out `zoom` in `size`.
pub fn layout(tree: &Tree, zoom: NodeId, size: Size, slots: &Slots) -> Layout {
    let mut out = Layout::default();
    if tree.get(zoom).is_none() || size.width < MIN_SIDE || size.height < MIN_SIDE {
        return out;
    }
    let bounds = Rectangle::new(Point::ORIGIN, size);
    lay_out_children(tree, zoom, bounds, 1, None, None, slots, &mut out);
    out
}

#[allow(clippy::too_many_arguments)]
fn lay_out_children(
    tree: &Tree,
    dir: NodeId,
    area: Rectangle,
    level: u8,
    slot: Option<u8>,
    parent: Option<usize>,
    slots: &Slots,
    out: &mut Layout,
) {
    let entries = entries(tree, dir);
    let values: Vec<u64> = entries.iter().map(|e| e.1).collect();
    let rects = squarify(&values, area);
    for ((kind, bytes, name), rect) in entries.into_iter().zip(rects) {
        // Half a gap on each side: siblings end up a full gap apart.
        let rect = inset(rect, GAP / 2.0);
        if rect.width < MIN_SIDE || rect.height < MIN_SIDE {
            continue;
        }
        let slot = match (level, kind) {
            (1, BlockKind::Dir(id)) => slots.slot_of(id),
            (1, _) => None,
            _ => slot,
        };
        let header =
            level == 1 && rect.width >= HEADER_FROM.width && rect.height >= HEADER_FROM.height;
        let index = out.blocks.len();
        out.blocks.push(Block {
            kind,
            rect,
            level,
            slot,
            header,
            name,
            bytes,
            parent,
        });
        if let BlockKind::Dir(id) = kind
            && level < MAX_LEVEL
        {
            let mut inner = inset(rect, GAP);
            if header {
                inner.y += HEADER_H - GAP;
                inner.height -= HEADER_H - GAP;
            }
            if inner.width >= SUBDIVIDE.width && inner.height >= SUBDIVIDE.height {
                lay_out_children(tree, id, inner, level + 1, slot, Some(index), slots, out);
            }
        }
    }
}

fn inset(r: Rectangle, by: f32) -> Rectangle {
    Rectangle {
        x: r.x + by,
        y: r.y + by,
        width: (r.width - 2.0 * by).max(0.0),
        height: (r.height - 2.0 * by).max(0.0),
    }
}

/// Squarified treemap (Bruls et al.), from the research probe. `values` sorted descending;
/// zeros get an empty rectangle.
pub fn squarify(values: &[u64], bounds: Rectangle) -> Vec<Rectangle> {
    let mut out = vec![Rectangle::new(Point::ORIGIN, Size::ZERO); values.len()];
    let total: f64 = values.iter().map(|&v| v as f64).sum();
    if total <= 0.0 || bounds.width < 1.0 || bounds.height < 1.0 {
        return out;
    }
    let area = f64::from(bounds.width) * f64::from(bounds.height);
    let scaled: Vec<f64> = values.iter().map(|&v| v as f64 * area / total).collect();
    let mut free = bounds;
    let mut start = 0;
    let end = scaled
        .iter()
        .rposition(|&v| v > 0.0)
        .map_or(0, |last| last + 1);
    while start < end && free.width >= 1.0 && free.height >= 1.0 {
        let short = f64::from(free.width.min(free.height));
        let mut stop = start + 1;
        let mut best = worst(&scaled[start..stop], short);
        while stop < end {
            let candidate = worst(&scaled[start..=stop], short);
            if candidate > best {
                break;
            }
            best = candidate;
            stop += 1;
        }
        let row_sum: f64 = scaled[start..stop].iter().sum();
        if free.width >= free.height {
            let strip = (row_sum / f64::from(free.height)) as f32;
            let mut y = free.y;
            for (i, v) in scaled.iter().enumerate().take(stop).skip(start) {
                let height = (v / f64::from(strip)) as f32;
                out[i] = Rectangle {
                    x: free.x,
                    y,
                    width: strip,
                    height,
                };
                y += height;
            }
            free.x += strip;
            free.width -= strip;
        } else {
            let strip = (row_sum / f64::from(free.width)) as f32;
            let mut x = free.x;
            for (i, v) in scaled.iter().enumerate().take(stop).skip(start) {
                let width = (v / f64::from(strip)) as f32;
                out[i] = Rectangle {
                    x,
                    y: free.y,
                    width,
                    height: strip,
                };
                x += width;
            }
            free.y += strip;
            free.height -= strip;
        }
        start = stop;
    }
    out
}

fn worst(row: &[f64], short: f64) -> f64 {
    let sum: f64 = row.iter().sum();
    let max = row.iter().copied().fold(f64::MIN, f64::max);
    let min = row
        .iter()
        .copied()
        .fold(f64::MAX, f64::min)
        .max(f64::MIN_POSITIVE);
    let (sum2, short2) = (sum * sum, short * short);
    (short2 * max / sum2).max(sum2 / (short2 * min))
}

/// When the map may take a new layout (UX §8.6): while scanning at most once a second and
/// not while the pointer is over it (deferred up to 5 s); otherwise whenever asked.
#[derive(Debug, Default)]
pub struct MapClock {
    /// Bumped on every allowed relayout; the canvas re-lays out when it changes.
    pub version: u64,
    last: Option<Instant>,
    /// Since when a relayout has been waiting for the pointer to leave.
    waiting: Option<Instant>,
    /// Something changed since the last layout.
    pub dirty: bool,
}

impl MapClock {
    /// Returns true when it bumped the version.
    pub fn tick(&mut self, now: Instant, scanning: bool, pointer_over: bool) -> bool {
        if !self.dirty {
            self.waiting = None;
            return false;
        }
        if scanning {
            if self
                .last
                .is_some_and(|t| now.saturating_duration_since(t) < RELAYOUT_EVERY)
            {
                return false;
            }
            if pointer_over {
                let since = *self.waiting.get_or_insert(now);
                if now.saturating_duration_since(since) < POINTER_DEFER {
                    return false;
                }
            }
        } else if pointer_over {
            // Completed scans relayout only on zoom, size or data changes; still never
            // under a pointer that is reading the map, up to the same limit.
            let since = *self.waiting.get_or_insert(now);
            if now.saturating_duration_since(since) < POINTER_DEFER {
                return false;
            }
        }
        self.dirty = false;
        self.waiting = None;
        self.last = Some(now);
        self.version += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dirstat_core::tree::testing::{Dir, TreeBuilder, dir, large};

    fn area(r: &Rectangle) -> f64 {
        f64::from(r.width) * f64::from(r.height)
    }

    fn overlaps(a: &Rectangle, b: &Rectangle) -> bool {
        a.x < b.x + b.width - 0.01
            && b.x < a.x + a.width - 0.01
            && a.y < b.y + b.height - 0.01
            && b.y < a.y + a.height - 0.01
    }

    #[test]
    fn squarify_is_proportional_and_fills_the_bounds() {
        let values = [600, 300, 100];
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(400.0, 250.0));
        let rects = squarify(&values, bounds);
        let total = area(&bounds);
        for (v, r) in values.iter().zip(&rects) {
            let share = area(r) / total;
            assert!((share - *v as f64 / 1000.0).abs() < 0.01, "{share}");
        }
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(!overlaps(a, b), "{a:?} {b:?}");
            }
        }
        assert!(squarify(&[0, 0], bounds).iter().all(|r| r.width == 0.0));
    }

    fn sample() -> Tree {
        TreeBuilder::new(
            "/r",
            dir("r")
                .child(
                    dir("a")
                        .child(
                            dir("a1")
                                .small_files(1, 40_000_000)
                                .child(dir("deep").small_files(1, 10_000_000)),
                        )
                        .child(dir("a2").small_files(1, 20_000_000)),
                )
                .child(dir("b").small_files(1, 30_000_000))
                .child(dir("c").small_files(1, 20_000_000))
                .child(dir("d").small_files(1, 10_000_000))
                .child(dir("empty"))
                .large(large("disk.img", 25_000_000))
                .small_files(3, 3_000_000),
        )
        .build()
    }

    fn find(tree: &Tree, p: &str) -> NodeId {
        tree.find_path(std::path::Path::new(p)).unwrap()
    }

    #[test]
    fn three_levels_slots_files_and_nesting() {
        let tree = sample();
        let mut slots = Slots::default();
        slots.update(&tree, NodeId::ROOT);
        let l = layout(&tree, NodeId::ROOT, Size::new(800.0, 600.0), &slots);
        let block = |p: &str| {
            let id = find(&tree, p);
            l.blocks
                .iter()
                .find(|b| b.kind == BlockKind::Dir(id))
                .unwrap()
        };
        let (a, b, d) = (block("/r/a"), block("/r/b"), block("/r/d"));
        assert_eq!((a.level, a.slot, a.header), (1, Some(0), true));
        assert_eq!(b.slot, Some(1));
        assert_eq!(d.slot, None, "fourth: Other gray");
        assert_eq!(block("/r/a/a1").level, 2);
        assert_eq!(
            block("/r/a/a1").slot,
            Some(0),
            "nested blocks keep the group's slot"
        );
        assert_eq!(block("/r/a/a1/deep").level, 3);
        assert!(
            !l.blocks
                .iter()
                .any(|b| b.kind == BlockKind::Dir(find(&tree, "/r/empty")))
        );
        assert!(
            l.blocks
                .iter()
                .any(|b| matches!(b.kind, BlockKind::File(_, _)) && b.name == "disk.img")
        );
        assert!(
            l.blocks
                .iter()
                .any(|b| b.kind == BlockKind::Files(NodeId::ROOT))
        );
        // Children lie inside their parent, below the header strip.
        for blk in &l.blocks {
            if let Some(p) = blk.parent {
                let parent = &l.blocks[p].rect;
                assert!(
                    blk.rect.x >= parent.x
                        && blk.rect.x + blk.rect.width <= parent.x + parent.width + 0.01
                );
                assert!(
                    blk.rect.y >= parent.y
                        && blk.rect.y + blk.rect.height <= parent.y + parent.height + 0.01
                );
                if l.blocks[p].header {
                    assert!(blk.rect.y >= parent.y + HEADER_H - 0.01);
                }
            }
        }
        // Siblings never overlap.
        let level1: Vec<_> = l.blocks.iter().filter(|b| b.level == 1).collect();
        for (i, x) in level1.iter().enumerate() {
            for y in &level1[i + 1..] {
                assert!(!overlaps(&x.rect, &y.rect));
            }
        }
        // Hit testing finds the deepest block.
        let deep = block("/r/a/a1/deep");
        let center = Point::new(
            deep.rect.x + deep.rect.width / 2.0,
            deep.rect.y + deep.rect.height / 2.0,
        );
        let hit = l.hit(center).unwrap();
        assert_eq!(l.blocks[hit].kind, deep.kind);
    }

    #[test]
    fn small_areas_are_not_subdivided_and_tiny_children_skipped() {
        let tree = sample();
        let slots = Slots::default();
        let l = layout(&tree, NodeId::ROOT, Size::new(90.0, 60.0), &slots);
        assert!(
            l.blocks.iter().all(|b| b.level <= 2),
            "{:?}",
            l.blocks.iter().map(|b| b.level).collect::<Vec<_>>()
        );
        assert!(
            l.blocks
                .iter()
                .all(|b| b.rect.width >= MIN_SIDE && b.rect.height >= MIN_SIDE)
        );
        assert!(
            layout(&tree, NodeId::ROOT, Size::new(2.0, 2.0), &slots)
                .blocks
                .is_empty()
        );
    }

    #[test]
    fn many_children_end_in_one_more_block() {
        let mut root = dir("r");
        for i in 0..250 {
            root = root.child(dir(format!("d{i:03}")).small_files(1, 1_000_000 + i * 1000));
        }
        let tree = TreeBuilder::new("/r", root).build();
        let e = entries(&tree, NodeId::ROOT);
        assert_eq!(e.len(), MAX_CHILDREN);
        assert!(matches!(e.last().unwrap().0, BlockKind::More(_, 51)));
        assert_eq!(e.last().unwrap().2, "51 more");
    }

    #[test]
    fn slots_stick_to_folders_and_reset_on_zoom() {
        let mut tree = sample();
        let mut slots = Slots::default();
        slots.update(&tree, NodeId::ROOT);
        let (a, b, c, d) = (
            find(&tree, "/r/a"),
            find(&tree, "/r/b"),
            find(&tree, "/r/c"),
            find(&tree, "/r/d"),
        );
        assert_eq!(slots.ids, [Some(a), Some(b), Some(c)]);
        // `b` is trashed: `d` moves up and takes b's slot; a and c keep theirs.
        tree.mark_trashed(b);
        slots.update(&tree, NodeId::ROOT);
        assert_eq!(slots.ids, [Some(a), Some(d), Some(c)]);
        slots.update(&tree, a);
        assert_eq!(
            slots.slot_of(find(&tree, "/r/a/a1")),
            Some(0),
            "new zoom root, new slots"
        );
    }

    #[test]
    fn layout_of_200_by_3_levels_is_fast() {
        let mut root: Dir = dir("r");
        for i in 0..200 {
            let mut l1 = dir(format!("l1-{i}"));
            for j in 0..20 {
                let mut l2 = dir(format!("l2-{j}"));
                for k in 0..10 {
                    l2 = l2.child(
                        dir(format!("l3-{k}"))
                            .small_files(1, 1_000_000 + (i * 7 + j * 3 + k) as u64 * 1000),
                    );
                }
                l1 = l1.child(l2);
            }
            root = root.child(l1);
        }
        let tree = TreeBuilder::new("/r", root).build();
        let mut slots = Slots::default();
        slots.update(&tree, NodeId::ROOT);
        let started = Instant::now();
        let l = layout(&tree, NodeId::ROOT, Size::new(480.0, 700.0), &slots);
        let took = started.elapsed();
        assert!(!l.blocks.is_empty());
        // The acceptance budget is 8 ms in release; debug builds get some slack.
        let budget = if cfg!(debug_assertions) { 80 } else { 8 };
        assert!(
            took < Duration::from_millis(budget),
            "{took:?} for {} blocks",
            l.blocks.len()
        );
    }

    #[test]
    fn relayout_waits_for_a_second_and_for_the_pointer() {
        let t0 = Instant::now();
        let s = |ms: u64| t0 + Duration::from_millis(ms);
        let mut clock = MapClock::default();
        assert!(!clock.tick(s(0), true, false), "nothing changed");
        clock.dirty = true;
        assert!(clock.tick(s(0), true, false));
        clock.dirty = true;
        assert!(!clock.tick(s(500), true, false), "at most once a second");
        assert!(clock.tick(s(1000), true, false));
        clock.dirty = true;
        assert!(!clock.tick(s(2500), true, true), "pointer over the map");
        assert!(!clock.tick(s(6000), true, true));
        assert!(clock.tick(s(7500), true, true), "deferred at most 5 s");
        clock.dirty = true;
        assert!(clock.tick(s(7600), false, false), "after the scan: at once");
        assert_eq!(clock.version, 4);
    }
}
