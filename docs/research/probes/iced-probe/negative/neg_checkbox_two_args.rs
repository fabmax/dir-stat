// EXPECT FAIL: checkbox(is_checked) takes ONE argument; label via .label(..)
fn main() {
    let _: iced::widget::Checkbox<'static, ()> = iced::widget::checkbox("Show hidden", true);
}
