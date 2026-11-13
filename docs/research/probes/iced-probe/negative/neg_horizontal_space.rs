// EXPECT FAIL: horizontal_space()/vertical_space() removed; use space::horizontal() / space()
fn main() {
    let _: iced::widget::Space = iced::widget::horizontal_space();
}
