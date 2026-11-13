//! The Results screen: the tree of one scan, its live state and the views around it.

pub mod activity;
pub mod scan_task;

use std::path::PathBuf;
use std::time::Instant;

use dirstat_core::scan::{StartError, Summary, Update};
use dirstat_core::tree::{NodeId, NodeState, Tree};
use dirstat_core::volumes::ScanEstimate;
use iced::widget::{Column, column, container, row, scrollable, text};
use iced::{Center, Element, Fill};

use crate::app::Message;
use crate::theme;
use crate::{fonts, format, widgets};
use activity::ActivityModel;
use scan_task::{ActiveScan, ScanId};

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
                    self.tree.apply(listing);
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

    pub fn tick(&mut self, now: Instant) {
        if self.active.is_some() && self.tree.is_started() {
            self.activity.refresh_path(&self.tree, now);
        }
    }
}

/// Temporary (M4): the root's direct children, unsorted, so progress can be seen. The real
/// tree view replaces it.
pub fn top_level_list(results: &Results) -> Element<'_, Message> {
    let tree = &results.tree;
    if !tree.is_started() {
        return container(
            theme::BODY
                .text(format!("Reading {}…", tree.root_path().display()))
                .style(theme::text_secondary),
        )
        .padding(16)
        .into();
    }
    let header = row![
        theme::SMALL
            .with_font(fonts::MEDIUM)
            .text("Name")
            .width(Fill)
            .style(theme::text_secondary),
        theme::SMALL
            .with_font(fonts::MEDIUM)
            .text("Size")
            .width(110)
            .align_x(iced::Right)
            .style(theme::text_secondary),
        theme::SMALL
            .with_font(fonts::MEDIUM)
            .text("Items")
            .width(90)
            .align_x(iced::Right)
            .style(theme::text_secondary),
    ]
    .padding([0, 16])
    .height(28)
    .align_y(Center);

    let rows = tree.children(NodeId::ROOT).take(500).map(|id| {
        let node = tree.node(id);
        let (marker, size, items) = match node.state() {
            NodeState::Pending => (
                "○",
                format::PENDING.to_string(),
                format::PENDING.to_string(),
            ),
            NodeState::Skipped(_) => ("", format::PENDING.to_string(), "other volume".into()),
            state => {
                let size = if tree.is_lower_bound(id) {
                    format::size_at_least(node.total().alloc)
                } else {
                    format::size(node.total().alloc)
                };
                (
                    if state == NodeState::Scanning && !tree.is_stopped() {
                        "•"
                    } else {
                        ""
                    },
                    size,
                    format::count(node.total().items),
                )
            }
        };
        let done = node.is_done();
        row![
            theme::BODY
                .text(widgets::truncate_end(
                    &node.name().to_string_lossy(),
                    520.0,
                    theme::BODY.size
                ))
                .width(Fill)
                .wrapping(text::Wrapping::None),
            theme::BODY
                .text(marker)
                .style(move |t| if marker == "○" {
                    theme::text_muted(t)
                } else {
                    theme::text_accent(t)
                })
                .width(14),
            theme::BODY
                .text(size)
                .width(96)
                .align_x(iced::Right)
                .style(move |t| if done {
                    theme::text_primary(t)
                } else {
                    theme::text_secondary(t)
                }),
            theme::SMALL
                .text(items)
                .width(90)
                .align_x(iced::Right)
                .style(theme::text_secondary),
        ]
        .padding([0, 16])
        .height(28)
        .align_y(Center)
        .into()
    });
    let list = scrollable(Column::with_children(rows).width(Fill)).height(Fill);
    container(column![header, list])
        .width(Fill)
        .height(Fill)
        .style(theme::surface)
        .into()
}
