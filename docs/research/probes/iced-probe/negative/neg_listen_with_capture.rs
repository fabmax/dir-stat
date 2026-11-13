// EXPECT FAIL: event::listen_with takes `fn(Event, Status, window::Id) -> Option<M>` (no captures)
use iced::{Subscription, event};
fn sub(tag: u32) -> Subscription<u32> {
    event::listen_with(move |_event, _status, _id| Some(tag))
}
fn main() { let _ = sub(1); }
