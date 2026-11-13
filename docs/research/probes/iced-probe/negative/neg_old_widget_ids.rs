// EXPECT FAIL: per-widget Id types (scrollable::Id, text_input::Id) are gone; use iced::widget::Id
fn main() {
    let _ = iced::widget::scrollable::Id::new("tree");
    let _ = iced::widget::text_input::Id::new("filter");
}
