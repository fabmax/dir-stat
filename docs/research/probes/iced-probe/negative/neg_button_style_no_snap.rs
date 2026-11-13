// EXPECT FAIL: button::Style has a new `snap: bool` field; use ..button::Style::default()
use iced::widget::button;
use iced::{Border, Color, Shadow};
fn main() {
    let _ = button::Style { background: None, text_color: Color::BLACK, border: Border::default(), shadow: Shadow::default() };
}
