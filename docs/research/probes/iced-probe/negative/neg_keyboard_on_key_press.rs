// EXPECT FAIL: keyboard::on_key_press was removed; use keyboard::listen().filter_map(..)
fn main() {
    let _ = iced::keyboard::on_key_press(|_key, _modifiers| Some(()));
}
