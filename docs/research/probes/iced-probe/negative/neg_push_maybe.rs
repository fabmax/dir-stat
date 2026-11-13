// EXPECT FAIL: Column::push_maybe removed; push(Option<impl Into<Element>>) instead
use iced::widget::{Column, text};
fn main() {
    let _: Column<'static, ()> = Column::new().push_maybe(Some(text("x")));
}
