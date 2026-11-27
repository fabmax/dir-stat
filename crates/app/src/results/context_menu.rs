//! The tree's context menu: right-click a row for the selection bar's actions, Move to
//! Trash first.
//!
//! The row's `mouse_area` reports *which* row (`Message::RowContextMenu`); [`RightClickAt`]
//! around the whole tree pane reports *where*, in window coordinates
//! (`Message::ContextMenuAt`). Rows sit in a scrollable, which translates the cursor of
//! its children, so the position has to come from outside it. Both arrive for the same click,
//! the row's first.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::advanced::{Clipboard, Shell, mouse as amouse, overlay, renderer};
use iced::widget::{button, column, container, mouse_area, opaque, space, stack};
use iced::{Element, Event, Fill, Length, Point, Rectangle, Size, Theme, Vector, mouse};

use super::Results;
use super::rows::RowKey;
use super::selection_bar::{self, Action, Labels};
use crate::app::Message;
use crate::theme;

const WIDTH: f32 = 240.0;
const ITEM_H: f32 = 28.0;
const REASON_H: f32 = 16.0;
const PAD: f32 = 4.0;
/// Room kept between the menu and the window edge.
const MARGIN: f32 = 4.0;

/// An open (or opening) menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContextMenu {
    pub key: RowKey,
    /// Window position of the click; `None` until `ContextMenuAt` arrives.
    pub at: Option<Point>,
}

/// The menu's entries for `key`: Trash first, then the others in the bar's order.
pub fn actions(results: &Results, key: RowKey, now: i64) -> Vec<Action> {
    let Some(contents) = selection_bar::contents(results, key, None, now) else {
        return Vec::new();
    };
    let mut actions = contents.actions;
    // Stable: the rest keeps the selection bar's order.
    actions.sort_by_key(|a| !matches!(a, Action::Trash(_)));
    actions
}

fn height(actions: &[Action]) -> f32 {
    let items: f32 = actions
        .iter()
        .map(|a| ITEM_H + a.labels().disabled.map_or(0.0, |_| REASON_H))
        .sum();
    let separator = if actions.len() > 1 && matches!(actions[0], Action::Trash(_)) {
        9.0
    } else {
        0.0
    };
    items + separator + 2.0 * PAD
}

/// Top-left corner for a menu of `size` opened at `at`: below and right of the pointer,
/// flipped left / up where it would leave the window.
pub fn place(at: Point, size: Size, window: Size) -> Point {
    let x = if at.x + size.width + MARGIN > window.width {
        (at.x - size.width).max(MARGIN)
    } else {
        at.x
    };
    let y = if at.y + size.height + MARGIN > window.height {
        (at.y - size.height).max(MARGIN)
    } else {
        at.y
    };
    Point::new(x, y)
}

/// The menu layer: a backdrop that closes it on any click, and the menu card.
pub fn view<'a>(
    results: &'a Results,
    menu: ContextMenu,
    window: Size,
    now: i64,
) -> Option<Element<'a, Message>> {
    let at = menu.at?;
    let actions = actions(results, menu.key, now);
    if actions.is_empty() {
        return None;
    }
    let size = Size::new(WIDTH, height(&actions));
    let corner = place(at, size, window);

    let mut items = column![].width(Fill);
    let trash_first = actions.len() > 1 && matches!(actions[0], Action::Trash(_));
    for (i, action) in actions.iter().enumerate() {
        let Labels {
            full,
            message,
            disabled,
            ..
        } = action.labels();
        let label = theme::BODY.text(full);
        let label = if disabled.is_some() {
            label.style(theme::text_muted)
        } else {
            label
        };
        let mut content = column![container(label).height(ITEM_H).center_y(ITEM_H)];
        if let Some(why) = disabled {
            content = content.push(
                container(theme::SMALL.text(why).style(theme::text_muted))
                    .height(REASON_H)
                    .clip(true),
            );
        }
        let item = button(content)
            .padding([0, 10])
            .width(Fill)
            .style(theme::row_button)
            .on_press_maybe(
                disabled
                    .is_none()
                    .then(|| Message::ContextMenuChose(Box::new(message))),
            );
        items = items.push(item);
        if i == 0 && trash_first {
            items = items.push(
                container(
                    container(space())
                        .width(Fill)
                        .height(1)
                        .style(theme::hairline),
                )
                .padding([4, 6]),
            );
        }
    }
    let card = container(items)
        .padding(PAD)
        .width(WIDTH)
        .style(theme::popover);
    let positioned = container(opaque(card))
        .width(Fill)
        .height(Fill)
        .padding(iced::Padding {
            top: corner.y,
            left: corner.x,
            ..iced::Padding::ZERO
        });
    let backdrop = mouse_area(container(space()).width(Fill).height(Fill))
        .on_press(Message::CloseContextMenu)
        .on_right_press(Message::CloseContextMenu)
        .on_scroll(|_| Message::CloseContextMenu);
    Some(stack![backdrop, positioned].into())
}

/// Wraps `content`; a right press over it publishes `on_press(window position)`, also when a
/// child (a row's `mouse_area`) handled the press.
pub struct RightClickAt<'a> {
    content: Element<'a, Message>,
    on_press: fn(Point) -> Message,
}

impl<'a> RightClickAt<'a> {
    pub fn new(content: impl Into<Element<'a, Message>>, on_press: fn(Point) -> Message) -> Self {
        RightClickAt {
            content: content.into(),
            on_press,
        }
    }
}

impl Widget<Message, Theme, iced::Renderer> for RightClickAt<'_> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: amouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) = event
            && let Some(at) = cursor.position_over(layout.bounds())
        {
            shell.publish((self.on_press)(at));
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: amouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> amouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: amouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a> From<RightClickAt<'a>> for Element<'a, Message> {
    fn from(w: RightClickAt<'a>) -> Self {
        Element::new(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menus_open_toward_the_room_left_in_the_window() {
        let window = Size::new(1200.0, 800.0);
        let size = Size::new(240.0, 100.0);
        assert_eq!(
            place(Point::new(100.0, 200.0), size, window),
            Point::new(100.0, 200.0)
        );
        // Near the right and bottom edges it flips left and up.
        assert_eq!(
            place(Point::new(1100.0, 750.0), size, window),
            Point::new(860.0, 650.0)
        );
    }
}
