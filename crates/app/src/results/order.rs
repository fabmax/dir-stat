//! Calm sorting (UX §5.4, design §6.7): values update in place, sibling order changes only on
//! a slow tick that waits while the user interacts, with ~10% hysteresis.

use std::cmp::Ordering;
use std::time::{Duration, Instant};

use dirstat_core::tree::{FileFlags, Node, NodeFlags, NodeId, Tree};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Name,
    Size,
    Items,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub column: Column,
    pub descending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Sort {
            column: Column::Size,
            descending: true,
        }
    }
}

impl Sort {
    /// A header click: the same column flips; a new one starts A→Z for names, largest first
    /// for sizes and counts.
    pub fn clicked(self, column: Column) -> Sort {
        if column == self.column {
            Sort {
                column,
                descending: !self.descending,
            }
        } else {
            Sort {
                column,
                descending: column != Column::Name,
            }
        }
    }
}

/// One entry among a folder's children.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChildKey {
    Dir(NodeId),
    /// Index into the folder's large files.
    File(u32),
    /// "≡ 1,234 files".
    Aggregate,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChildOrder {
    pub keys: Vec<ChildKey>,
}

/// Listed individually: files ≥ 1% of the folder or ≥ 100 MB, at most 10 (UX §6.3).
const SHOWN_FILES: usize = 10;
const BIG_FILE: u64 = 100_000_000;

/// How a folder's files are displayed: some individually, the rest as one aggregate row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileSplit {
    /// Large-file indices listed individually, largest first.
    pub shown: Vec<u32>,
    /// The remaining kept large files, largest first (listed when the aggregate expands).
    pub kept_rest: Vec<u32>,
    pub aggregate_count: u64,
    pub aggregate_alloc: u64,
}

pub fn split_files(node: &Node) -> FileSplit {
    let files = node.large_files();
    let mut by_size: Vec<u32> = (0..files.len() as u32).collect();
    by_size.sort_by_key(|&i| std::cmp::Reverse(files[i as usize].alloc));
    let total = node.total().alloc;
    let (mut shown, mut kept_rest) = (Vec::new(), Vec::new());
    for i in by_size {
        let alloc = files[i as usize].alloc;
        let notable = alloc >= BIG_FILE || alloc.saturating_mul(100) >= total;
        if notable && shown.len() < SHOWN_FILES {
            shown.push(i);
        } else {
            kept_rest.push(i);
        }
    }
    // Trashed files left the folder's counts already; they stay listed until a re-order.
    let shown_live = shown
        .iter()
        .filter(|&&i| !files[i as usize].flags.contains(FileFlags::TRASHED));
    let (count, alloc) = shown_live.fold((0u64, 0u64), |(c, a), &i| {
        (c + 1, a + files[i as usize].alloc)
    });
    FileSplit {
        shown,
        kept_rest,
        aggregate_count: node.files().count.saturating_sub(count),
        aggregate_alloc: node.files().alloc.saturating_sub(alloc),
    }
}

/// The current set of children. Trashed ones are gone: a re-order drops their rows, which
/// stay (struck through) only until then.
fn child_keys(tree: &Tree, id: NodeId, split: &FileSplit) -> Vec<ChildKey> {
    let mut keys: Vec<ChildKey> = tree
        .children(id)
        .filter(|&c| !tree.node(c).has_flag(NodeFlags::TRASHED))
        .map(ChildKey::Dir)
        .collect();
    let files = tree.node(id).large_files();
    keys.extend(
        split
            .shown
            .iter()
            .filter(|&&i| !files[i as usize].flags.contains(FileFlags::TRASHED))
            .map(|&i| ChildKey::File(i)),
    );
    if split.aggregate_count > 0 {
        keys.push(ChildKey::Aggregate);
    }
    keys
}

struct Keyed<'a> {
    tree: &'a Tree,
    node: &'a Node,
    split: FileSplit,
}

impl Keyed<'_> {
    fn size(&self, key: ChildKey) -> u64 {
        match key {
            ChildKey::Dir(id) => self.tree.node(id).total().alloc,
            ChildKey::File(i) => self
                .node
                .large_files()
                .get(i as usize)
                .map_or(0, |f| f.alloc),
            ChildKey::Aggregate => self.split.aggregate_alloc,
        }
    }

    fn items(&self, key: ChildKey) -> u64 {
        match key {
            ChildKey::Dir(id) => self.tree.node(id).total().items,
            ChildKey::File(_) => 0,
            ChildKey::Aggregate => self.split.aggregate_count,
        }
    }

    fn name(&self, key: ChildKey) -> String {
        match key {
            ChildKey::Dir(id) => self.tree.node(id).name().to_string_lossy().into_owned(),
            ChildKey::File(i) => self
                .node
                .large_files()
                .get(i as usize)
                .map_or_else(String::new, |f| f.name.to_string_lossy().into_owned()),
            ChildKey::Aggregate => String::new(),
        }
    }

    /// Exact comparison; names break ties. With name sort the aggregate is always last.
    fn cmp(&self, a: ChildKey, b: ChildKey, sort: Sort) -> Ordering {
        let by_name = || natural_cmp(&self.name(a), &self.name(b));
        let ord = match sort.column {
            Column::Name => match (a == ChildKey::Aggregate, b == ChildKey::Aggregate) {
                (true, false) => return Ordering::Greater,
                (false, true) => return Ordering::Less,
                _ => by_name(),
            },
            Column::Size => self.size(a).cmp(&self.size(b)),
            Column::Items => self.items(a).cmp(&self.items(b)),
        };
        let ord = if sort.descending { ord.reverse() } else { ord };
        ord.then_with(by_name)
    }

    fn bucket_of(&self, key: ChildKey, sort: Sort) -> i32 {
        let value = match sort.column {
            Column::Items => self.items(key),
            _ => self.size(key),
        };
        if sort.descending {
            -bucket(value)
        } else {
            bucket(value)
        }
    }
}

/// About 10% wide: sizes in one bucket never swap on a calm re-order.
pub fn bucket(size: u64) -> i32 {
    if size == 0 {
        -1
    } else {
        ((size as f64).ln() / 1.1f64.ln()).floor() as i32
    }
}

/// When a folder is expanded or its listing arrives: nothing was shown yet, sort exactly.
pub fn exact(tree: &Tree, id: NodeId, sort: Sort) -> ChildOrder {
    let node = tree.node(id);
    let keyed = Keyed {
        tree,
        node,
        split: split_files(node),
    };
    let mut keys = child_keys(tree, id, &keyed.split);
    keys.sort_by(|&a, &b| keyed.cmp(a, b, sort));
    ChildOrder { keys }
}

/// A tick re-order: the previous order is kept within a bucket (stable sort by bucket).
/// Children that appeared meanwhile are placed by the same rule.
pub fn calm(tree: &Tree, id: NodeId, prev: &ChildOrder, sort: Sort) -> ChildOrder {
    if sort.column == Column::Name {
        return exact(tree, id, sort); // names don't change; only new rows need a place
    }
    let node = tree.node(id);
    let keyed = Keyed {
        tree,
        node,
        split: split_files(node),
    };
    let current = child_keys(tree, id, &keyed.split);
    let mut keys: Vec<ChildKey> = prev
        .keys
        .iter()
        .copied()
        .filter(|k| current.contains(k))
        .collect();
    let mut new: Vec<ChildKey> = current
        .into_iter()
        .filter(|k| !prev.keys.contains(k))
        .collect();
    new.sort_by(|&a, &b| keyed.cmp(a, b, sort));
    keys.extend(new);
    keys.sort_by_key(|&k| keyed.bucket_of(k, sort));
    ChildOrder { keys }
}

/// Case-insensitive natural order: "file2" before "file10".
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut x, mut y) = (a.as_bytes(), b.as_bytes());
    loop {
        match (x.first(), y.first()) {
            (None, None) => return a.cmp(&b),
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let n = x.iter().take_while(|c| c.is_ascii_digit()).count();
                let m = y.iter().take_while(|c| c.is_ascii_digit()).count();
                let (p, q) = (trim_zeros(&x[..n]), trim_zeros(&y[..m]));
                let ord = p.len().cmp(&q.len()).then_with(|| p.cmp(q));
                if ord != Ordering::Equal {
                    return ord;
                }
                (x, y) = (&x[n..], &y[m..]);
            }
            (Some(c), Some(d)) => {
                if c != d {
                    return c.cmp(d);
                }
                (x, y) = (&x[1..], &y[1..]);
            }
        }
    }
}

fn trim_zeros(digits: &[u8]) -> &[u8] {
    let zeros = digits.iter().take_while(|&&c| c == b'0').count();
    &digits[zeros.min(digits.len().saturating_sub(1))..]
}

/// Re-order every 1.5 s at most.
const TICK: Duration = Duration::from_millis(1500);
/// No re-order within this long of pointer, key or scroll activity.
const QUIET: Duration = Duration::from_millis(700);
/// A deferred re-order runs at most this late.
const MAX_DEFER: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
pub struct ReorderClock {
    last_tick: Option<Instant>,
    last_activity: Option<Instant>,
    deferred_since: Option<Instant>,
    /// A trash finished: one re-order should run even after the scan.
    pub post_trash: bool,
}

impl ReorderClock {
    pub fn activity(&mut self, now: Instant) {
        self.last_activity = Some(now);
    }

    /// Whether a re-order should run now; if so, it counts as done.
    pub fn due(&mut self, now: Instant, overlay_open: bool, button_held: bool) -> bool {
        if self
            .last_tick
            .is_some_and(|t| now.saturating_duration_since(t) < TICK)
        {
            return false;
        }
        let busy = overlay_open
            || button_held
            || self
                .last_activity
                .is_some_and(|a| now.saturating_duration_since(a) < QUIET);
        if busy {
            let since = *self.deferred_since.get_or_insert(now);
            if now.saturating_duration_since(since) < MAX_DEFER {
                return false;
            }
        }
        self.last_tick = Some(now);
        self.deferred_since = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dirstat_core::tree::testing::{TreeBuilder, dir, large};

    const MB: u64 = 1_000_000;
    const MS: Duration = Duration::from_millis(1);

    fn names(tree: &Tree, order: &ChildOrder) -> Vec<String> {
        let node = tree.node(NodeId::ROOT);
        order
            .keys
            .iter()
            .map(|k| match k {
                ChildKey::Dir(id) => tree.node(*id).name().to_string_lossy().into_owned(),
                ChildKey::File(i) => node.large_files()[*i as usize]
                    .name
                    .to_string_lossy()
                    .into_owned(),
                ChildKey::Aggregate => "≡".into(),
            })
            .collect()
    }

    fn sized(name: &str, bytes: u64) -> dirstat_core::tree::testing::Dir {
        dir(name).dir_alloc(0).small_files(1, bytes)
    }

    #[test]
    fn exact_sorts_by_each_column() {
        let tree = TreeBuilder::new(
            "/r",
            dir("r")
                .dir_alloc(0)
                .child(sized("b10", 30 * MB))
                .child(sized("B2", 50 * MB))
                .child(sized("a", 10 * MB))
                .small_files(3, 1000),
        )
        .build();
        let order = |column, descending| {
            names(
                &tree,
                &exact(&tree, NodeId::ROOT, Sort { column, descending }),
            )
        };
        assert_eq!(order(Column::Size, true), ["B2", "b10", "a", "≡"]);
        assert_eq!(order(Column::Size, false), ["≡", "a", "b10", "B2"]);
        assert_eq!(
            order(Column::Name, false),
            ["a", "B2", "b10", "≡"],
            "natural, case-insensitive"
        );
        assert_eq!(
            order(Column::Name, true),
            ["b10", "B2", "a", "≡"],
            "aggregate last either way"
        );
        assert_eq!(
            order(Column::Items, true)[0],
            "≡",
            "3 files beat 1 item each"
        );
    }

    #[test]
    fn files_shown_individually_and_the_aggregate() {
        // 1% of ~1.2 GB is 12 MB: the 50 MB file is listed, the 5 MB one is not.
        let mut root = dir("r").dir_alloc(0).child(sized("big", 1000 * MB));
        root = root
            .large(large("movie.mov", 150 * MB))
            .large(large("notes.pdf", 50 * MB))
            .large(large("small.bin", 5 * MB));
        let tree = TreeBuilder::new("/r", root.small_files(10, 1000)).build();
        let split = split_files(tree.node(NodeId::ROOT));
        assert_eq!(split.shown.len(), 2);
        assert_eq!(split.kept_rest.len(), 1);
        assert_eq!(split.aggregate_count, 11);
        assert_eq!(split.aggregate_alloc, 5 * MB + 1000);
        assert_eq!(
            names(&tree, &exact(&tree, NodeId::ROOT, Sort::default())),
            ["big", "movie.mov", "notes.pdf", "≡"]
        );
    }

    #[test]
    fn calm_keeps_siblings_within_ten_percent() {
        let builder = TreeBuilder::new(
            "/r",
            dir("r")
                .dir_alloc(0)
                .child(sized("x", 100 * MB))
                .child(sized("y", 105 * MB))
                .child(sized("z", 300 * MB)),
        );
        let tree = builder.build();
        let prev = ChildOrder {
            keys: vec![
                ChildKey::Dir(NodeId(1)),
                ChildKey::Dir(NodeId(2)),
                ChildKey::Dir(NodeId(3)),
            ],
        };
        // y is 5% bigger than x: no flip; z is far bigger: moves up.
        assert_eq!(
            names(&tree, &calm(&tree, NodeId::ROOT, &prev, Sort::default())),
            ["z", "x", "y"]
        );
        assert_eq!(
            names(&tree, &exact(&tree, NodeId::ROOT, Sort::default())),
            ["z", "y", "x"]
        );
    }

    #[test]
    fn calm_places_new_children() {
        let tree = TreeBuilder::new(
            "/r",
            dir("r")
                .dir_alloc(0)
                .child(sized("x", 10 * MB))
                .child(sized("y", 500 * MB)),
        )
        .build();
        let prev = ChildOrder {
            keys: vec![ChildKey::Dir(NodeId(1))],
        };
        assert_eq!(
            names(&tree, &calm(&tree, NodeId::ROOT, &prev, Sort::default())),
            ["y", "x"]
        );
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["file10", "File2", "file1", "a", "file02", "B"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a", "B", "file1", "file02", "File2", "file10"]);
    }

    #[test]
    fn header_clicks() {
        let s = Sort::default();
        assert_eq!(
            s.clicked(Column::Size),
            Sort {
                column: Column::Size,
                descending: false
            }
        );
        assert_eq!(
            s.clicked(Column::Name),
            Sort {
                column: Column::Name,
                descending: false
            }
        );
        assert_eq!(
            s.clicked(Column::Items),
            Sort {
                column: Column::Items,
                descending: true
            }
        );
    }

    #[test]
    fn reorder_waits_for_quiet() {
        let t0 = Instant::now();
        let mut clock = ReorderClock::default();
        assert!(clock.due(t0, false, false));
        assert!(
            !clock.due(t0 + 1000 * MS, false, false),
            "at most every 1.5 s"
        );
        clock.activity(t0 + 1400 * MS);
        assert!(
            !clock.due(t0 + 1600 * MS, false, false),
            "within 700 ms of activity"
        );
        assert!(!clock.due(t0 + 2000 * MS, false, false));
        assert!(clock.due(t0 + 2150 * MS, false, false), "quiet again");
    }

    #[test]
    fn reorder_defers_at_most_five_seconds() {
        let t0 = Instant::now();
        let mut clock = ReorderClock::default();
        assert!(clock.due(t0, false, false));
        // Busy the whole time: a button held, an overlay, constant activity.
        let mut forced = Vec::new();
        for ms in (1500..20_000).step_by(50) {
            let now = t0 + ms * MS;
            clock.activity(now);
            if clock.due(now, ms % 2 == 0, true) {
                forced.push(ms);
            }
        }
        assert_eq!(
            forced,
            [6500, 13_000, 19_500],
            "one forced re-order per 5 s of deferral"
        );
    }
}
