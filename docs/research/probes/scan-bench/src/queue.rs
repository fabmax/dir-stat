//! Work queues for the scanner's worker pool. A job is one directory to list.
//!
//! Both variants implement the same contract:
//! * `pop(worker)` blocks until a job is available; `None` means "scan complete or cancelled".
//! * `complete(worker, children)` publishes the children of the job the worker just finished and
//!   retires that job. `outstanding` (= queued + in-flight jobs) is increased by the children
//!   *before* it is decreased for the retired job, so it reaches 0 exactly once, when everything
//!   is done. That is the whole termination protocol.
//!
//! * [`Schedule::SharedLifo`] / [`Schedule::SharedFifo`]: one `Mutex<VecDeque>` + `Condvar`.
//!   Simplest possible design. Idle workers grab the most recently pushed jobs, i.e. they tend to
//!   list *sibling* directories at the same time.
//! * [`Schedule::WorkStealing`]: one deque per worker. A worker pushes its children to its own
//!   deque and pops from the back (depth-first in its own subtree); an idle worker steals from the
//!   *front* of another worker's deque (the oldest = shallowest = largest remaining subtree).
//!   Workers therefore spread over different subtrees. std-only; sleeping uses an epoch counter
//!   ("eventcount") so that a wake-up can never be lost.

use std::collections::VecDeque;
use std::sync::atomic::{fence, AtomicBool, AtomicUsize, Ordering::{Relaxed, SeqCst}};
use std::sync::{Condvar, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Schedule {
    SharedLifo,
    SharedFifo,
    WorkStealing,
}

pub enum WorkQueue<J> {
    Shared(SharedQueue<J>),
    Stealing(StealingQueue<J>),
}

impl<J> WorkQueue<J> {
    pub fn new(schedule: Schedule, workers: usize, root: J) -> Self {
        match schedule {
            Schedule::SharedLifo | Schedule::SharedFifo => WorkQueue::Shared(SharedQueue {
                lifo: schedule == Schedule::SharedLifo,
                state: Mutex::new(SharedState { jobs: VecDeque::from([root]), outstanding: 1 }),
                cv: Condvar::new(),
            }),
            Schedule::WorkStealing => {
                let locals: Vec<_> = (0..workers.max(1)).map(|_| Mutex::new(VecDeque::new())).collect();
                locals[0].lock().unwrap().push_back(root);
                WorkQueue::Stealing(StealingQueue {
                    locals,
                    outstanding: AtomicUsize::new(1),
                    sleepers: AtomicUsize::new(0),
                    epoch: Mutex::new(0),
                    wake: Condvar::new(),
                })
            }
        }
    }

    pub fn pop(&self, worker: usize, cancelled: &AtomicBool) -> Option<J> {
        match self {
            WorkQueue::Shared(q) => q.pop(cancelled),
            WorkQueue::Stealing(q) => q.pop(worker, cancelled),
        }
    }

    pub fn complete(&self, worker: usize, children: &mut Vec<J>) {
        match self {
            WorkQueue::Shared(q) => q.complete(children),
            WorkQueue::Stealing(q) => q.complete(worker, children),
        }
    }

    /// Wake every sleeping worker (after setting the cancel flag).
    pub fn wake_all(&self) {
        match self {
            WorkQueue::Shared(q) => {
                // Taking the lock guarantees no worker is between its flag check and `wait()`.
                let _s = q.state.lock().unwrap();
                q.cv.notify_all();
            }
            WorkQueue::Stealing(q) => q.bump_epoch(usize::MAX),
        }
    }

    pub fn outstanding(&self) -> usize {
        match self {
            WorkQueue::Shared(q) => q.state.lock().unwrap().outstanding,
            WorkQueue::Stealing(q) => q.outstanding.load(Relaxed),
        }
    }
}

pub struct SharedQueue<J> {
    lifo: bool,
    state: Mutex<SharedState<J>>,
    cv: Condvar,
}

struct SharedState<J> {
    jobs: VecDeque<J>,
    outstanding: usize,
}

impl<J> SharedQueue<J> {
    fn pop(&self, cancelled: &AtomicBool) -> Option<J> {
        let mut s = self.state.lock().unwrap();
        loop {
            if cancelled.load(Relaxed) || s.outstanding == 0 {
                return None;
            }
            let job = if self.lifo { s.jobs.pop_back() } else { s.jobs.pop_front() };
            if job.is_some() {
                return job;
            }
            s = self.cv.wait(s).unwrap();
        }
    }

    fn complete(&self, children: &mut Vec<J>) {
        let n = children.len();
        let mut s = self.state.lock().unwrap();
        s.jobs.extend(children.drain(..));
        s.outstanding = s.outstanding + n - 1;
        let done = s.outstanding == 0;
        drop(s);
        if done || n > 1 {
            self.cv.notify_all();
        } else if n == 1 {
            self.cv.notify_one();
        }
    }
}

pub struct StealingQueue<J> {
    locals: Vec<Mutex<VecDeque<J>>>,
    outstanding: AtomicUsize,
    /// Workers that announced they are about to sleep.
    sleepers: AtomicUsize,
    /// Bumped (under the lock) whenever sleepers should re-check for work.
    epoch: Mutex<u64>,
    wake: Condvar,
}

impl<J> StealingQueue<J> {
    fn find(&self, me: usize) -> Option<J> {
        if let Some(j) = self.locals[me].lock().unwrap().pop_back() {
            return Some(j);
        }
        let n = self.locals.len();
        (1..n).find_map(|k| self.locals[(me + k) % n].lock().unwrap().pop_front())
    }

    fn pop(&self, me: usize, cancelled: &AtomicBool) -> Option<J> {
        loop {
            if cancelled.load(Relaxed) || self.outstanding.load(SeqCst) == 0 {
                return None;
            }
            if let Some(j) = self.find(me) {
                return Some(j);
            }
            // Announce the intent to sleep, then look once more. A producer pushes, fences, then
            // reads `sleepers`; we increment `sleepers`, fence, then look (Dekker): one of the two
            // always sees the other, so a job pushed concurrently is either found here or the
            // producer bumps the epoch and we do not sleep (or are woken). Even a lost wake-up
            // could not strand a job: the producer itself drains its own deque before sleeping.
            let epoch = *self.epoch.lock().unwrap();
            self.sleepers.fetch_add(1, SeqCst);
            fence(SeqCst);
            if let Some(j) = self.find(me) {
                self.sleepers.fetch_sub(1, SeqCst);
                return Some(j);
            }
            let mut e = self.epoch.lock().unwrap();
            while *e == epoch && !cancelled.load(Relaxed) && self.outstanding.load(SeqCst) != 0 {
                e = self.wake.wait(e).unwrap();
            }
            drop(e);
            self.sleepers.fetch_sub(1, SeqCst);
        }
    }

    fn complete(&self, me: usize, children: &mut Vec<J>) {
        let n = children.len();
        if n > 0 {
            self.outstanding.fetch_add(n, SeqCst);
            self.locals[me].lock().unwrap().extend(children.drain(..));
            fence(SeqCst); // pairs with the fence in `pop` (push -> fence -> read `sleepers`)
        }
        if self.outstanding.fetch_sub(1, SeqCst) == 1 {
            self.bump_epoch(usize::MAX); // scan complete: everybody wakes up and exits
        } else if n > 1 && self.sleepers.load(SeqCst) > 0 {
            self.bump_epoch(n - 1); // we keep one child for ourselves
        }
    }

    fn bump_epoch(&self, wake: usize) {
        *self.epoch.lock().unwrap() += 1;
        if wake >= self.locals.len() {
            self.wake.notify_all();
        } else {
            for _ in 0..wake {
                self.wake.notify_one();
            }
        }
    }
}
