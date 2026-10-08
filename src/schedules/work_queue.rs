//! Work queue entries, for splitting a system's work across threads.

use std::{iter::Peekable, sync::atomic::{AtomicBool, Ordering}};

use mutual::{RelaxedMutex, SharedData};

/// A unit of submittable work that may be run by any number of threads.
/// An entry may be run once or many times; `is_finished` is the signal
/// that tells the queue the entry has no more work to hand out and can be
/// dropped instead of being re-queued.
///
/// Warning: make sure if run can be called at anytime, even after is_finished
/// returns true.  This is an assumption that should be used to allow for less
/// locks in sharing work queues
pub trait WorkQueueEntry: Send + Sync {
    /// Claims and runs a single piece of work from this entry.  Must be
    /// safe to call concurrently and must do nothing if already finished.
    fn run(&self);

    /// Returns true once all work has been claimed from this entry.
    fn is_finished(&self) -> bool;

    /// Run this entry on the calling thread until all of its work has been
    /// claimed.  Work claimed by other threads may still be running when this returns.
    fn complete(&self) {
        while !self.is_finished() {
            self.run();
        }
    }
}

/// Runs `function` over every item of `iterator`, letting each thread
/// that picks up this entry pull and process one item per `run`.
pub struct ParIter<Function, Iter>
    where Function: Fn(Iter::Item) + Send + Sync, Iter: Iterator + Send, Iter::Item: Send
{
    function: Function,
    iterator: RelaxedMutex<Peekable<Iter>>,
    finished: AtomicBool
}

impl <Function, Iter> ParIter<Function, Iter>
    where Function: Fn(Iter::Item) + Send + Sync, Iter: Iterator + Send, Iter::Item: Send
{
    /// Create an entry that calls `function` on every item of `iterator`.
    pub fn new(iterator: Iter, function: Function) -> Self {
        let mut iterator = iterator.peekable();
        let finished = iterator.peek().is_none();
        Self { function, iterator: RelaxedMutex::new(iterator), finished: AtomicBool::new(finished) }
    }
}

impl <Function, Iter> WorkQueueEntry for ParIter<Function, Iter>
    where Function: Fn(Iter::Item) + Send + Sync, Iter: Iterator + Send, Iter::Item: Send
{
    fn run(&self) {
        // only hold the lock long enough to claim an item
        let item = {
            let mut iterator = self.iterator.lock_mut();
            if self.finished.load(Ordering::Acquire) { return; }

            let item = iterator.next();
            // peek so the entry is marked finished as soon as the last item
            // is claimed, rather than needing an extra empty run
            if iterator.peek().is_none() { self.finished.store(true, Ordering::Release); }
            item
        };

        if let Some(item) = item { (self.function)(item); }
    }

    fn is_finished(&self) -> bool { self.finished.load(Ordering::Acquire) }
}

/// An entry that runs its function exactly once.
pub struct SingleRun<Function: FnOnce() + Send> {
    function: RelaxedMutex<Option<Function>>,
    finished: AtomicBool
}

impl <Function: FnOnce() + Send> SingleRun<Function> {
    /// Create an entry that calls `function` once.
    pub fn new(function: Function) -> Self {
        Self { function: RelaxedMutex::new(Some(function)), finished: AtomicBool::new(false) }
    }
}

impl <Function: FnOnce() + Send> WorkQueueEntry for SingleRun<Function> {
    fn run(&self) {
        let function = {
            let mut function = self.function.lock_mut();
            self.finished.store(true, Ordering::Release);
            function.take()
        };

        if let Some(function) = function { function(); }
    }

    fn is_finished(&self) -> bool { self.finished.load(Ordering::Acquire) }
}
