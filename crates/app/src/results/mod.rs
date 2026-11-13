//! The Results screen: the tree of one scan, its live state and the views around it.

pub mod activity;
pub mod columns;
pub mod errors;
pub mod nav;
pub mod order;
pub mod rows;
pub mod scan_task;
pub mod selection_bar;
pub mod tree_view;

use std::path::PathBuf;
use std::time::Instant;

use dirstat_core::scan::{StartError, Summary, Update};
use dirstat_core::tree::Tree;
use dirstat_core::volumes::ScanEstimate;

use activity::ActivityModel;
use order::Column;
use rows::RowKey;
use scan_task::{ActiveScan, ScanId};
use tree_view::{Effect, TreeView};

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
}

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
        self.active.is_none() && matches!(self.phase, Phase::Complete | Phase::Stopped)
    }

    pub fn tick(&mut self, now: Instant, button_held: bool) {
        if !self.tree.is_started() {
            return;
        }
        if self.active.is_some() {
            self.activity.refresh_path(&self.tree, now);
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
