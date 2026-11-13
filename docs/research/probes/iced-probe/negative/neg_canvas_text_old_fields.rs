// EXPECT FAIL: canvas::Text uses align_x / align_y (horizontal_alignment / vertical_alignment gone)
use iced::widget::canvas;
fn main() {
    let _ = canvas::Text { horizontal_alignment: iced::alignment::Horizontal::Center, ..canvas::Text::default() };
}
