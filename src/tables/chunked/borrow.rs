//! The borrow flag every row of every column in a [`ChunkedTable`](crate::ChunkedTable)
//! has, a reader count or a writer bit in one atomic.
//!
//! Writers are preferred: once a writer is waiting, new readers wait too, so a stream of
//! readers can't starve it.  Like the rest of anarchy's locks they aren't reentrant, so
//! taking a flag again on a thread that already holds it mutably deadlocks.

use std::sync::atomic::{AtomicU32, Ordering};

/// Set while a writer holds the flag or waits for its readers to leave.
const WRITER: u32 = 1 << 31;

/// Spins for a while, then yields to other threads.
fn backoff(spins: &mut u32) {
    *spins += 1;
    if *spins < 64 { std::hint::spin_loop() } else { std::thread::yield_now() }
}

/// Takes a shared borrow if no writer holds or waits for the flag.
pub(crate) fn try_read(flag: &AtomicU32) -> bool {
    let mut state = flag.load(Ordering::Relaxed);
    while state & WRITER == 0 {
        match flag.compare_exchange_weak(state, state + 1, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => return true,
            Err(now) => state = now
        }
    }
    false
}

/// Takes a shared borrow, waiting for writers.
pub(crate) fn read(flag: &AtomicU32) {
    let mut spins = 0;
    while !try_read(flag) { backoff(&mut spins) }
}

/// Takes `extra` more shared borrows of a flag the caller already reads, so they can't wait
/// on a writer that is itself waiting for the caller.
pub(crate) fn read_again(flag: &AtomicU32, extra: u32) {
    flag.fetch_add(extra, Ordering::Relaxed);
}

/// Releases `count` shared borrows.
pub(crate) fn unlock_read(flag: &AtomicU32, count: u32) {
    flag.fetch_sub(count, Ordering::Release);
}

/// Takes the flag mutably if nothing holds it.
pub(crate) fn try_write(flag: &AtomicU32) -> bool {
    flag.compare_exchange(0, WRITER, Ordering::Acquire, Ordering::Relaxed).is_ok()
}

/// Takes the flag mutably, waiting for other writers and then for its readers.
pub(crate) fn write(flag: &AtomicU32) {
    let mut spins = 0;
    loop {
        let state = flag.load(Ordering::Relaxed);
        if state & WRITER == 0 && flag.compare_exchange_weak(state, state | WRITER, Ordering::Acquire, Ordering::Relaxed).is_ok() { break }
        backoff(&mut spins);
    }
    while flag.load(Ordering::Acquire) != WRITER { backoff(&mut spins) }
}

/// Releases a mutable borrow.
pub(crate) fn unlock_write(flag: &AtomicU32) {
    flag.store(0, Ordering::Release);
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readers_share_writers_exclude() {
        let flag = AtomicU32::new(0);
        assert!(try_read(&flag) && try_read(&flag));
        assert!(!try_write(&flag));
        read_again(&flag, 2);
        unlock_read(&flag, 4);
        assert!(try_write(&flag));
        assert!(!try_read(&flag) && !try_write(&flag));
        unlock_write(&flag);
        assert!(try_read(&flag));
        unlock_read(&flag, 1);
    }

    #[test]
    fn waiting_writer_blocks_new_readers() {
        let flag = AtomicU32::new(0);
        read(&flag);
        std::thread::scope(|s| {
            let writer = s.spawn(|| { write(&flag); unlock_write(&flag); });
            // once the writer is waiting, readers have to wait for it
            while flag.load(Ordering::Relaxed) & WRITER == 0 { std::thread::yield_now() }
            assert!(!try_read(&flag));
            unlock_read(&flag, 1);
            writer.join().unwrap();
        });
        assert_eq!(flag.load(Ordering::Relaxed), 0);
    }
}
