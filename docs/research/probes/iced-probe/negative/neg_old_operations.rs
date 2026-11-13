// EXPECT FAIL: scrollable::scroll_to / text_input::focus moved to iced::widget::operation::*
use iced::Task;
use iced::widget::{scrollable, text_input};
fn main() {
    let _: Task<()> = scrollable::scroll_to("tree", scrollable::AbsoluteOffset { x: 0.0, y: 0.0 });
    let _: Task<()> = text_input::focus("filter");
}
