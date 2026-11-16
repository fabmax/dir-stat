//! The Map tab (UX §8): a canvas with a cached block layer and a live overlay (hover,
//! selection, dimming, tooltip), and the legend row below it.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::time::{Duration, Instant};

use dirstat_core::tree::{NodeId, Tree};
use iced::mouse;
use iced::widget::canvas::{
    self, Action, Cache, Event, Frame, Geometry, Path as CPath, Stroke, Text,
};
use iced::widget::{button, column, container, mouse_area, row, space};
use iced::{Center, Color, Element, Fill, Pixels, Point, Rectangle, Renderer, Size, Theme};

use super::Results;
use super::rows::RowKey;
use super::treemap::{self, BlockKind, HEADER_H, Layout, MapClock, Slots};
use crate::app::Message;
use crate::{fonts, format, theme, widgets};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Legend entries are as tall as a 28 px row minus its padding.
const LEGEND_ROW: f32 = 20.0;

/// Map state kept in `Results`. The layout is computed by the canvas for its own size,
/// whenever the size or `clock.version` changes.
#[derive(Default)]
pub struct MapState {
    pub clock: MapClock,
    pub slots: Slots,
    pub pointer_over: bool,
    /// The block under the pointer: its tree row is highlighted too.
    pub hover: Option<RowKey>,
    zoom_seen: Option<NodeId>,
    layout: RefCell<Option<(Size, u64, Layout)>>,
    blocks: Cache,
    dark: Cell<Option<bool>>,
}

impl std::fmt::Debug for MapState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapState")
            .field("clock", &self.clock)
            .field("slots", &self.slots)
            .field("pointer_over", &self.pointer_over)
            .finish_non_exhaustive()
    }
}

impl MapState {
    /// The tree's data changed (listings, trash, rescan): relayout when the clock allows.
    pub fn touch(&mut self) {
        self.clock.dirty = true;
    }

    /// From `Results::tick`. A zoom change relayouts at once (new slots, too).
    pub fn tick(&mut self, tree: &Tree, zoom: NodeId, now: Instant, scanning: bool) {
        if self.zoom_seen != Some(zoom) {
            self.zoom_seen = Some(zoom);
            self.clock.dirty = true;
            self.slots.update(tree, zoom);
            self.clock.tick(now, false, false);
            return;
        }
        if self.clock.tick(now, scanning, self.pointer_over) {
            self.slots.update(tree, zoom);
        }
    }

    /// The current layout for `size`, computed if needed.
    fn ensure(&self, tree: &Tree, zoom: NodeId, size: Size) {
        let fresh = matches!(&*self.layout.borrow(), Some((s, v, _)) if *s == size && *v == self.clock.version);
        if !fresh {
            let layout = treemap::layout(tree, zoom, size, &self.slots);
            *self.layout.borrow_mut() = Some((size, self.clock.version, layout));
            self.blocks.clear();
        }
    }

    #[cfg(test)]
    pub fn layout_for(&self, tree: &Tree, zoom: NodeId, size: Size) -> Layout {
        self.ensure(tree, zoom, size);
        self.layout
            .borrow()
            .as_ref()
            .map(|(_, _, l)| l.clone())
            .unwrap_or_default()
    }
}

struct MapProgram<'a> {
    results: &'a Results,
    home: Option<&'a Path>,
}

#[derive(Default)]
struct CanvasState {
    hovered: Option<usize>,
    last_click: Option<(Instant, usize)>,
}

impl MapProgram<'_> {
    fn hit(&self, size: Size, at: Point) -> Option<(usize, BlockKind)> {
        let map = &self.results.map;
        map.ensure(&self.results.tree, self.results.view.zoom, size);
        let layout = map.layout.borrow();
        let (_, _, layout) = layout.as_ref()?;
        let index = layout.hit(at)?;
        Some((index, layout.blocks[index].kind))
    }
}

impl canvas::Program<Message> for MapProgram<'_> {
    type State = CanvasState;

    fn update(
        &self,
        state: &mut CanvasState,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
                let hit = cursor
                    .position_in(bounds)
                    .and_then(|p| self.hit(bounds.size(), p));
                let index = hit.map(|(i, _)| i);
                if index != state.hovered {
                    state.hovered = index;
                    return Some(Action::publish(Message::MapHover(
                        hit.map(|(_, k)| k.row_key()),
                    )));
                }
                // The tooltip follows the pointer.
                index.map(|_| Action::request_redraw())
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let (index, kind) = self.hit(bounds.size(), cursor.position_in(bounds)?)?;
                let now = Instant::now();
                let double = state
                    .last_click
                    .is_some_and(|(at, i)| i == index && now.duration_since(at) < DOUBLE_CLICK);
                state.last_click = (!double).then_some((now, index));
                let message = if double {
                    Message::MapActivated(kind.row_key())
                } else {
                    Message::MapPressed(kind.row_key())
                };
                Some(Action::publish(message).and_capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &CanvasState,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let results = self.results;
        let map = &results.map;
        let tokens = theme::tokens(theme);
        let dark = theme.extended_palette().is_dark;
        if map.dark.replace(Some(dark)) != Some(dark) {
            map.blocks.clear();
        }
        map.ensure(&results.tree, results.view.zoom, bounds.size());
        let guard = map.layout.borrow();
        let Some((_, _, layout)) = guard.as_ref() else {
            return Vec::new();
        };

        let blocks = map.blocks.draw(renderer, bounds.size(), |frame| {
            frame.fill_rectangle(Point::ORIGIN, frame.size(), tokens.bg_surface);
            for block in &layout.blocks {
                let files = matches!(block.kind, BlockKind::Files(_));
                let (fill, label) = theme::map_fill(theme, block.slot, block.level, files);
                let radius = if block.level == 1 { 4.0 } else { 2.0 };
                let r = block.rect;
                frame.fill(
                    &CPath::rounded_rectangle(r.position(), r.size(), radius.into()),
                    fill,
                );
                if block.header {
                    let size = format::size(block.bytes);
                    let size_w = widgets::estimated_width(&size, 11.0);
                    let room = r.width - 12.0 - size_w - 8.0;
                    let name = widgets::truncate_end(&block.name, room.max(0.0), 12.0);
                    if room >= 24.0 && !name.is_empty() {
                        frame.fill_text(Text {
                            content: name,
                            position: Point::new(r.x + 6.0, r.y + (HEADER_H - 12.0) / 2.0),
                            color: label,
                            size: Pixels(12.0),
                            font: fonts::MEDIUM,
                            ..Text::default()
                        });
                        frame.fill_text(Text {
                            content: size,
                            position: Point::new(
                                r.x + r.width - 6.0 - size_w,
                                r.y + (HEADER_H - 11.0) / 2.0 + 0.5,
                            ),
                            color: label.scale_alpha(0.85),
                            size: Pixels(11.0),
                            font: fonts::INTER,
                            ..Text::default()
                        });
                    }
                } else if block.level == 2 {
                    // Only a label that fits completely: never clipped mid-glyph.
                    let w = widgets::estimated_width(&block.name, 11.0);
                    if w + 12.0 <= r.width && r.height >= 11.0 + 12.0 {
                        frame.fill_text(Text {
                            content: block.name.clone(),
                            position: Point::new(r.x + 6.0, r.y + 5.0),
                            color: label,
                            size: Pixels(11.0),
                            font: fonts::INTER,
                            ..Text::default()
                        });
                    }
                }
            }
        });

        let mut overlay = Frame::new(renderer, bounds.size());
        // Selection: outline the selected block (or its nearest drawn ancestor) and dim
        // everything outside a selected folder.
        if let Some(index) = selected_block(results, layout) {
            let selected = &layout.blocks[index];
            if let BlockKind::Dir(sel) = selected.kind {
                let tree = &results.tree;
                let related = |kind: BlockKind| {
                    let d = kind.dir();
                    let inside = matches!(kind, BlockKind::Dir(_)) && tree.is_within(d, sel)
                        || !matches!(kind, BlockKind::Dir(_)) && tree.is_within(d, sel);
                    inside || tree.is_within(sel, d)
                };
                for block in &layout.blocks {
                    let parent_related =
                        block.parent.is_none_or(|p| related(layout.blocks[p].kind));
                    if !related(block.kind) && parent_related {
                        let r = block.rect;
                        overlay.fill(
                            &CPath::rounded_rectangle(r.position(), r.size(), 2.0.into()),
                            tokens.bg_surface.scale_alpha(0.45),
                        );
                    }
                }
            }
            let r = selected.rect;
            overlay.stroke(
                &CPath::rounded_rectangle(r.position(), r.size(), 3.0.into()),
                Stroke::default()
                    .with_color(tokens.text_primary)
                    .with_width(2.0),
            );
            let ring = Rectangle {
                x: r.x - 2.0,
                y: r.y - 2.0,
                width: r.width + 4.0,
                height: r.height + 4.0,
            };
            overlay.stroke(
                &CPath::rounded_rectangle(ring.position(), ring.size(), 4.0.into()),
                Stroke::default()
                    .with_color(tokens.bg_surface)
                    .with_width(2.0),
            );
        }
        // Hover: 12% toward white, and the tooltip next to the pointer.
        if let Some(block) = state.hovered.and_then(|i| layout.blocks.get(i)) {
            let r = block.rect;
            overlay.fill(
                &CPath::rounded_rectangle(r.position(), r.size(), 2.0.into()),
                Color::WHITE.scale_alpha(0.12),
            );
            if let Some(at) = cursor.position_in(bounds) {
                tooltip(
                    &mut overlay,
                    results,
                    block.kind,
                    block.bytes,
                    &block.name,
                    at,
                    bounds.size(),
                    self.home,
                    tokens,
                );
            }
        }
        vec![blocks, overlay.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &CanvasState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.hovered.is_some() && cursor.is_over(bounds) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

/// The block of the tree selection, or of its nearest ancestor that is drawn.
pub(crate) fn selected_block(results: &Results, layout: &Layout) -> Option<usize> {
    let key = results.view.selection?;
    if let Some(i) = layout.find(key) {
        return Some(i);
    }
    let tree = &results.tree;
    let dir = key.dir();
    tree.get(dir)?;
    std::iter::once(dir)
        .chain(tree.ancestors(dir))
        .take_while(|&a| a != results.view.zoom)
        .find_map(|a| layout.find(RowKey::Dir(a)))
}

#[allow(clippy::too_many_arguments)]
fn tooltip(
    frame: &mut Frame,
    results: &Results,
    kind: BlockKind,
    bytes: u64,
    name: &str,
    at: Point,
    size: Size,
    home: Option<&Path>,
    tokens: &theme::Tokens,
) {
    let tree = &results.tree;
    let (path, share_of, items) = match kind {
        BlockKind::Dir(id) => {
            let parent = tree.node(id).parent().unwrap_or(id);
            (tree.path_of(id), parent, Some(tree.node(id).total().items))
        }
        BlockKind::File(dir, index) => {
            let file = &tree.node(dir).large_files()[index as usize];
            (tree.path_of(dir).join(&*file.name), dir, None)
        }
        BlockKind::Files(dir) | BlockKind::More(dir, _) => (tree.path_of(dir), dir, None),
    };
    let parent_total = tree.node(share_of).total().alloc;
    let parent_name = tree.node(share_of).name().to_string_lossy().into_owned();
    let mut fourth = format!("{} of {parent_name}", format::percent(bytes, parent_total));
    if let Some(items) = items {
        fourth += &format!(" · {}", format::plural(items, "item", "items"));
    }
    let lines = [
        (
            format::size(bytes),
            13.0,
            fonts::MEDIUM,
            tokens.text_primary,
        ),
        (name.to_string(), 12.0, fonts::INTER, tokens.text_primary),
        (
            widgets::truncate_middle(&super::selection_bar::tilde(&path, home), 320.0, 11.0),
            11.0,
            fonts::INTER,
            tokens.text_secondary,
        ),
        (fourth, 11.0, fonts::INTER, tokens.text_secondary),
    ];
    let width = lines
        .iter()
        .map(|(t, s, ..)| widgets::estimated_width(t, *s))
        .fold(0.0, f32::max)
        + 16.0;
    let height = 8.0 + lines.iter().map(|(_, s, ..)| s * 1.35).sum::<f32>() + 6.0;
    // 12 px from the pointer, flipped at the edges.
    let mut x = at.x + 12.0;
    let mut y = at.y + 12.0;
    if x + width > size.width {
        x = (at.x - 12.0 - width).max(0.0);
    }
    if y + height > size.height {
        y = (at.y - 12.0 - height).max(0.0);
    }
    let rect = CPath::rounded_rectangle(Point::new(x, y), Size::new(width, height), 8.0.into());
    frame.fill(&rect, tokens.bg_raised);
    frame.stroke(
        &rect,
        Stroke::default()
            .with_color(tokens.border_hairline)
            .with_width(1.0),
    );
    let mut line_y = y + 8.0;
    for (content, size, font, color) in lines {
        frame.fill_text(Text {
            content,
            position: Point::new(x + 8.0, line_y),
            color,
            size: Pixels(size),
            font,
            ..Text::default()
        });
        line_y += size * 1.35;
    }
}

/// The Map tab: the canvas, or an empty state, and the legend.
pub fn view<'a>(results: &'a Results, home: Option<&'a Path>) -> Element<'a, Message> {
    let zoom = results.view.zoom;
    let empty =
        !results.tree.is_started() || results.tree.get(zoom).is_none_or(|n| n.total().alloc == 0);
    if empty {
        return container(
            theme::SMALL
                .text("Nothing to show: this folder is empty.")
                .style(theme::text_secondary),
        )
        .center(Fill)
        .into();
    }
    let map = mouse_area(
        container(
            canvas::Canvas::new(MapProgram { results, home })
                .width(Fill)
                .height(Fill),
        )
        .padding(8)
        .width(Fill)
        .height(Fill),
    )
    .on_enter(Message::PointerInMap(true))
    .on_exit(Message::PointerInMap(false));
    column![map, legend(results)].into()
}

/// The three colored groups and "Other (N)" with their sizes; a click selects the folder.
fn legend(results: &Results) -> Element<'_, Message> {
    let tree = &results.tree;
    let zoom = results.view.zoom;
    let slots = &results.map.slots;
    let mut entries = row![].spacing(12).align_y(Center);
    let swatch = |slot: Option<u8>| {
        container(space().width(10).height(10)).style(move |t: &Theme| container::Style {
            background: Some(theme::map_fill(t, slot, 1, false).0.into()),
            border: iced::Border {
                radius: 2.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
    };
    let mut colored = 0u64;
    // Largest first; each keeps its slot's color.
    let mut groups: Vec<(usize, NodeId, u64)> = slots
        .ids
        .iter()
        .enumerate()
        .filter_map(|(slot, id)| Some((slot, (*id)?, tree.get((*id)?)?.total().alloc)))
        .collect();
    groups.sort_by_key(|&(_, _, bytes)| std::cmp::Reverse(bytes));
    for (slot, id, _) in groups {
        let node = tree.node(id);
        colored += node.total().alloc;
        let name = widgets::truncate_end(&node.name().to_string_lossy(), 110.0, 11.0);
        entries = entries.push(
            button(
                row![
                    swatch(Some(slot as u8)),
                    theme::CAPTION.with_font(fonts::INTER).text(name),
                    theme::CAPTION
                        .with_font(fonts::INTER)
                        .text(format::size(node.total().alloc))
                        .style(theme::text_secondary),
                ]
                .spacing(5)
                .align_y(Center),
            )
            .padding([2, 4])
            .height(LEGEND_ROW)
            .style(theme::row_button)
            .on_press(Message::MapPressed(RowKey::Dir(id))),
        );
    }
    let total = tree.node(zoom).total().alloc;
    let others = tree.children(zoom).filter(|&c| {
        tree.get(c).is_some_and(|n| n.total().alloc > 0) && !slots.ids.contains(&Some(c))
    });
    let other_count = others.count() as u64 + u64::from(tree.node(zoom).files().count > 0);
    let other_bytes = total.saturating_sub(colored);
    if other_count > 0 && other_bytes > 0 {
        entries = entries.push(
            row![
                swatch(None),
                theme::CAPTION
                    .with_font(fonts::INTER)
                    .text(format!("Other ({})", format::thousands(other_count))),
                theme::CAPTION
                    .with_font(fonts::INTER)
                    .text(format::size(other_bytes))
                    .style(theme::text_secondary),
            ]
            .spacing(5)
            .align_y(Center),
        );
    }
    // One 28 px row when it fits, a second line in a narrow pane.
    container(entries.wrap().vertical_spacing(2))
        .padding([6, 8])
        .width(Fill)
        .into()
}
