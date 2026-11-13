// EXPECT FAIL: horizontal_rule(..) removed; use rule::horizontal(..)
fn main() {
    let _: iced::widget::Rule<'static> = iced::widget::horizontal_rule(1);
}
