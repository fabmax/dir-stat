//! Manual virtualization: fixed row height + scrollable::on_scroll(Viewport) -> render only the
//! visible slice between two spacers. Handles 100k+ rows with constant view() cost.
use crate::model::{self, Row};
use crate::style;

use iced::widget::{
    Column, Id, button, container, hover, mouse_area, progress_bar, row, scrollable, space, text,
    tooltip,
};
use iced::{Element, Fill, Font, Length, Theme, font, mouse};

pub const ROW_HEIGHT: f32 = 26.0;
const OVERSCAN: usize = 6;

/// Widget Id of the tree scrollable (0.14: one unified `iced::widget::Id` for all widgets).
pub const TREE_SCROLLABLE: &str = "tree-scrollable";

pub fn tree_id() -> Id {
    Id::new(TREE_SCROLLABLE)
}

/// What we remember from the last `on_scroll` Viewport.
#[derive(Debug, Clone, Copy)]
pub struct Window {
    pub offset_y: f32,
    pub height: f32,
}

impl Default for Window {
    fn default() -> Self {
        // Until the first Viewport arrives (only after the content overflows!), assume a tall
        // window so that the first screen is fully populated.
        Self {
            offset_y: 0.0,
            height: 1200.0,
        }
    }
}

impl Window {
    pub fn from_viewport(viewport: &scrollable::Viewport) -> Self {
        Self {
            offset_y: viewport.absolute_offset().y,
            height: viewport.bounds().height,
        }
    }

    /// (first, last_exclusive) row indices to materialize.
    pub fn range(&self, total: usize) -> (usize, usize) {
        let first = ((self.offset_y / ROW_HEIGHT).floor() as usize).saturating_sub(OVERSCAN);
        let count = (self.height / ROW_HEIGHT).ceil() as usize + 2 * OVERSCAN;
        let first = first.min(total);
        (first, (first + count).min(total))
    }
}

pub struct Callbacks<Message> {
    pub on_toggle: fn(usize) -> Message,
    pub on_select: fn(usize) -> Message,
    pub on_context: fn(usize) -> Message,
    pub on_scroll: fn(scrollable::Viewport) -> Message,
}

pub fn view<'a, Message: Clone + 'a>(
    rows: &'a [Row],
    window: Window,
    selected: Option<usize>,
    callbacks: Callbacks<Message>,
) -> Element<'a, Message> {
    let (first, last) = window.range(rows.len());

    // NOTE: Column::push silently DROPS children whose size hint has a Fixed(0.0) side
    // (`is_void`); spacers keep Shrink width so only zero *height* makes them vanish (fine).
    let top = space().height(first as f32 * ROW_HEIGHT);
    let bottom = space().height((rows.len() - last) as f32 * ROW_HEIGHT);

    let visible = rows[first..last]
        .iter()
        .enumerate()
        .map(|(offset, row)| {
            let index = first + offset;
            tree_row(row, index, selected == Some(index), &callbacks)
        });

    let content = Column::with_capacity(last - first + 2)
        .push(top)
        .extend(visible)
        .push(bottom)
        .width(Fill);

    scrollable(content)
        .id(tree_id())
        .on_scroll(callbacks.on_scroll)
        .width(Fill)
        .height(Fill)
        .style(style::thin_scrollbar)
        .into()
}

fn tree_row<'a, Message: Clone + 'a>(
    row_data: &'a Row,
    index: usize,
    selected: bool,
    callbacks: &Callbacks<Message>,
) -> Element<'a, Message> {
    let indent = space().width(f32::from(row_data.depth) * 16.0);

    let toggle: Element<'a, Message> = if row_data.has_children {
        button(text(if row_data.expanded { "▾" } else { "▸" }).size(14))
            .on_press((callbacks.on_toggle)(index))
            .padding([0, 4])
            .style(button::text)
            .into()
    } else {
        space().width(22).into()
    };

    let name = text(&row_data.name)
        .size(14)
        .wrapping(text::Wrapping::None) // no ellipsis in 0.14 -> clip the container instead
        .font(if row_data.depth == 0 {
            Font {
                weight: font::Weight::Bold,
                ..Font::DEFAULT
            }
        } else {
            Font::DEFAULT
        });

    let size = text(model::format_bytes(row_data.bytes))
        .size(13)
        .font(Font::MONOSPACE)
        .width(90)
        .align_x(text::Alignment::Right);

    let share = tooltip(
        progress_bar(0.0..=1.0, row_data.share)
            .length(80)
            .girth(6)
            .style(style::share_bar),
        container(text(format!("{:.1}% of parent", row_data.share * 100.0)).size(12))
            .padding(6)
            .style(container::rounded_box),
        tooltip::Position::Top,
    );

    let files = text(format!("{} files", row_data.files))
        .size(12)
        .style(style::muted)
        .width(90)
        .align_x(text::Alignment::Right);

    let line = row![
        indent,
        toggle,
        container(name).width(Fill).clip(true),
        share,
        size,
        files
    ]
    .spacing(8)
    .align_y(iced::Center)
    .height(Length::Fixed(ROW_HEIGHT));

    // A `button` would capture the left press, so an enclosing mouse_area would never see
    // on_press / on_double_click. Use a styled container + mouse_area instead, and get a hover
    // highlight for free (no messages) with the `hover(base, top)` helper.
    let base = container(line)
        .padding([0, 8])
        .width(Fill)
        .height(ROW_HEIGHT)
        .style(style::tree_row_background(selected, index % 2 == 1));

    let highlight = container(space())
        .width(Fill)
        .height(Fill)
        .style(style::row_hover_overlay);

    mouse_area(hover(base, highlight))
        .on_press((callbacks.on_select)(index))
        .on_double_click((callbacks.on_toggle)(index))
        .on_right_press((callbacks.on_context)(index))
        .interaction(mouse::Interaction::Pointer)
        .into()
}

/// The button-based variant (hover/pressed styling via button::Status). Kept to probe the
/// closure-factory style signature; double-click must then be detected manually in update().
#[allow(dead_code)]
pub fn button_row<'a, Message: Clone + 'a>(
    content: Element<'a, Message>,
    index: usize,
    selected: bool,
    on_select: Message,
) -> Element<'a, Message> {
    button(content)
        .on_press(on_select)
        .padding([0, 8])
        .width(Fill)
        .height(ROW_HEIGHT)
        .style(style::tree_row(selected, index % 2 == 1))
        .into()
}

/// Theme import kept for doc purposes: style fns receive `&Theme`.
#[allow(dead_code)]
fn _theme_marker(_: &Theme) {}
