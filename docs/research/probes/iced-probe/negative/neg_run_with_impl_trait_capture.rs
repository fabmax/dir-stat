// EXPECT FAIL (edition 2024): `fn(&D) -> impl Stream` captures the &D lifetime; add `+ use<>`
use iced::futures::{Stream, stream};
use iced::Subscription;
fn countdown(from: &u32) -> impl Stream<Item = u32> {
    stream::iter((0..=*from).rev().collect::<Vec<_>>())
}
fn main() { let _: Subscription<u32> = Subscription::run_with(3u32, countdown); }
