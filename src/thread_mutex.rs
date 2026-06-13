use std::{cell::UnsafeCell, fmt::Debug, ops::{Deref, DerefMut}, sync::atomic::{AtomicBool, Ordering}, thread::{self, ThreadId}};

pub struct ThreadMutex<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
    thread_id: UnsafeCell<ThreadId>
}

impl <T: Debug> Debug for ThreadMutex<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("RelaxedMutex")
            .field(unsafe { &*self.data.get() })
            .finish()
    }
}

unsafe impl <T: Send> Sync for ThreadMutex<T> {}
unsafe impl <T: Send> Send for ThreadMutex<T> {}

impl <T> ThreadMutex<T> {
    /// Creates a new thread protected mutex.
    pub fn new(data: T) -> ThreadMutex<T> {
        ThreadMutex { 
            locked: AtomicBool::new(false), 
            data: UnsafeCell::new(data),
            thread_id: UnsafeCell::new(std::thread::current().id())
        }
    }

    /// Locks the current mutex and returns a guard to the stored value.
    /// This function will block until a lock can be achieved on the stored value.
    /// The value will remain locked until the guard is dropped.
    /// If the same thread tries to lock this mutex while it is locked by the same thread,
    /// a panic will occur.
    pub fn lock(&self) -> ThreadMutexGuard<'_, T> {
        let my_thread_id = std::thread::current().id();
        // spin lock until a lock is acquired
        loop {
            if !self.locked.swap(true, Ordering::Acquire) {
                break;
            }

            // if we made it this far, the mutex is locked, make sure its not locked by this thread
            let is_locked_by_self = unsafe { *self.thread_id.get() } == my_thread_id;
            if is_locked_by_self {
                panic!("Mutex deadlocked by the same thread attempting to lock the same mutex twice at one time.")
            }

            thread::yield_now();
        }

        unsafe { *self.thread_id.get() = my_thread_id; }
        ThreadMutexGuard { mutex: self }
    }

    /// This function will lock the stored value and return a guard
    /// only if it is not currently locked.  Otherwise, an empty
    /// option will be returned.
    pub fn lock_now(&self) -> Option<ThreadMutexGuard<'_, T>> {
        if !self.locked.swap(true, Ordering::Acquire) {
            let my_thread_id = std::thread::current().id();
            unsafe { *self.thread_id.get() = my_thread_id; }
            Some(ThreadMutexGuard { mutex: self })
        } else {
            None
        }
    }

    /// This removes any existing locks on this mutex.
    /// This is meant to unlock the mutex when a poison occurs.
    /// DO NOT USE THIS UNLESS ABSOLUTELY NECESSARY.
    #[allow(dead_code)]
    pub(crate) fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

pub struct ThreadMutexGuard<'a, T> {
    mutex: &'a ThreadMutex<T>
}

impl <'a, T> Deref for ThreadMutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.data.get() }
    }
}

impl <'a, T> DerefMut for ThreadMutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl <'a, T> Drop for ThreadMutexGuard<'a, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
    }
}
