//! Running a scan from the UI (design §4.4): the sink that feeds iced, scan ids, and the
//! abortable task that turns updates into messages.

use std::time::Duration;

use dirstat_core::scan::{self, Delivery, Options, ScanHandle, ScanRequest, Update, UpdateSink};
use iced::Task;
use iced::futures::channel::mpsc;
use iced::task;

use crate::app::Message;

/// Every message from a background job carries the id of the scan it belongs to; results
/// of a replaced scan are dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScanId(pub u64);

#[derive(Debug, Default)]
pub struct ScanIdGen(u64);

impl ScanIdGen {
    pub fn next(&mut self) -> ScanId {
        self.0 += 1;
        ScanId(self.0)
    }
}

/// The core's sink over iced's channel. A newtype, because the orphan rule forbids
/// implementing the core trait for the foreign sender type.
pub struct UiSink(pub mpsc::Sender<Update>);

impl UpdateSink for UiSink {
    fn try_deliver(&mut self, update: Update) -> Delivery {
        match self.0.try_send(update) {
            Ok(()) => Delivery::Accepted,
            Err(e) if e.is_full() => Delivery::Busy(e.into_inner()),
            Err(_) => Delivery::Closed,
        }
    }

    fn deliver_final(&mut self, mut update: Update) {
        // iced's futures re-export has no executor to block on: retry until delivered.
        loop {
            match self.0.try_send(update) {
                Err(e) if e.is_full() => {
                    update = e.into_inner();
                    std::thread::sleep(Duration::from_millis(2));
                }
                _ => return, // delivered, or the UI side is gone
            }
        }
    }
}

/// A scan whose updates still matter. Dropping it aborts the task, which closes the
/// channel, and cancels the scan.
pub struct ActiveScan {
    pub id: ScanId,
    pub handle: ScanHandle,
    _abort: Option<task::Handle>,
}

/// Starts the scan (no I/O on this thread) and returns the task that delivers its updates.
pub fn launch(id: ScanId, req: ScanRequest, opts: Options) -> (ActiveScan, Task<Message>) {
    let (tx, rx) = mpsc::channel(1);
    let handle = scan::start(req, opts, UiSink(tx));
    let (task, abort) = Task::run(rx, move |update| Message::Scan(id, update)).abortable();
    let active = ActiveScan {
        id,
        handle,
        _abort: Some(abort.abort_on_drop()),
    };
    (active, task)
}

#[cfg(test)]
impl ActiveScan {
    /// No scan behind it: tests feed `Message::Scan` by hand.
    pub fn inert(id: ScanId) -> ActiveScan {
        ActiveScan {
            id,
            handle: ScanHandle::inert(),
            _abort: None,
        }
    }
}
