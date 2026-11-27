//! The space bar under the path bar: the zoomed folder split by kind of content (Documents,
//! Applications, Caches, ...), like the storage bar of macOS's settings.
//!
//! A tall segmented bar: one block per kind, widths proportional to bytes, 2 px surface gaps,
//! the share printed inside blocks wide enough for it. The legend below names the kinds in
//! block order with their sizes; hovering a block shows name, size, share and what counts.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dirstat_core::breakdown::{self, Breakdown, Kind, Places};
use dirstat_core::catalog::Engine;
use dirstat_core::tree::{NodeId, Tree};
use iced::widget::{Row, column, container, row, space, tooltip};
use iced::{Center, Color, Element, Fill, Length, color};

use super::Results;
use crate::app::Message;
use crate::{format, theme, widgets};

/// The whole strip, rules excluded: padding, bar, gap, legend.
pub const HEIGHT: f32 = PAD_TOP + BAR_H + GAP + LEGEND_H + PAD_BOTTOM;
const PAD_TOP: f32 = 10.0;
const PAD_BOTTOM: f32 = 8.0;
const BAR_H: f32 = 24.0;
const GAP: f32 = 8.0;
const LEGEND_H: f32 = 16.0;
/// Surface gap between blocks.
const SPACER: f32 = 2.0;
/// A non-empty kind never draws narrower than this.
const MIN_BLOCK: f32 = 3.0;
/// While scanning, recount at most this often (a full walk of the zoomed subtree).
const LIVE_EVERY: Duration = Duration::from_millis(1000);

/// Breakdown state kept in `Results`.
#[derive(Default)]
pub struct BreakdownState {
    pub places: Arc<Places>,
    pub data: Breakdown,
    /// The volume's used space the scan didn't find, when `data` is for the scan root.
    pub unaccounted: Option<u64>,
    dirty: bool,
    zoom_seen: Option<NodeId>,
    counted_at: Option<Instant>,
}

impl BreakdownState {
    /// The tree's data changed: recount when the clock allows.
    pub fn touch(&mut self) {
        self.dirty = true;
    }

    /// From `Results::tick`. A zoom change recounts at once.
    pub fn tick(
        &mut self,
        tree: &Tree,
        engine: Option<&Engine>,
        zoom: NodeId,
        unaccounted: Option<u64>,
        now: Instant,
        scanning: bool,
    ) {
        let Some(engine) = engine else { return };
        let zoomed = self.zoom_seen != Some(zoom);
        let due = !scanning
            || self
                .counted_at
                .is_none_or(|at| now.duration_since(at) >= LIVE_EVERY);
        if !(zoomed || self.dirty && due) {
            return;
        }
        self.data = breakdown::compute(tree, engine, &self.places, zoom);
        self.unaccounted = unaccounted.filter(|_| zoom == NodeId::ROOT);
        self.zoom_seen = Some(zoom);
        self.dirty = false;
        self.counted_at = Some(now);
    }

    /// The blocks in bar order: kinds, then "Unaccounted".
    fn parts(&self) -> Vec<Part> {
        let mut parts: Vec<Part> = self
            .data
            .parts()
            .map(|(kind, bytes)| Part {
                kind: Some(kind),
                bytes,
            })
            .collect();
        if let Some(bytes) = self.unaccounted.filter(|&b| b > 0) {
            parts.push(Part { kind: None, bytes });
        }
        parts
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Part {
    /// `None`: Unaccounted.
    kind: Option<Kind>,
    bytes: u64,
}

impl Part {
    fn label(self) -> &'static str {
        self.kind.map_or("Unaccounted", Kind::label)
    }
    fn hint(self) -> &'static str {
        self.kind
            .map_or("Used on the volume, but not found by the scan", Kind::hint)
    }
}

/// Fill and label ink of a block. The eight kinds take the categorical slots in order
/// (validated: adjacent CVD ΔE ≥ 8.4, normal-vision ΔE ≥ 19.3 in both modes); Other is the
/// map's neutral gray, Unaccounted the track color.
fn fill(theme: &iced::Theme, kind: Option<Kind>) -> (Color, Color) {
    const INK: Color = color!(0x0b0b0b);
    let dark = theme.extended_palette().is_dark;
    let Some(kind) = kind else {
        let t = theme::tokens(theme);
        return (t.border_strong, t.text_primary);
    };
    let (light, dark_fill) = match kind {
        Kind::Documents => (color!(0x2a78d6), color!(0x3987e5)),
        Kind::Applications => (color!(0xeb6834), color!(0xd95926)),
        Kind::Games => (color!(0x1baf7a), color!(0x199e70)),
        Kind::Developer => (color!(0xeda100), color!(0xc98500)),
        Kind::Caches => (color!(0xe87ba4), color!(0xd55181)),
        Kind::Logs => (color!(0x008300), color!(0x0ca30c)),
        Kind::Trash => (color!(0x6250d6), color!(0x9085e9)),
        Kind::System => (color!(0xe34948), color!(0xe66767)),
        Kind::Other => (color!(0xa8a69e), color!(0x5f5e59)),
    };
    if dark {
        return (dark_fill, Color::WHITE);
    }
    // Light mode: near-black on the light fills (aqua, yellow, magenta, gray).
    let ink = match kind {
        Kind::Games | Kind::Developer | Kind::Caches | Kind::Other => INK,
        _ => Color::WHITE,
    };
    (light, ink)
}

/// Block widths in px for a bar `width` wide: proportional, each at least `MIN_BLOCK`.
fn widths(parts: &[Part], width: f32) -> Vec<f32> {
    let total: u64 = parts.iter().map(|p| p.bytes).sum();
    let avail = (width - SPACER * parts.len().saturating_sub(1) as f32).max(0.0);
    if total == 0 || parts.is_empty() {
        return vec![0.0; parts.len()];
    }
    let raw: Vec<f32> = parts
        .iter()
        .map(|p| (p.bytes as f64 / total as f64) as f32 * avail)
        .collect();
    // Tiny blocks take `MIN_BLOCK` from the big ones, proportionally.
    let bumped: f32 = raw.iter().map(|w| (MIN_BLOCK - w).max(0.0)).sum();
    let big: f32 = raw.iter().filter(|&&w| w >= MIN_BLOCK).sum();
    raw.iter()
        .map(|&w| {
            if w < MIN_BLOCK {
                MIN_BLOCK
            } else if big > 0.0 {
                w - bumped * w / big
            } else {
                w
            }
        })
        .collect()
}

fn tip<'a>(part: Part, total: u64) -> Element<'a, Message> {
    container(
        column![
            row![
                theme::SMALL
                    .with_font(crate::fonts::MEDIUM)
                    .text(part.label()),
                space().width(Fill),
                theme::SMALL.text(format!(
                    "{} · {}",
                    format::size(part.bytes),
                    format::percent(part.bytes, total)
                )),
            ]
            .spacing(12),
            theme::SMALL.text(part.hint()).style(theme::text_secondary),
        ]
        .spacing(2)
        .width(Length::Shrink),
    )
    .padding([6, 8])
    .max_width(320)
    .style(theme::raised)
    .into()
}

/// The strip, `width` px wide (the window).
pub fn view<'a>(results: &'a Results, width: f32) -> Element<'a, Message> {
    let state = &results.breakdown;
    let parts = state.parts();
    let total: u64 = parts.iter().map(|p| p.bytes).sum();
    let bar_width = width - 24.0;

    let bar: Element<'a, Message> = if total == 0 {
        container(space())
            .width(Fill)
            .height(BAR_H)
            .style(|t| block_style(theme::tokens(t).bar_track))
            .into()
    } else {
        let widths = widths(&parts, bar_width);
        let mut bar = Row::new().spacing(SPACER).height(BAR_H);
        for (part, w) in parts.iter().copied().zip(widths) {
            let share = format::percent(part.bytes, total);
            let label =
                (widgets::estimated_width(&share, theme::CAPTION.size) + 12.0 <= w).then(|| {
                    theme::CAPTION.text(share).style(move |t: &iced::Theme| {
                        iced::widget::text::Style {
                            color: Some(fill(t, part.kind).1),
                        }
                    })
                });
            let block = container(label)
                .padding([0, 6])
                .center_y(BAR_H)
                .width(w)
                .style(move |t| block_style(fill(t, part.kind).0));
            bar = bar.push(
                tooltip(block, tip(part, total), tooltip::Position::Bottom)
                    .delay(Duration::from_millis(150)),
            );
        }
        bar.into()
    };

    container(column![bar, legend(&parts, bar_width)].spacing(GAP))
        .padding(iced::Padding {
            top: PAD_TOP,
            bottom: PAD_BOTTOM,
            left: 12.0,
            right: 12.0,
        })
        .width(Fill)
        .height(HEIGHT)
        .into()
}

fn block_style(color: Color) -> container::Style {
    container::Style {
        background: Some(color.into()),
        border: iced::Border {
            radius: 4.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// "● Documents 12 GB  ● Applications 3 GB …": one line; sizes go first when it is too
/// narrow, then the smallest entries (their blocks keep their tooltips).
fn legend<'a>(parts: &[Part], width: f32) -> Element<'a, Message> {
    let size = theme::SMALL.size;
    let entry_w = |p: &Part, with_size: bool| {
        let mut w = 8.0 + 6.0 + widgets::estimated_width(p.label(), size) + 16.0;
        if with_size {
            w += 6.0 + widgets::estimated_width(&format::size(p.bytes), size);
        }
        w
    };
    let fits = |shown: &[Part], with_size: bool| {
        shown.iter().map(|p| entry_w(p, with_size)).sum::<f32>() <= width
    };
    let mut shown = parts.to_vec();
    let with_size = fits(&shown, true);
    while !fits(&shown, with_size) && shown.len() > 1 {
        let smallest = (0..shown.len())
            .min_by_key(|&i| shown[i].bytes)
            .expect("not empty");
        shown.remove(smallest);
    }
    let mut line = Row::new().spacing(16).height(LEGEND_H).align_y(Center);
    for part in shown {
        let dot = container(space())
            .width(8)
            .height(8)
            .style(move |t| container::Style {
                background: Some(fill(t, part.kind).0.into()),
                border: iced::Border {
                    radius: 4.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        let mut entry = row![dot, theme::SMALL.text(part.label())]
            .spacing(6)
            .align_y(Center);
        if with_size {
            entry = entry.push(
                theme::SMALL
                    .text(format::size(part.bytes))
                    .style(theme::text_secondary),
            );
        }
        line = line.push(entry);
    }
    line.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(bytes: u64) -> Part {
        Part {
            kind: Some(Kind::Other),
            bytes,
        }
    }

    #[test]
    fn widths_fill_the_bar_and_keep_tiny_parts_visible() {
        let parts = [part(1_000_000), part(1), part(500_000)];
        let w = widths(&parts, 302.0);
        let sum: f32 = w.iter().sum();
        assert!((sum - 298.0).abs() < 0.01, "{w:?}");
        assert_eq!(w[1], MIN_BLOCK);
        assert!(w[0] > 2.0 * w[2] - 1.0 && w[0] < 2.0 * w[2] + 1.0, "{w:?}");
    }

    #[test]
    fn unaccounted_follows_the_kinds() {
        let mut s = BreakdownState::default();
        s.data.bytes[Kind::System.index()] = 5;
        s.data.bytes[Kind::Documents.index()] = 7;
        s.unaccounted = Some(3);
        let order: Vec<_> = s.parts().iter().map(|p| p.label()).collect();
        assert_eq!(order, ["Documents", "System", "Unaccounted"]);
    }
}
