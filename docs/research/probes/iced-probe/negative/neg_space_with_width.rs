// EXPECT FAIL: Space::with_width removed; use space().width(..) / Space::new().width(..)
fn main() {
    let _ = iced::widget::Space::with_width(10);
}
