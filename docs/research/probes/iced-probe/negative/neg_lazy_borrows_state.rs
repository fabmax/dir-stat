// EXPECT FAIL: lazy's view must return Element<'static, ..> (cannot borrow app state)
use iced::Element;
use iced::widget::{lazy, text};
struct App { name: String }
fn view(app: &App) -> Element<'_, ()> {
    lazy(app.name.len(), |_| text(&app.name)).into()
}
fn main() { let _ = view(&App { name: String::new() }); }
