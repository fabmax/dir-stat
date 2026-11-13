// EXPECT FAIL: 0.14 application(boot, update, view) — the title is set with .title(..)
fn update(_: &mut u32, _: ()) {}
fn view(_: &u32) -> iced::Element<'_, ()> { iced::widget::text("x").into() }
fn main() -> iced::Result {
    iced::application("dir-stat", update, view).run()
}
