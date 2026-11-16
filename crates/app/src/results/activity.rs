//! The activity field: the one place for scan status (UX §5.2). The model and the line texts
//! are pure; `view` only lays them out.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use dirstat_core::scan::Progress;
use dirstat_core::tree::{NodeId, Tree};
use dirstat_core::volumes::ScanEstimate;
use iced::widget::{Space, button, column, container, mouse_area, row, space, text, tooltip};
use iced::{Center, Element, Fill, FillPortion, Length};

use super::{Phase, Results, errors};
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
    /// Progress fraction per second, averaged over ~10 s, and the last sample.
    fraction_rate: Option<f64>,
    last_fraction: Option<(Instant, f64)>,
    eta: Option<f64>,
}

/// The ETA's rate is averaged over about this long (UX §5.2).
const ETA_WINDOW: Duration = Duration::from_secs(10);

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

    /// Determinate scans: a new progress fraction. The ETA shows after 5 s and past 5%, and
    /// never grows by more than 30% from one update to the next (UX §5.2).
    pub fn on_fraction(&mut self, fraction: f64, elapsed: Duration, now: Instant) {
        if let Some((at, before)) = self.last_fraction {
            let dt = now.saturating_duration_since(at).as_secs_f64();
            if dt <= 0.0 {
                return;
            }
            let instant = (fraction - before).max(0.0) / dt;
            let alpha = 1.0 - (-dt / ETA_WINDOW.as_secs_f64()).exp();
            self.fraction_rate = Some(
                self.fraction_rate
                    .map_or(instant, |r| r + alpha * (instant - r)),
            );
        }
        self.last_fraction = Some((now, fraction));
        let rate = self.fraction_rate.filter(|&r| r > 0.0);
        self.eta = match rate {
            Some(rate) if elapsed >= Duration::from_secs(5) && fraction >= 0.05 => {
                let raw = (1.0 - fraction) / rate;
                Some(self.eta.map_or(raw, |prev| raw.min(prev * 1.3)))
            }
            _ => None,
        };
    }

    pub fn eta(&self) -> Option<f64> {
        self.eta
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
    /// "23 folders couldn't be read": opens the errors popover.
    pub link: Option<String>,
    pub button: &'static str,
}

/// A subtree rescan looks like a scan in the activity field; the phase itself stays.
fn shown_phase(results: &Results) -> Phase {
    if results.rescan.is_some() {
        Phase::Scanning
    } else {
        results.phase
    }
}

pub fn lines(results: &Results, title: &str) -> Lines {
    let a = &results.activity;
    let p = &a.progress;
    let rescan_name = results.rescan.as_ref().map(|r| {
        results
            .tree
            .node(r.target)
            .name()
            .to_string_lossy()
            .into_owned()
    });
    match shown_phase(results) {
        Phase::Opening => Lines {
            line1: format!("Opening {title}…"),
            line2: String::new(),
            path: None,
            link: None,
            button: "Stop",
        },
        Phase::Scanning => {
            let mut line1 = match &rescan_name {
                Some(name) => format!("Rescanning {name}"),
                None => format!("Scanning {title}"),
            };
            if rescan_name.is_none()
                && let Some(fraction) = progress_fraction(results.estimate.as_ref(), p)
            {
                line1 += &format!(" · {}%", (fraction * 100.0).floor() as u64);
                if let Some(eta) = a.eta() {
                    line1 += &format!(" · {}", eta_text(eta));
                }
            } else if let Some((done, of)) = folders_done(results) {
                line1 += &format!(" · {done} of {of} folders done");
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
                link: None,
                button: "Stop",
            }
        }
        Phase::Complete => {
            let total = results.tree.node(NodeId::ROOT).total();
            let elapsed = results
                .summary
                .as_ref()
                .map_or(p.elapsed, |s| s.progress.elapsed);
            let line2 = format!("Scanned in {}", format::elapsed(elapsed));
            Lines {
                link: errors::headline(&results.tree),
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
                link: errors::headline(&results.tree),
                button: "Rescan",
            }
        }
    }
}

/// "about 2 min left" (UX §5.2), rounded so it doesn't flicker.
pub fn eta_text(secs: f64) -> String {
    if secs < 60.0 {
        return "less than a minute left".into();
    }
    let minutes = (secs / 60.0).round() as u64;
    if minutes < 90 {
        return format!("about {minutes} min left");
    }
    let (h, m) = (minutes / 60, (minutes % 60) / 10 * 10);
    if m == 0 {
        format!("about {h} h left")
    } else {
        format!("about {h} h {m} min left")
    }
}

/// Folder scans: the scan root's subfolders that are complete, of those to scan.
fn folders_done(results: &Results) -> Option<(u64, u64)> {
    use dirstat_core::tree::NodeState;
    let tree = results.rescan.as_ref().map_or(&results.tree, |r| &r.sub);
    if !tree.is_started() || tree.node(NodeId::ROOT).state() == NodeState::Pending {
        return None;
    }
    let (mut done, mut of) = (0, 0);
    for child in tree.children(NodeId::ROOT) {
        let node = tree.node(child);
        if matches!(node.state(), NodeState::Skipped(_)) {
            continue;
        }
        of += 1;
        done += u64::from(node.is_done());
    }
    (of > 0).then_some((done, of))
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
    let icon: Element<'a, Message> = match shown_phase(results) {
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
    .push(path.filter(|_| path_room >= 120.0))
    .push(l.link.map(|link| {
        mouse_area(
            theme::CAPTION
                .text(format!(" · {link}"))
                .style(theme::text_accent_link)
                .wrapping(text::Wrapping::None),
        )
        .on_press(Message::ToggleErrorsPopover)
        .interaction(iced::mouse::Interaction::Pointer)
    }));

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
    let field = container(column![
        container(body).height(Fill).center_y(Fill).clip(true),
        line
    ])
    .width(width)
    .height(36)
    .style(theme::activity_field)
    .clip(true);
    // Folders that vanished or changed are not errors; they are only mentioned here.
    let changed = results.activity.progress.vanished;
    if changed > 0 {
        let tip = format!(
            "{} changed during the scan",
            format::plural(changed, "item", "items")
        );
        let tip = container(theme::SMALL.text(tip))
            .padding([4, 8])
            .style(theme::raised);
        tooltip(field, tip, tooltip::Position::Bottom).into()
    } else {
        field.into()
    }
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
    let fraction = progress_fraction(results.estimate.as_ref(), &results.activity.progress)
        .filter(|_| results.rescan.is_none());
    match (shown_phase(results), fraction) {
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
mod eta_tests {
    use super::*;

    #[test]
    fn eta_waits_rounds_and_never_jumps_up() {
        let t0 = Instant::now();
        let s = |n: u64| t0 + Duration::from_secs(n);
        let mut a = ActivityModel::default();
        a.on_fraction(0.0, Duration::ZERO, s(0));
        a.on_fraction(0.04, Duration::from_secs(4), s(4));
        assert_eq!(a.eta(), None, "before 5 s and 5%");
        a.on_fraction(0.10, Duration::from_secs(10), s(10));
        let first = a.eta().expect("past 5 s and 5%");
        assert!((60.0..200.0).contains(&first), "{first}");
        // A stall: the raw estimate shoots up, the shown one grows at most 30%.
        a.on_fraction(0.10, Duration::from_secs(20), s(20));
        assert!(a.eta().unwrap() <= first * 1.3 + 1e-9);

        assert_eq!(eta_text(30.0), "less than a minute left");
        assert_eq!(eta_text(130.0), "about 2 min left");
        assert_eq!(eta_text(2.0 * 3600.0 + 120.0), "about 2 h left");
        assert_eq!(eta_text(3600.0 + 35.0 * 60.0), "about 1 h 30 min left");
    }
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
