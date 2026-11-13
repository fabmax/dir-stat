// EXPECT FAIL: Subscription::run_with takes a fn pointer `fn(&D) -> S` (no captures)
use iced::futures::stream;
use iced::Subscription;
fn sub(extra: u32) -> Subscription<u32> {
    Subscription::run_with(1u32, move |d| stream::iter([*d + extra]))
}
fn main() { let _ = sub(1); }
