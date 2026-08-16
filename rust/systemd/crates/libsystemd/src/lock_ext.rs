//! Poison-recovering lock extension traits.
//!
//! When a thread panics while holding a `Mutex` or `RwLock`, the lock becomes
//! "poisoned" — all subsequent `.lock()` / `.read()` / `.write()` calls return
//! `Err(PoisonError)`. In a service manager (PID 1) we must **never** cascade
//! one thread's panic into every other thread, so we recover the inner data
//! from the `PoisonError` and continue.
//!
//! Usage:
//! ```ignore
//! use crate::lock_ext::LockExt;
//!
//! let data = my_mutex.lock_poisoned();
//! let data = my_rwlock.read_poisoned();
//! let data = my_rwlock.write_poisoned();
//! ```

use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Extension trait that adds poison-recovering methods to `Mutex`.
pub trait MutexExt<T> {
    /// Acquire the mutex, recovering from a poisoned state.
    ///
    /// If the mutex was poisoned (a thread panicked while holding it),
    /// the data is recovered and a warning is logged once.
    fn lock_poisoned(&self) -> MutexGuard<'_, T>;
}

/// Extension trait that adds poison-recovering methods to `RwLock`.
pub trait RwLockExt<T> {
    /// Acquire a read lock, recovering from a poisoned state.
    fn read_poisoned(&self) -> RwLockReadGuard<'_, T>;

    /// Acquire a write lock, recovering from a poisoned state.
    fn write_poisoned(&self) -> RwLockWriteGuard<'_, T>;

    /// Acquire a write lock WITHOUT registering as a blocking pending writer.
    ///
    /// glibc's `RwLock` is writer-preferring: a thread blocked in `write()`
    /// registers a pending writer that blocks ALL subsequent `read()` requests
    /// until it acquires the lock. On PID 1's single-threaded control-socket
    /// loop that is a deadlock hazard — activation worker threads hold read
    /// locks on the `RuntimeInfo` for extended periods and may need to acquire
    /// further read locks to make progress (and release the ones they hold); a
    /// blocking writer there freezes the whole activation, which in turn stalls
    /// udevd (its `udev-event` notifications go unread) and drops device
    /// uevents. Poll `try_write()` with a short sleep instead: `try_write()`
    /// never registers a pending writer, so readers keep flowing and we simply
    /// retry until the lock is momentarily free. Recovers from poisoning like
    /// [`write_poisoned`](RwLockExt::write_poisoned).
    fn write_poisoned_nonblocking(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_poisoned(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| {
            log::warn!("Recovered poisoned Mutex (a thread panicked while holding this lock)");
            e.into_inner()
        })
    }
}

impl<T> RwLockExt<T> for RwLock<T> {
    fn read_poisoned(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(|e| {
            log::warn!(
                "Recovered poisoned RwLock (read) (a thread panicked while holding this lock)"
            );
            e.into_inner()
        })
    }

    fn write_poisoned(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(|e| {
            log::warn!(
                "Recovered poisoned RwLock (write) (a thread panicked while holding this lock)"
            );
            e.into_inner()
        })
    }

    fn write_poisoned_nonblocking(&self) -> RwLockWriteGuard<'_, T> {
        loop {
            match self.try_write() {
                Ok(guard) => return guard,
                Err(std::sync::TryLockError::Poisoned(e)) => {
                    log::warn!(
                        "Recovered poisoned RwLock (write) (a thread panicked while holding this lock)"
                    );
                    return e.into_inner();
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
    }
}
