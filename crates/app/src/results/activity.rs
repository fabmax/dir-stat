//! The activity field: the one place for scan status (UX §5.2). The model and the line texts
//! are pure; `view` only lays them out.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use dirstat_core::scan::Progress;
use dirstat_core::tree::{NodeId, Tree};
use dirstat_core::volumes::ScanEstimate;
use iced::widget::{Space, button, column, container, row, space, text};
use iced::{Center, Element, Fill, FillPortion, Length};

use super::{Phase, Results};
use crate::app::Message;
use crate::theme::{self, TextRole};
use crate::{format, widgets};

/// Items/s are averaged over about this long.
const RATE_WINDOW: Duration = Duration::from_secs(3);
/// The current path changes at most 5 times a second, so it stays legible.
const PATH_REFRESH: Duration = Duration::from_millis(200);
/// A job running this long is what the scan is waiting on.
const WAITING_AFTER: Duration = Duration::from_secs(1);
/// One sweep of the indeterminate progress line.
const SWEEP_PERIOD: Duration = Duration::from_millis(1400);

#[derive(Debug, Default)]
pub struct ActivityModel {
    pub progress: Progress,
    rate: Option<f64>,
    last_sample: Option<(Instant, u64)>,
    /// Relative to the scan root, and whether the scan is waiting on it.
    current: Option<(PathBuf, bool)>,
    last_path_refresh: Option<Instant>,
}

impl ActivityModel {
    pub fn on_progress(&mut self, progress: Progress, now: Instant) {
        let items = items_of(&progress);
        if let Some((at, before)) = self.last_sample {
            let dt = now.saturating_duration_since(at).as_secs_f64();
            if dt > 0.0 {
                let instant = items.saturating_sub(before) as f64 / dt;
                let alpha = 1.0 - (-dt / RATE_WINDOW.as_secs_f64()).exp();
                self.rate = Some(self.rate.map_or(instant, |r| r + alpha * (instant - r)));
            }
        }
        self.last_sample = Some((now, items));
        self.progress = progress;
    }

    /// Picks up the in-flight folder, at most every 200 ms. An id not in the tree yet keeps
    /// the previous path.
    pub fn refresh_path(&mut self, tree: &Tree, now: Instant) {
        if self
            .last_path_refresh
            .is_some_and(|t| now.saturating_duration_since(t) < PATH_REFRESH)
        {
            return;
        }
        self.last_path_refresh = Some(now);
        let Some((id, running)) = self.progress.current else {
            return;
        };
        if let Some(rel) = tree
            .get(id)
            .and_then(|_| tree.relative_path(id, NodeId::ROOT))
        {
            self.current = Some((rel.iter().map(|n| &**n).collect(), running >= WAITING_AFTER));
        }
    }

    pub fn items(&self) -> u64 {
        items_of(&self.progress)
    }

    pub fn rate(&self) -> Option<f64> {
        self.rate
    }
}

fn items_of(p: &Progress) -> u64 {
    p.dirs_listed + p.files
}

/// The texts of the field in one state.
#[derive(Debug, PartialEq)]
pub struct Lines {
    pub line1: String,
    pub line2: String,
    /// Shown at the end of line 2, muted and middle-truncated.
    pub path: Option<String>,
    pub button: &'static str,
}

pub fn lines(results: &Results, title: &str) -> Lines {
    let a = &results.activity;
    let p = &a.progress;
    match results.phase {
        Phase::Opening => Lines {
            line1: format!("Opening {title}…"),
            line2: String::new(),
            path: None,
            button: "Stop",
        },
        Phase::Scanning => {
            let mut line1 = format!("Scanning {title}");
            if let Some(fraction) = progress_fraction(results.estimate.as_ref(), p) {
                line1 += &format!(" · {}%", (fraction * 100.0).floor() as u64);
            }
            let mut stats = vec![
                format::plural(a.items(), "item", "items"),
                format::size(p.alloc),
            ];
            if let Some(rate) = a.rate() {
                stats.push(format::rate(rate));
            }
            stats.push(format::elapsed(p.elapsed));
            if p.errors > 0 {
                stats.push(format!("{} unreadable", format::thousands(p.errors)));
            }
            let path = a.current.as_ref().map(|(rel, waiting)| {
                let shown = if rel.as_os_str().is_empty() {
                    ".".into()
                } else {
                    rel.display().to_string()
                };
                if *waiting {
                    format!("Waiting on {shown}…")
                } else {
                    shown
                }
            });
            Lines {
                line1,
                line2: stats.join(" · "),
                path,
                button: "Stop",
            }
        }
        Phase::Complete => {
            let total = results.tree.node(NodeId::ROOT).total();
            let elapsed = results
                .summary
                .as_ref()
                .map_or(p.elapsed, |s| s.progress.elapsed);
            let mut line2 = format!("Scanned in {}", format::elapsed(elapsed));
            let unreadable =
                total.error_dirs + u32::from(results.tree.node(NodeId::ROOT).error().is_some());
            if unreadable > 0 {
                line2 += &format!(
                    " · {} couldn't be read",
                    format::plural(u64::from(unreadable), "folder", "folders")
                );
            }
            Lines {
                line1: format!(
                    "{title} · {} · {}",
                    format::size(total.alloc),
                    format::plural(total.items, "item", "items")
                ),
                line2,
                path: None,
                button: "Rescan",
            }
        }
        Phase::Stopped => {
            let (alloc, items) = if results.tree.is_started() {
                let total = results.tree.node(NodeId::ROOT).total();
                (total.alloc, total.items)
            } else {
                (0, 0)
            };
            Lines {
                line1: format!(
                    "Scan stopped · {} · {}",
                    format::size_at_least(alloc),
                    format::plural(items, "item", "items")
                ),
                line2: "Sizes marked ≥ are incomplete".into(),
                path: None,
                button: "Rescan",
            }
        }
    }
}

/// Determinate progress for volume roots; `None` means the indeterminate sweep.
pub fn progress_fraction(estimate: Option<&ScanEstimate>, p: &Progress) -> Option<f64> {
    estimate?.fraction(items_of(p), p.alloc)
}

pub fn view<'a>(
    results: &'a Results,
    title: &'a str,
    width: f32,
    now: Instant,
) -> Element<'a, Message> {
    let l = lines(results, title);
    let icon: Element<'a, Message> = match results.phase {
        Phase::Complete => theme::BODY.text("✓").style(theme::text_good).into(),
        Phase::Stopped => theme::BODY.text("·").style(theme::text_secondary).into(),
        Phase::Opening | Phase::Scanning => {
            // The scanning dot pulses with the tick (UX §11.8: 1.2 s).
            let phase = (now.duration_since(results.started_at).as_secs_f32() / 1.2).fract();
            let alpha = 0.35 + 0.65 * (1.0 - (phase * 2.0 - 1.0).abs());
            theme::BODY
                .text("•")
                .style(move |t| theme::text_accent_alpha(t, alpha))
                .into()
        }
    };
    let message = if l.button == "Stop" {
        Message::Stop
    } else {
        Message::RescanAll
    };
    let action = button(theme::SMALL.with_font(crate::fonts::MEDIUM).text(l.button))
        .padding([2, 10])
        .style(theme::secondary_button)
        .on_press(message);

    let path_room = width - 24.0 - estimate_width(&l.line2, theme::CAPTION) - 16.0;
    let path: Option<Element<'a, Message>> = l.path.map(|p| {
        let shown = widgets::truncate_middle(&p, path_room.max(0.0), theme::CAPTION.size);
        theme::CAPTION
            .with_font(crate::fonts::INTER)
            .text(format!(" · {shown}"))
            .style(theme::text_muted)
            .wrapping(text::Wrapping::None)
            .into()
    });
    let line2 = row![
        theme::CAPTION
            .with_font(crate::fonts::INTER)
            .text(l.line2)
            .style(theme::text_secondary)
            .wrapping(text::Wrapping::None)
    ]
    .push(path.filter(|_| path_room >= 120.0));

    let body = row![
        container(icon).width(16).center_x(16),
        column![
            theme::BODY.text(l.line1).wrapping(text::Wrapping::None),
            line2
        ]
        .spacing(0)
        .width(Fill),
        action,
    ]
    .spacing(6)
    .align_y(Center)
    .padding([2, 10]);

    let line = progress_line(results, now);
    container(column![
        container(body).height(Fill).center_y(Fill).clip(true),
        line
    ])
    .width(width)
    .height(36)
    .style(theme::activity_field)
    .clip(true)
    .into()
}

/// The 2 px line along the bottom edge: determinate for volume roots, else a sweep.
fn progress_line<'a>(results: &Results, now: Instant) -> Element<'a, Message> {
    let filled = |portion: Length| {
        container(space())
            .width(portion)
            .height(2)
            .style(theme::accent_fill)
    };
    let gap = |portion: Length| Space::new().width(portion).height(2);
    let fraction = progress_fraction(results.estimate.as_ref(), &results.activity.progress);
    match (results.phase, fraction) {
        (Phase::Complete | Phase::Stopped, _) => Space::new().width(Fill).height(2).into(),
        (Phase::Scanning, Some(f)) => {
            let done = ((f * 1000.0) as u16).max(1);
            row![filled(FillPortion(done)), gap(FillPortion(1000 - done))].into()
        }
        _ => {
            let phase = (now.duration_since(results.started_at).as_secs_f32()
                / SWEEP_PERIOD.as_secs_f32())
            .fract();
            let before = (phase * 75.0) as u16 + 1;
            row![
                gap(FillPortion(before)),
                filled(FillPortion(25)),
                gap(FillPortion(77 - before))
            ]
            .into()
        }
    }
}

fn estimate_width(s: &str, role: TextRole) -> f32 {
    widgets::estimated_width(s, role.size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dirstat_core::tree::testing::{TreeBuilder, dir};

    fn progress(dirs: u64, files: u64, alloc: u64, secs: u64) -> Progress {
        Progress {
            dirs_listed: dirs,
            files,
            alloc,
            elapsed: Duration::from_secs(secs),
            ..Progress::default()
        }
    }

    #[test]
    fn rate_is_a_smoothed_average() {
        let t0 = Instant::now();
        let mut a = ActivityModel::default();
        a.on_progress(progress(0, 0, 0, 0), t0);
        assert_eq!(a.rate(), None);
        a.on_progress(progress(100, 900, 0, 1), t0 + Duration::from_secs(1));
        assert_eq!(a.rate(), Some(1000.0));
        // A sudden stall only pulls the average down gradually.
        a.on_progress(progress(100, 900, 0, 2), t0 + Duration::from_secs(2));
        let r = a.rate().unwrap();
        assert!(r > 600.0 && r < 800.0, "{r}");
    }

    #[test]
    fn current_path_and_waiting() {
        let tree =
            TreeBuilder::new("/r", dir("r").child(dir("Library").child(dir("Caches")))).build();
        let t0 = Instant::now();
        let mut a = ActivityModel::default();
        a.on_progress(
            Progress {
                current: Some((NodeId(2), Duration::from_millis(30))),
                ..Progress::default()
            },
            t0,
        );
        a.refresh_path(&tree, t0);
        assert_eq!(a.current, Some(("Library/Caches".into(), false)));
        // Within 200 ms nothing changes; afterwards a long-running job reads "waiting".
        a.on_progress(
            Progress {
                current: Some((NodeId(1), Duration::from_secs(3))),
                ..Progress::default()
            },
            t0,
        );
        a.refresh_path(&tree, t0 + Duration::from_millis(100));
        assert_eq!(a.current, Some(("Library/Caches".into(), false)));
        a.refresh_path(&tree, t0 + Duration::from_millis(250));
        assert_eq!(a.current, Some(("Library".into(), true)));
        // An id that isn't in the tree yet keeps the old path.
        a.on_progress(
            Progress {
                current: Some((NodeId(99), Duration::ZERO)),
                ..Progress::default()
            },
            t0,
        );
        a.refresh_path(&tree, t0 + Duration::from_millis(500));
        assert_eq!(a.current, Some(("Library".into(), true)));
    }

    #[test]
    fn determinate_progress_only_with_an_estimate() {
        let p = progress(10, 40, 0, 1);
        assert_eq!(progress_fraction(None, &p), None);
        let e = ScanEstimate {
            items: Some(200),
            bytes: None,
            container_used: None,
        };
        assert_eq!(progress_fraction(Some(&e), &p), Some(0.25));
    }
}
