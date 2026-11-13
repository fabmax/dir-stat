// EXPECT FAIL: Subscription::map closures must be non-capturing (const-asserted zero-sized).
use iced::{Subscription, time};
fn sub(offset: u64) -> Subscription<u64> {
    time::every(std::time::Duration::from_secs(1)).map(move |_| offset)
}
fn main() { let _ = sub(1); }
