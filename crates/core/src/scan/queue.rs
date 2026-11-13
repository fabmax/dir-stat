//! Work-stealing job queue for the scan workers (scan-bench `queue.rs`, stealing variant).
//!
//! One deque per worker. A worker pushes its children to its own deque and pops from the back
//! (depth first in its own subtree); an idle worker steals from the *front* of another deque
//! (the oldest, shallowest job, which has the largest remaining subtree). Workers therefore
//! spread over different subtrees, and the top-level folders complete early.
//!
//! Termination: `outstanding` (queued + in-flight jobs) grows by the children *before* it
//! shrinks for the retired job, so it reaches 0 exactly once, when everything is done.
//! Sleeping uses an epoch counter, so a wake-up can never be lost.

use std::collections::VecDeque;
use std::sync::atomic::Ordering::{Relaxed, SeqCst};
use std::sync::atomic::{AtomicBool, AtomicUsize, fence};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

pub struct StealingQueue<J> {
    locals: Vec<Mutex<VecDeque<J>>>,
    outstanding: AtomicUsize,
    /// Workers that announced they are about to sleep.
    sleepers: AtomicUsize,
    /// Bumped (under the lock) whenever sleepers should re-check for work.
    epoch: Mutex<u64>,
    wake: Condvar,
}

/// A worker that panicked while holding a lock must not take the scan down with it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<J> StealingQueue<J> {
    pub fn new(workers: usize, root: J) -> Self {
        let locals: Vec<_> = (0..workers.max(1))
            .map(|_| Mutex::new(VecDeque::new()))
            .collect();
        lock(&locals[0]).push_back(root);
        StealingQueue {
            locals,
            outstanding: AtomicUsize::new(1),
            sleepers: AtomicUsize::new(0),
            epoch: Mutex::new(0),
            wake: Condvar::new(),
        }
    }

    pub fn outstanding(&self) -> usize {
        self.outstanding.load(Relaxed)
    }

    fn find(&self, me: usize) -> Option<J> {
        if let Some(job) = lock(&self.locals[me]).pop_back() {
            return Some(job);
        }
        let n = self.locals.len();
        (1..n).find_map(|k| lock(&self.locals[(me + k) % n]).pop_front())
    }

    /// Blocks until a job is available. `None`: the scan is complete or cancelled.
    pub fn pop(&self, me: usize, cancelled: &AtomicBool) -> Option<J> {
        loop {
            if cancelled.load(Relaxed) || self.outstanding.load(SeqCst) == 0 {
                return None;
            }
            if let Some(job) = self.find(me) {
                return Some(job);
            }
            // Announce the intent to sleep, then look once more. A producer pushes, fences, then
            // reads `sleepers`; we increment `sleepers`, fence, then look (Dekker): one of the two
            // always sees the other, so a concurrently pushed job is either found here or the
            // producer bumps the epoch and we don't sleep (or are woken).
            let epoch = *lock(&self.epoch);
            self.sleepers.fetch_add(1, SeqCst);
            fence(SeqCst);
            if let Some(job) = self.find(me) {
                self.sleepers.fetch_sub(1, SeqCst);
                return Some(job);
            }
            let mut e = lock(&self.epoch);
            while *e == epoch && !cancelled.load(Relaxed) && self.outstanding.load(SeqCst) != 0 {
                e = self.wake.wait(e).unwrap_or_else(PoisonError::into_inner);
            }
            drop(e);
            self.sleepers.fetch_sub(1, SeqCst);
        }
    }

    /// Publishes the children of the job `me` just finished, then retires that job.
    pub fn complete(&self, me: usize, children: &mut Vec<J>) {
        let n = children.len();
        if n > 0 {
            self.outstanding.fetch_add(n, SeqCst);
            lock(&self.locals[me]).extend(children.drain(..));
            fence(SeqCst); // pairs with the fence in `pop` (push -> fence -> read `sleepers`)
        }
        if self.outstanding.fetch_sub(1, SeqCst) == 1 {
            self.bump_epoch(usize::MAX); // scan complete: everybody wakes up and exits
        } else if n > 1 && self.sleepers.load(SeqCst) > 0 {
            self.bump_epoch(n - 1); // we keep one child for ourselves
        }
    }

    /// Wakes every sleeping worker (after the cancel flag was set).
    pub fn wake_all(&self) {
        self.bump_epoch(usize::MAX);
    }

    fn bump_epoch(&self, wake: usize) {
        *lock(&self.epoch) += 1;
        if wake >= self.locals.len() {
            self.wake.notify_all();
        } else {
            for _ in 0..wake {
                self.wake.notify_one();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    /// A synthetic tree: job `n` has children `n*3+1 ..= n*3+3` while they are below `limit`.
    #[test]
    fn every_job_runs_exactly_once() {
        const LIMIT: u64 = 20_000;
        for workers in [1, 3, 8] {
            let queue = StealingQueue::new(workers, 0u64);
            let cancelled = AtomicBool::new(false);
            let (sum, count) = (AtomicU64::new(0), AtomicU64::new(0));
            std::thread::scope(|s| {
                for me in 0..workers {
                    let (queue, cancelled, sum, count) = (&queue, &cancelled, &sum, &count);
                    s.spawn(move || {
                        let mut children = Vec::new();
                        while let Some(job) = queue.pop(me, cancelled) {
                            sum.fetch_add(job, Relaxed);
                            count.fetch_add(1, Relaxed);
                            children.extend((job * 3 + 1..=job * 3 + 3).filter(|&c| c < LIMIT));
                            queue.complete(me, &mut children);
                        }
                    });
                }
            });
            assert_eq!(count.into_inner(), LIMIT, "{workers} workers");
            assert_eq!(sum.into_inner(), LIMIT * (LIMIT - 1) / 2);
            assert_eq!(queue.outstanding(), 0);
        }
    }

    #[test]
    fn cancel_wakes_sleepers() {
        let queue = StealingQueue::new(4, 0u32);
        let cancelled = AtomicBool::new(false);
        std::thread::scope(|s| {
            // Worker 0 takes the root and holds it, so the others go to sleep.
            let root = queue.pop(0, &cancelled);
            assert_eq!(root, Some(0));
            let sleepers: Vec<_> = (1..4)
                .map(|me| {
                    s.spawn({
                        let (queue, cancelled) = (&queue, &cancelled);
                        move || queue.pop(me, cancelled)
                    })
                })
                .collect();
            std::thread::sleep(std::time::Duration::from_millis(50));
            cancelled.store(true, Relaxed);
            queue.wake_all();
            for sleeper in sleepers {
                assert_eq!(sleeper.join().unwrap(), None);
            }
        });
    }
}
