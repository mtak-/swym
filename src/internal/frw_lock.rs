//! A very simple and cheap spinning RwLock. These are used for protection of the gc's
//! GlobalThreadList.
//!
//! TODO: it's likely that parking_lot's RwLocks are totally adequate/amazing for this purpose,
//! gotta bench and see.

use lock_api::{GuardNoSend, RawRwLock};
use std::{
    mem,
    sync::atomic::{AtomicUsize, Ordering::*},
};

const WRITE_BIT: usize = (1 as usize) << (mem::size_of::<usize>() * 8 - 1);
const READ_MASK: usize = !WRITE_BIT;

#[inline]
const fn write_locked(val: usize) -> bool {
    val & WRITE_BIT != 0
}

#[inline]
const fn shared_locked(val: usize) -> bool {
    val & READ_MASK != 0
}

#[derive(Debug)]
pub struct FrwLock {
    read_count: AtomicUsize,
}

unsafe impl RawRwLock for FrwLock {
    const INIT: FrwLock = FrwLock {
        read_count: AtomicUsize::new(0),
    };
    type GuardMarker = GuardNoSend; // TODO: might be send?

    // if done right, this func on x86_64 is..
    //      lock incq (%rdi)
    //      jle  slow_path
    //      retq
    #[inline]
    fn lock_shared(&self) {
        // TODO: theoretical overflow possible...
        if unlikely!(write_locked(self.read_count.fetch_add(1, Acquire))) {
            self.lock_shared_slow();
        }
    }

    #[inline]
    fn unlock_shared(&self) {
        let _prev = self.read_count.fetch_sub(1, Release);
        debug_assert!(
            shared_locked(_prev),
            "attempt to unlock an unlocked `FrwLock`"
        );
    }

    #[inline]
    fn lock_exclusive(&self) {
        let test = self
            .read_count
            .compare_exchange_weak(0, WRITE_BIT, Acquire, Relaxed);
        if unlikely!(test.is_err()) {
            self.lock_exclusive_slow()
        }
    }

    #[inline]
    fn unlock_exclusive(&self) {
        let _prev = self.read_count.fetch_and(READ_MASK, Release);
        debug_assert!(
            write_locked(_prev),
            "attempt to unlock an unlocked `FrwLock`"
        );
    }

    #[inline]
    fn try_lock_shared(&self) -> bool {
        unimplemented!()
    }

    #[inline]
    fn try_lock_exclusive(&self) -> bool {
        unimplemented!()
    }
}

impl FrwLock {
    pub const INIT_LOCKED: Self = FrwLock {
        read_count: AtomicUsize::new(WRITE_BIT),
    };

    #[cold]
    #[inline(never)]
    fn lock_shared_slow(&self) {
        self.read_count.fetch_sub(1, Relaxed);

        loop {
            backoff();
            let read_state = self
                .read_count
                .load(Relaxed)
                .checked_add(1)
                .expect("overflowed the maximum number of read locks on `FrwLock`");
            if !write_locked(read_state)
                && self
                    .read_count
                    .compare_exchange_weak(read_state - 1, read_state, Acquire, Relaxed)
                    .is_ok()
            {
                break;
            }
        }

        debug_assert!(shared_locked(self.read_count.load(Relaxed)));
    }

    #[inline]
    fn request_exclusive_lock(&self) -> usize {
        let mut prev_read_count = self.read_count.load(Relaxed);
        // first come first serve
        while write_locked(prev_read_count)
            || self
                .read_count
                .compare_exchange_weak(
                    prev_read_count,
                    prev_read_count | WRITE_BIT,
                    Acquire,
                    Relaxed,
                )
                .is_err()
        {
            backoff();
            prev_read_count = self.read_count.load(Relaxed);
        }

        prev_read_count
    }

    #[inline]
    fn wait_for_readers(&self, mut prev_read_count: usize) {
        while likely!(shared_locked(prev_read_count)) {
            backoff();
            prev_read_count = self.read_count.load(Acquire);
        }
    }

    #[inline(never)]
    #[cold]
    fn lock_exclusive_slow(&self) {
        let prev_read_count = self.request_exclusive_lock();
        self.wait_for_readers(prev_read_count);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn send_sync<T: Send + Sync>() {}

    #[test]
    fn is_send_sync() {
        send_sync::<FrwLock>()
    }
}

// TODO: better backoff options
#[doc(hidden)]
#[inline]
pub fn backoff() {
    std::sync::atomic::spin_loop_hint()
}
