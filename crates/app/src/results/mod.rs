//! The Results screen: the tree of one scan, its live state and the views around it.

pub mod activity;
pub mod columns;
pub mod order;
pub mod rows;
pub mod scan_task;
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
use tree_view::TreeView;

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

    pub fn toggle(&mut self, key: RowKey, now: Instant) {
        if self.tree.is_started() {
            self.scroll_to = self
                .view
                .toggle(&mut self.tree, key, now)
                .or(self.scroll_to);
        }
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
