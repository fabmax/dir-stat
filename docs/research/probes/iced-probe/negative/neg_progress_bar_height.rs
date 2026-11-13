// EXPECT FAIL: ProgressBar uses .length(..)/.girth(..) instead of .width/.height
fn main() {
    let _: iced::widget::ProgressBar<'static> = iced::widget::progress_bar(0.0..=1.0, 0.5).height(8);
}
