//! canvas::Program probe: a squarified treemap with a cached base layer, a live hover layer,
//! click -> message, clipped labels and a custom mouse cursor.
use crate::style;

use iced::mouse;
use iced::widget::canvas::{self, Action, Cache, Event, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Font, Pixels, Point, Rectangle, Renderer, Size, Theme, Vector, alignment, font};

pub struct Tile {
    pub label: String,
    pub detail: String,
    pub bytes: u64,
}

/// The Program borrows from application state, so it is built fresh in every `view`.
pub struct Treemap<'a, Message> {
    pub tiles: &'a [Tile],
    /// Lives in the application state; call `cache.clear()` whenever tiles OR theme change.
    pub cache: &'a Cache,
    pub on_click: fn(usize) -> Message,
}

/// Per-widget state kept by iced in the widget tree (must be Default + 'static).
#[derive(Debug, Default)]
pub struct State {
    hovered: Option<usize>,
}

impl<Message> canvas::Program<Message> for Treemap<'_, Message> {
    type State = State;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
                let hovered = cursor
                    .position_in(bounds)
                    .and_then(|position| self.hit(bounds.size(), position));

                if hovered != state.hovered {
                    state.hovered = hovered;
                    // State changed without a message: we MUST ask for a redraw ourselves.
                    return Some(Action::request_redraw());
                }
                None
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let position = cursor.position_in(bounds)?;
                let index = self.hit(bounds.size(), position)?;
                // publish => message to update(); and_capture => stop propagation.
                Some(Action::publish((self.on_click)(index)).and_capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let palette = theme.extended_palette();

        // Layer 1: cached tiles (re-tessellated only after cache.clear() or a resize).
        let tiles = self.cache.draw(renderer, bounds.size(), |frame| {
            frame.fill_rectangle(Point::ORIGIN, frame.size(), palette.background.base.color);

            let rects = squarify(
                &self.tiles.iter().map(|tile| tile.bytes).collect::<Vec<_>>(),
                Rectangle::with_size(frame.size()),
            );

            for (index, (rect, tile)) in rects.iter().zip(self.tiles).enumerate() {
                if rect.width < 1.0 || rect.height < 1.0 {
                    continue;
                }

                let inner = Rectangle {
                    x: rect.x + 1.0,
                    y: rect.y + 1.0,
                    width: (rect.width - 2.0).max(0.0),
                    height: (rect.height - 2.0).max(0.0),
                };
                let fill = style::tile_color(theme, index);
                let path = Path::rounded_rectangle(inner.position(), inner.size(), 4.0.into());

                frame.fill(&path, fill);
                frame.stroke(
                    &path,
                    Stroke::default()
                        .with_color(palette.background.base.color)
                        .with_width(1.0),
                );

                // Only draw labels that fit. NOTE: with_clip clips text on wgpu, but the
                // tiny-skia fallback renderer does NOT clip canvas text -> size-check first.
                if inner.width > 48.0 && inner.height > 20.0 {
                    let text_color = iced::theme::palette::readable(fill, Color::WHITE);
                    frame.with_clip(inner, |frame| {
                        // Coordinates inside with_clip are still in the PARENT frame's space.
                        // canvas::Text word-wraps at `max_width` (default INFINITY) and has
                        // no ellipsis: truncate manually and keep max_width = INFINITY.
                        frame.fill_text(Text {
                            content: truncate_to_width(&tile.label, inner.width - 12.0, 13.0),
                            position: Point::new(inner.x + 6.0, inner.y + 4.0),
                            color: text_color,
                            size: Pixels(13.0),
                            font: Font {
                                weight: font::Weight::Semibold,
                                ..Font::DEFAULT
                            },
                            ..Text::default()
                        });

                        if inner.height > 40.0 {
                            frame.fill_text(Text {
                                content: truncate_to_width(&tile.detail, inner.width - 12.0, 11.0),
                                position: Point::new(inner.x + 6.0, inner.y + 22.0),
                                color: text_color.scale_alpha(0.85),
                                size: Pixels(11.0),
                                ..Text::default()
                            });
                        }
                    });
                }
            }
        });

        // Layer 2: hover highlight, redrawn every frame (cheap, not cached).
        let mut overlay = Frame::new(renderer, bounds.size());
        if let Some(index) = state.hovered {
            let rects = squarify(
                &self.tiles.iter().map(|tile| tile.bytes).collect::<Vec<_>>(),
                Rectangle::with_size(bounds.size()),
            );
            if let (Some(rect), Some(tile)) = (rects.get(index), self.tiles.get(index)) {
                overlay.stroke(
                    &Path::rounded_rectangle(
                        rect.position() + Vector::new(1.0, 1.0),
                        Size::new(rect.width - 2.0, rect.height - 2.0),
                        4.0.into(),
                    ),
                    Stroke::default()
                        .with_color(palette.background.base.text)
                        .with_width(2.0),
                );

                // Centered caption at the bottom of the canvas.
                overlay.fill_text(Text {
                    content: format!("{} — {}", tile.label, tile.detail),
                    position: Point::new(bounds.width / 2.0, bounds.height - 6.0),
                    color: palette.background.base.text,
                    size: Pixels(12.0),
                    align_x: iced::widget::text::Alignment::Center,
                    align_y: alignment::Vertical::Bottom,
                    ..Text::default()
                });
            }
        }

        vec![tiles, overlay.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
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

impl<Message> Treemap<'_, Message> {
    fn hit(&self, size: Size, position: Point) -> Option<usize> {
        let rects = squarify(
            &self.tiles.iter().map(|tile| tile.bytes).collect::<Vec<_>>(),
            Rectangle::with_size(size),
        );
        rects
            .iter()
            .position(|rect| rect.width >= 1.0 && rect.height >= 1.0 && rect.contains(position))
    }
}

/// Cheap width estimate (avg glyph ≈ 0.6 em) + "…". Good enough for labels; for exact
/// measurement you would need iced::advanced::text::Paragraph (feature "advanced").
pub fn truncate_to_width(label: &str, max_width: f32, size: f32) -> String {
    let max_chars = (max_width / (size * 0.6)).floor().max(0.0) as usize;
    let count = label.chars().count();
    if count <= max_chars {
        label.to_owned()
    } else if max_chars <= 1 {
        String::new()
    } else {
        let mut truncated: String = label.chars().take(max_chars - 1).collect();
        truncated.push('…');
        truncated
    }
}

/// Squarified treemap (Bruls et al.). `values` should be sorted descending; zeros get an
/// empty rectangle.
pub fn squarify(values: &[u64], bounds: Rectangle) -> Vec<Rectangle> {
    let mut out = vec![Rectangle::new(Point::ORIGIN, Size::ZERO); values.len()];
    let total: f64 = values.iter().map(|&value| value as f64).sum();
    if total <= 0.0 || bounds.width < 1.0 || bounds.height < 1.0 {
        return out;
    }

    let area = f64::from(bounds.width) * f64::from(bounds.height);
    let scaled: Vec<f64> = values
        .iter()
        .map(|&value| value as f64 * area / total)
        .collect();

    let mut free = bounds;
    let mut start = 0;
    // Skip trailing zero values entirely.
    let end = scaled.iter().rposition(|&value| value > 0.0).map_or(0, |last| last + 1);

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
            for (index, value) in scaled.iter().enumerate().take(stop).skip(start) {
                let height = (value / f64::from(strip)) as f32;
                out[index] = Rectangle { x: free.x, y, width: strip, height };
                y += height;
            }
            free.x += strip;
            free.width -= strip;
        } else {
            let strip = (row_sum / f64::from(free.width)) as f32;
            let mut x = free.x;
            for (index, value) in scaled.iter().enumerate().take(stop).skip(start) {
                let width = (value / f64::from(strip)) as f32;
                out[index] = Rectangle { x, y: free.y, width, height: strip };
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
    let min = row.iter().copied().fold(f64::MAX, f64::min).max(f64::MIN_POSITIVE);
    let sum2 = sum * sum;
    let short2 = short * short;
    (short2 * max / sum2).max(sum2 / (short2 * min))
}
