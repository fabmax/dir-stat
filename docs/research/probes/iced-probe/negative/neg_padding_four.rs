// EXPECT FAIL: Padding is From<u16|f32|[u16;2]|[f32;2]|Pixels> only, not [_; 4]
use iced::widget::container;
fn main() {
    let _: container::Container<'static, ()> = container("x").padding([1, 2, 3, 4]);
}
