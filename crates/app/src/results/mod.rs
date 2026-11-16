//! The Results screen: the tree of one scan, its live state and the views around it.

pub mod activity;
pub mod candidates;
pub mod cleanup;
pub mod cleanup_pane;
pub mod columns;
pub mod errors;
pub mod migrate;
pub mod nav;
pub mod order;
pub mod rows;
pub mod scan_task;
pub mod selection_bar;
pub mod trash;
pub mod tree_view;

use std::path::PathBuf;
use std::time::Instant;

use std::collections::HashMap;

use dirstat_core::scan::{StartError, Summary, Update};
use dirstat_core::trash::{TrashJob, TrashJobId, TrashOutcome};
use dirstat_core::tree::{FileFlags, NodeFlags, NodeId, Tree};
use dirstat_core::volumes::ScanEstimate;

use activity::ActivityModel;
use order::Column;
use rows::RowKey;
use scan_task::{ActiveScan, ScanId};
use tree_view::{Effect, TreeView};

use crate::format;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The scan thread is opening the root; nothing is known yet.
    Opening,
    Scanning,
    Complete,
    /// Stop was pressed: final at once, even if `Finished` never arrives.
    Stopped,
}

pub struct Results {
    pub tree: Tree,
    pub phase: Phase,
    /// The scan whose updates drive the phase.
    pub active: Option<ActiveScan>,
    /// A stopped scan whose `Finished` hasn't arrived. Its late batches still apply (they
    /// only add data), but it doesn't count as running.
    pub draining: Option<ActiveScan>,
    pub activity: ActivityModel,
    pub estimate: Option<ScanEstimate>,
    pub summary: Option<Summary>,
    /// "Macintosh HD", or the folder name.
    pub title: String,
    pub started_at: Instant,
    pub view: TreeView,
    /// Set when the rows moved under the anchor; the app turns it into a `scroll_to`.
    pub scroll_to: Option<f32>,
    /// The full scan this screen shows; results of background jobs carry it.
    pub scan_id: ScanId,
    /// macOS: the first privacy-blocked folder of this scan raised the Full Disk Access bar.
    pub fda_bar: bool,
    pub errors_popover: bool,
    /// When the scan started: contents-mode trash leaves newer files in place (G9).
    pub scan_started: std::time::SystemTime,
    /// Bumped by every full scan; trash results for another generation don't touch the tree.
    pub tree_gen: u32,
    pub candidates: candidates::Candidates,
    pub trash_jobs: HashMap<TrashJobId, PendingTrash>,
    /// Set by the app when the scan starts; the cleanup pane and the pills need the catalog.
    pub engine: Option<std::sync::Arc<dirstat_core::catalog::Engine>>,
    pub cleanup: cleanup_pane::CleanupPane,
    pub focus: Pane,
    cleanup_dirty: bool,
    cleanup_seen: (u64, NodeId),
    /// A subtree rescan in flight (design §4.6). It never changes `phase`.
    pub rescan: Option<Rescan>,
}

/// A rescan accumulates into its own tree; the main tree keeps the old numbers until the
/// swap.
pub struct Rescan {
    pub active: ActiveScan,
    pub target: NodeId,
    pub sub: Tree,
}

/// Dropping a big tree takes a while; do it off the UI thread.
fn drop_later(tree: Tree) {
    std::thread::spawn(move || drop(tree));
}

/// Which pane has the keyboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pane {
    #[default]
    Tree,
    Cleanup,
}

/// How long a revealed row flashes.
pub const FLASH: std::time::Duration = std::time::Duration::from_millis(600);

/// A trash job in flight: what to account for when it finishes.
#[derive(Clone, Debug)]
pub struct PendingTrash {
    pub target: trash::TrashTarget,
    pub name: String,
    pub frees: u64,
    pub shared_space: bool,
}

/// The toast a finished trash job asks for: text, and whether it stays until closed.
pub type TrashToast = (String, bool);

/// What a scan update means for the screen.
#[derive(Debug)]
pub enum ScanOutcome {
    /// From a scan that was replaced: dropped.
    Ignored,
    Applied,
    /// The root opened: remember it as a recent location.
    Started(PathBuf),
    /// The root could not be opened.
    Failed(StartError),
    /// A subtree rescan couldn't start; the toast text.
    RescanFailed(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum StopOutcome {
    Nothing,
    /// Stopped while opening: there is nothing to show.
    BackToStart,
    Stopped,
}

impl Results {
    pub fn new(
        root: PathBuf,
        title: String,
        estimate: Option<ScanEstimate>,
        active: ActiveScan,
    ) -> Results {
        Results {
            scan_id: active.id,
            fda_bar: false,
            errors_popover: false,
            scan_started: std::time::SystemTime::now(),
            tree_gen: active.id.0 as u32,
            candidates: candidates::Candidates::default(),
            trash_jobs: HashMap::new(),
            engine: None,
            cleanup: cleanup_pane::CleanupPane::default(),
            focus: Pane::Tree,
            cleanup_dirty: true,
            cleanup_seen: (0, NodeId::ROOT),
            rescan: None,
            tree: Tree::opening(root),
            phase: Phase::Opening,
            active: Some(active),
            draining: None,
            activity: ActivityModel::default(),
            estimate,
            summary: None,
            title,
            started_at: Instant::now(),
            view: TreeView::default(),
            scroll_to: None,
        }
    }

    pub fn on_scan(&mut self, id: ScanId, update: Update, now: Instant) -> ScanOutcome {
        if self.rescan.as_ref().is_some_and(|r| r.active.id == id) {
            return self.on_rescan(update, now);
        }
        let from_active = self.active.as_ref().is_some_and(|a| a.id == id);
        let from_draining = self.draining.as_ref().is_some_and(|a| a.id == id);
        if !from_active && !from_draining {
            return ScanOutcome::Ignored;
        }
        match update {
            Update::Started(info) => {
                let root = info.root.clone();
                self.tree.start(info);
                if self.phase == Phase::Opening {
                    self.phase = Phase::Scanning;
                }
                ScanOutcome::Started(root)
            }
            Update::Failed(error) => {
                self.active = None;
                ScanOutcome::Failed(error)
            }
            Update::Batch(batch) => {
                for listing in batch.listings {
                    let id = listing.id;
                    let effect = self.tree.apply(listing);
                    if effect.first_privacy_error && cfg!(target_os = "macos") {
                        self.fda_bar = true;
                    }
                    self.view.on_listing(&self.tree, id, effect);
                }
                if from_active {
                    self.activity.on_progress(batch.progress, now);
                }
                ScanOutcome::Applied
            }
            Update::Finished(summary) => {
                self.tree.finish(&summary);
                if from_active {
                    self.activity.on_progress(summary.progress, now);
                    self.phase = if summary.cancelled {
                        Phase::Stopped
                    } else {
                        Phase::Complete
                    };
                    self.active = None;
                    // Completion: one exact re-sort, anchored to the selection.
                    if self.tree.is_started() {
                        self.scroll_to = self.view.settle(&self.tree, now);
                        self.candidates.collect(&self.tree);
                        self.cleanup_dirty = true;
                    }
                } else {
                    self.draining = None;
                }
                self.summary = Some(summary);
                ScanOutcome::Applied
            }
        }
    }

    /// Stop is final on the UI side at once: a worker blocked in a syscall may delay
    /// `Finished` for minutes (design §4.5).
    pub fn stop(&mut self) -> StopOutcome {
        if let Some(rescan) = self.rescan.take() {
            // Nothing changes: the sub-tree is discarded (design §4.5).
            rescan.active.handle.cancel();
            self.clear_rescanning(rescan.target);
            return StopOutcome::Stopped;
        }
        let Some(active) = self.active.take() else {
            return StopOutcome::Nothing;
        };
        active.handle.cancel();
        if self.phase == Phase::Opening {
            return StopOutcome::BackToStart;
        }
        self.tree.mark_stopped();
        self.phase = Phase::Stopped;
        self.draining = Some(active);
        StopOutcome::Stopped
    }

    pub fn is_scanning(&self) -> bool {
        self.active.is_some()
    }

    /// Rescan only needs no scan in flight; a draining one doesn't count.
    pub fn can_rescan(&self) -> bool {
        self.active.is_none()
            && self.rescan.is_none()
            && matches!(self.phase, Phase::Complete | Phase::Stopped)
    }

    pub fn is_rescanning(&self) -> bool {
        self.rescan.is_some()
    }

    /// A rescan runs at or below `id`: it can't be moved to the Trash meanwhile.
    pub fn rescanning_within(&self, id: NodeId) -> bool {
        self.rescan
            .as_ref()
            .is_some_and(|r| self.tree.is_within(r.target, id))
    }

    /// The selected folder, or the zoom root if a file or nothing is selected (UX §6.6).
    pub fn rescan_target(&self) -> NodeId {
        match self.view.selection {
            Some(RowKey::Dir(id)) if self.tree.get(id).is_some() => id,
            _ => self.view.zoom,
        }
    }

    /// Why `target` can't be rescanned now (design §4.6), or `None`.
    pub fn rescan_refusal(&self, target: NodeId) -> Option<&'static str> {
        use dirstat_core::tree::NodeState;
        if !self.can_rescan() {
            return Some("Available when the scan ends.");
        }
        let node = self.tree.get(target)?;
        let busy = std::iter::once(target)
            .chain(self.tree.ancestors(target))
            .any(|a| {
                let f = self.tree.node(a).flags();
                f.contains(NodeFlags::TRASHED) || f.contains(NodeFlags::TRASH_PENDING)
            });
        if busy {
            return Some("Already in the Trash or being moved.");
        }
        if self
            .trash_jobs
            .values()
            .any(|job| self.tree.is_within(job.target.dir(), target))
        {
            return Some("Available when the move to the Trash finishes.");
        }
        if let NodeState::Skipped(_) = node.state() {
            return Some("This folder isn't part of the scan.");
        }
        None
    }

    /// The request for a rescan of `target`: a subtree gets its catalog context and the
    /// linked inodes counted elsewhere (D9); the root is scanned like the first time.
    pub fn rescan_request(
        &self,
        engine: &dirstat_core::catalog::Engine,
        target: NodeId,
    ) -> dirstat_core::scan::ScanRequest {
        use dirstat_core::scan::ScanRequest;
        let path = self.tree.path_of(target);
        if target == NodeId::ROOT {
            return ScanRequest::new(path);
        }
        ScanRequest {
            context: Some(self.tree.context_for(engine, target)),
            counted_elsewhere: Some(std::sync::Arc::new(self.tree.counted_elsewhere(target))),
            ..ScanRequest::new(path)
        }
    }

    /// The rescan has been launched: mark the target, keep the old numbers on screen.
    pub fn begin_rescan(&mut self, target: NodeId, active: ActiveScan) {
        // Starting any new scan drops a draining one (design §4.5).
        self.draining = None;
        self.tree.set_flag(target, NodeFlags::RESCANNING, true);
        self.activity = ActivityModel::default();
        let sub = Tree::opening(self.tree.path_of(target));
        self.rescan = Some(Rescan {
            active,
            target,
            sub,
        });
    }

    fn clear_rescanning(&mut self, target: NodeId) {
        if self.tree.get(target).is_some() {
            self.tree.set_flag(target, NodeFlags::RESCANNING, false);
        }
    }

    fn on_rescan(&mut self, update: Update, now: Instant) -> ScanOutcome {
        let Some(rescan) = self.rescan.as_mut() else {
            return ScanOutcome::Ignored;
        };
        match update {
            Update::Started(info) => rescan.sub.start(info),
            Update::Batch(batch) => {
                for listing in batch.listings {
                    rescan.sub.apply(listing);
                }
                self.activity.on_progress(batch.progress, now);
            }
            Update::Failed(error) => {
                let rescan = self.rescan.take().expect("checked above");
                let name = self
                    .tree
                    .node(rescan.target)
                    .name()
                    .to_string_lossy()
                    .into_owned();
                self.clear_rescanning(rescan.target);
                return ScanOutcome::RescanFailed(match error.kind {
                    std::io::ErrorKind::NotFound => {
                        "Folder no longer exists — rescan the parent".into()
                    }
                    _ => format!("Couldn't rescan “{name}”: {}", error.message),
                });
            }
            Update::Finished(summary) => {
                let Rescan {
                    active,
                    target,
                    mut sub,
                } = self.rescan.take().expect("checked above");
                if summary.cancelled || !sub.is_started() {
                    self.clear_rescanning(target);
                } else {
                    sub.finish(&summary);
                    self.swap_in(target, sub, active.id, summary, now);
                }
            }
        }
        ScanOutcome::Applied
    }

    /// Everything swaps in one step with one re-order (UX §6.6, design §4.6).
    fn swap_in(&mut self, target: NodeId, sub: Tree, id: ScanId, summary: Summary, now: Instant) {
        let saved = migrate::SavedView::capture(self, target);
        let mut replaced = false;
        if target == NodeId::ROOT {
            drop_later(std::mem::replace(&mut self.tree, sub));
            replaced = true;
            self.phase = Phase::Complete;
            self.summary = Some(summary);
        } else {
            self.tree.splice(target, sub);
            self.tree.set_flag(target, NodeFlags::RESCANNING, false);
            // Compaction changes every id: never while a trash job holds some.
            if self.tree.needs_compaction() && self.trash_jobs.is_empty() {
                let compacted = self.tree.compacted();
                drop_later(std::mem::replace(&mut self.tree, compacted));
                replaced = true;
            }
            if self.phase == Phase::Stopped && self.tree.node(NodeId::ROOT).is_done() {
                self.phase = Phase::Complete;
            }
        }
        if replaced {
            // Late git and small-file results carry the old id and are dropped.
            self.scan_id = id;
            self.tree_gen = id.0 as u32;
        }
        self.scroll_to = Some(saved.restore(self, replaced, now));
        self.cleanup_dirty = true;
    }

    pub fn tick(&mut self, now: Instant, button_held: bool) {
        if !self.tree.is_started() {
            return;
        }
        if self.active.is_some() {
            self.activity.refresh_path(&self.tree, now);
        } else if let Some(rescan) = &self.rescan
            && rescan.sub.is_started()
        {
            self.activity.refresh_path(&rescan.sub, now);
        }
        // While scanning the pane re-ranks with the tree's re-orders; afterwards (late
        // listings of a stopped scan) a new candidate shows at once.
        if self.candidates.collect(&self.tree) && self.active.is_none() {
            self.cleanup_dirty = true;
        }
        if let Some(offset) = self
            .view
            .tick(&self.tree, now, self.active.is_some(), button_held)
        {
            self.scroll_to = Some(offset);
        }
    }

    /// Takes the scroll request out of an effect and hands back the rest.
    fn effect(&mut self, effect: Effect) -> Effect {
        if effect.scroll.is_some() {
            self.scroll_to = effect.scroll;
        }
        effect
    }

    pub fn toggle(&mut self, key: RowKey, now: Instant) -> Effect {
        if !self.tree.is_started() {
            return Effect::default();
        }
        let effect = self.view.toggle(&mut self.tree, key, now);
        self.effect(effect)
    }

    pub fn press(&mut self, key: RowKey, now: Instant) -> Effect {
        let effect = self.view.press(&self.tree, key, now);
        self.effect(effect)
    }

    pub fn activate(&mut self, key: RowKey, now: Instant) -> Effect {
        let effect = self.view.activate(&mut self.tree, key, now);
        self.effect(effect)
    }

    pub fn key(&mut self, action: crate::keys::KeyAction, now: Instant) -> Effect {
        if !self.tree.is_started() {
            return Effect::default();
        }
        let effect = self.view.key(&mut self.tree, action, now);
        self.effect(effect)
    }

    pub fn zoom_to(&mut self, id: dirstat_core::tree::NodeId, now: Instant) {
        // Zooming out to an ancestor selects the folder on the way, as in Finder.
        let from = self
            .tree
            .ancestors(self.view.zoom)
            .take_while(|&a| a != id)
            .last()
            .unwrap_or(self.view.zoom);
        let select =
            (self.tree.is_within(self.view.zoom, id) && from != id).then_some(RowKey::Dir(from));
        self.scroll_to = self
            .view
            .zoom_to(&self.tree, id, select, now)
            .or(self.scroll_to);
    }

    pub fn small_files_listed(
        &mut self,
        dir: dirstat_core::tree::NodeId,
        result: Result<Vec<dirstat_core::smallfiles::SmallFile>, String>,
        now: Instant,
    ) {
        if self.tree.get(dir).is_some() {
            self.scroll_to = self
                .view
                .small_files_listed(&self.tree, dir, result, now)
                .or(self.scroll_to);
        }
    }

    pub fn reveal_in_tree(&mut self, id: dirstat_core::tree::NodeId, now: Instant) {
        self.errors_popover = false;
        if self.tree.is_started() {
            self.scroll_to = self
                .view
                .reveal_node(&mut self.tree, id, now)
                .or(self.scroll_to);
        }
    }

    /// Marks the target pending and records the job: only the confirm button gets here.
    pub fn start_trash(&mut self, plan: &trash::TrashPlan, id: TrashJobId) -> TrashJob {
        let target = plan.target;
        match target {
            trash::TrashTarget::Dir(dir) => self.tree.set_flag(dir, NodeFlags::TRASH_PENDING, true),
            trash::TrashTarget::File(dir, index) => {
                self.tree
                    .set_file_flag(dir, index, FileFlags::TRASH_PENDING, true)
            }
            trash::TrashTarget::SmallFile(dir, index) => {
                if let Some(file) = self.small_file_mut(dir, index) {
                    file.flags = FileFlags::TRASH_PENDING;
                }
            }
        }
        self.trash_jobs.insert(
            id,
            PendingTrash {
                target,
                name: plan.name.clone(),
                frees: plan.frees,
                shared_space: plan.shared_space,
            },
        );
        TrashJob {
            id,
            tree_gen: self.tree_gen,
            scan_root: self.tree.root_path().to_path_buf(),
            items: plan.items.clone(),
        }
    }

    fn small_file_mut(
        &mut self,
        dir: NodeId,
        index: u32,
    ) -> Option<&mut dirstat_core::smallfiles::SmallFile> {
        match &mut self.view.open_aggregates.get_mut(&dir)?.small {
            rows::SmallFiles::Loaded(files) => files.get_mut(index as usize),
            _ => None,
        }
    }

    /// Accounts for a finished trash job (design §7.4). Sizes leave the ancestors at once;
    /// the struck-through row stays until the next re-order.
    pub fn trash_finished(
        &mut self,
        id: TrashJobId,
        generation: u32,
        outcomes: Vec<TrashOutcome>,
        home: Option<&std::path::Path>,
        now: Instant,
    ) -> Option<TrashToast> {
        let pending = self.trash_jobs.remove(&id)?;
        if generation != self.tree_gen {
            return Some((
                "The Trash finished for an earlier scan; rescan to see the change.".into(),
                false,
            ));
        }
        let name = &pending.name;
        let mut toast = None;
        for outcome in outcomes {
            match outcome {
                TrashOutcome::Trashed(_) => {
                    match pending.target {
                        trash::TrashTarget::Dir(dir) => self.tree.mark_trashed(dir),
                        trash::TrashTarget::File(dir, index) => {
                            self.tree.mark_file_trashed(dir, index)
                        }
                        trash::TrashTarget::SmallFile(dir, index) => {
                            if let Some(file) = self.small_file_mut(dir, index) {
                                file.flags = FileFlags::TRASHED;
                                let (alloc, apparent, linked) =
                                    (file.alloc, file.apparent, file.nlink > 1);
                                self.tree.subtract_small_file(dir, alloc, apparent, linked);
                            }
                        }
                    }
                    let up_to = if pending.shared_space { "up to " } else { "" };
                    toast = Some((
                        format!(
                            "Moved “{name}” to the Trash. Empty the Trash to free {up_to}{}.",
                            format::size(pending.frees)
                        ),
                        false,
                    ));
                }
                TrashOutcome::Contents { outcome, .. } => {
                    let dir = pending.target.dir();
                    self.tree.subtract_contents(dir, &outcome);
                    self.tree.set_flag(dir, NodeFlags::TRASH_PENDING, false);
                    let dirs: u64 = outcome
                        .trashed_dirs
                        .iter()
                        .filter_map(|n| {
                            self.tree
                                .children(dir)
                                .find(|&c| self.tree.node(c).name() == &**n)
                        })
                        .map(|c| self.tree.node(c).total().alloc)
                        .sum();
                    let mut text = format!(
                        "Moved the contents of “{name}” to the Trash. Empty the Trash to free {}.",
                        format::size(dirs + outcome.files_alloc)
                    );
                    if outcome.left_in_place > 0 {
                        text += &format!(
                            " {} changed since the scan and stayed.",
                            format::plural(u64::from(outcome.left_in_place), "item", "items")
                        );
                    }
                    let sticky = !outcome.failures.is_empty();
                    if let Some((path, error)) = outcome.failures.first() {
                        text += &format!(
                            " {}",
                            error.message(&path.file_name().unwrap_or_default().to_string_lossy())
                        );
                    }
                    toast = Some((text, sticky));
                }
                TrashOutcome::Failed(_, error) => {
                    match pending.target {
                        trash::TrashTarget::Dir(dir) => {
                            self.tree.set_flag(dir, NodeFlags::TRASH_PENDING, false)
                        }
                        trash::TrashTarget::File(dir, index) => {
                            self.tree
                                .set_file_flag(dir, index, FileFlags::TRASH_PENDING, false)
                        }
                        trash::TrashTarget::SmallFile(dir, index) => {
                            if let Some(file) = self.small_file_mut(dir, index) {
                                file.flags = FileFlags::default();
                            }
                        }
                    }
                    return Some((error.message(name), true));
                }
            }
        }
        // The struck-through row leaves at the next calm re-order, also after the scan.
        self.view.reorder.post_trash = true;
        let candidate = match pending.target {
            trash::TrashTarget::Dir(dir) => Some(dirstat_core::tree::CandidateRef::Dir(dir)),
            trash::TrashTarget::File(dir, index) => {
                Some(dirstat_core::tree::CandidateRef::File(dir, index))
            }
            trash::TrashTarget::SmallFile(..) => None,
        };
        if let Some(candidate) = candidate {
            self.cleanup.keep_trashed.insert(candidate);
        }
        self.cleanup_dirty = true;
        if let Some(trash_node) = home.and_then(|h| self.tree.find_path(&h.join(".Trash"))) {
            self.tree.set_flag(trash_node, NodeFlags::OUTDATED, true);
        }
        // Zoomed into what was just trashed: back to the nearest folder that is still there.
        if let trash::TrashTarget::Dir(dir) = pending.target
            && self.tree.is_within(self.view.zoom, dir)
            && let Some(parent) = self.tree.node(dir).parent()
        {
            self.scroll_to = self
                .view
                .zoom_to(&self.tree, parent, Some(RowKey::Dir(dir)), now);
            return Some((
                format!("“{name}” was moved to the Trash; showing the folder it was in."),
                false,
            ));
        }
        self.scroll_to = self.view.rebuild(&self.tree, now).or(self.scroll_to);
        toast
    }

    /// Ticks are needed while git or trash work is in flight, or a re-order is due.
    pub fn busy(&self) -> bool {
        let flashing = self.view.flash.is_some_and(|(_, at)| at.elapsed() < FLASH);
        flashing
            || self.is_scanning()
            || self.is_rescanning()
            || self.candidates.busy()
            || !self.trash_jobs.is_empty()
            || self.view.reorder.post_trash
    }

    pub fn git_checked(
        &mut self,
        statuses: Vec<(
            dirstat_core::tree::CandidateRef,
            dirstat_core::git::GitStatus,
        )>,
    ) {
        self.candidates.finished(statuses);
        self.cleanup_dirty = true;
    }

    /// Rebuilds the cleanup model when something it shows changed: a calm re-order (the
    /// trashed rows leave then), a new zoom root, git results, trash, completion.
    pub fn refresh_cleanup(&mut self, home: Option<&std::path::Path>, now_unix: i64) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let seen = (self.view.reorders, self.view.zoom);
        if seen.0 != self.cleanup_seen.0 {
            self.cleanup.keep_trashed.clear();
        }
        if !self.cleanup_dirty && seen == self.cleanup_seen {
            return;
        }
        self.cleanup_dirty = false;
        self.cleanup_seen = seen;
        let candidates = &self.candidates;
        self.cleanup.model = cleanup::build(
            &self.tree,
            &engine,
            |c| candidates.state(c),
            self.view.zoom,
            home,
            now_unix,
            self.is_scanning(),
            &self.cleanup.keep_trashed,
        );
        if let Some(cursor) = self.cleanup.cursor
            && !self.cleanup.lines().contains(&cursor)
        {
            self.cleanup.cursor = None;
        }
    }

    /// A cleanup row was clicked or activated: show it in the tree, centered, flashing.
    pub fn reveal_candidate(&mut self, candidate: dirstat_core::tree::CandidateRef, now: Instant) {
        let dir = candidate.dir();
        let target = match candidate {
            dirstat_core::tree::CandidateRef::Dir(id) => id,
            dirstat_core::tree::CandidateRef::File(..) => dir,
        };
        if self.tree.is_started() {
            self.scroll_to = self
                .view
                .reveal_node(&mut self.tree, target, now)
                .or(self.scroll_to);
        }
        let key = match candidate {
            dirstat_core::tree::CandidateRef::File(dir, index)
                if self
                    .view
                    .rows
                    .iter()
                    .any(|r| r.key == RowKey::File(dir, index)) =>
            {
                RowKey::File(dir, index)
            }
            _ => RowKey::Dir(target),
        };
        self.view.selection = Some(key);
        self.view.flash = Some((key, now));
    }

    /// The path a row stands for (an aggregate or placeholder: its folder).
    pub fn path_for(&self, key: RowKey) -> Option<PathBuf> {
        let dir = self.tree.get(key.dir())?;
        let dir_path = self.tree.path_of(key.dir());
        Some(match key {
            RowKey::File(_, index) => dir_path.join(&*dir.large_files().get(index as usize)?.name),
            RowKey::SmallFile(id, index) => match &self.view.open_aggregates.get(&id)?.small {
                rows::SmallFiles::Loaded(files) => dir_path.join(&*files.get(index as usize)?.name),
                _ => return None,
            },
            _ => dir_path,
        })
    }

    pub fn sort_by(&mut self, column: Column, now: Instant) {
        if self.tree.is_started() {
            self.scroll_to = self
                .view
                .sort_by(&self.tree, column, now)
                .or(self.scroll_to);
        }
    }
}
