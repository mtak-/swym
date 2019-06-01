use core::{
    sync::atomic::{AtomicU8, Ordering},
    time::Duration,
};
use lock_api::{GuardNoSend, RawMutex as RawMutexTrait};
use parking_lot_core::{self, ParkResult, SpinWait, UnparkResult, UnparkToken, DEFAULT_PARK_TOKEN};
use std::time::Instant;

type U8 = u8;

const TOKEN_NORMAL: UnparkToken = UnparkToken(0);
const LOCKED_BIT: U8 = 1;
const PARKED_BIT: U8 = 2;

#[inline]
pub fn to_deadline(timeout: Duration) -> Option<Instant> {
    #[cfg(has_checked_instant)]
    let deadline = Instant::now().checked_add(timeout);
    #[cfg(not(has_checked_instant))]
    let deadline = Some(Instant::now() + timeout);

    deadline
}

/// Raw mutex type backed by the parking lot.
pub struct Starvation {
    state: AtomicU8,
}

unsafe impl RawMutexTrait for Starvation {
    const INIT: Starvation = Starvation {
        state: AtomicU8::new(0),
    };

    type GuardMarker = GuardNoSend;

    #[inline]
    fn lock(&self) {
        if self
            .state
            .compare_exchange_weak(0, LOCKED_BIT, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            drop(self.lock_slow());
        }
    }

    #[inline]
    fn try_lock(&self) -> bool {
        unimplemented!()
    }

    #[inline]
    fn unlock(&self) {
        if self
            .state
            .compare_exchange(LOCKED_BIT, 0, Ordering::Release, Ordering::Relaxed)
            .is_ok()
        {
            return;
        }
        self.unlock_slow(false);
    }
}

impl Starvation {
    #[inline]
    pub fn wait_unlocked(&self) {
        let state = self.state.load(Ordering::Relaxed);
        if unlikely!(state & LOCKED_BIT != 0) {
            self.wait_unlocked_slow()
        }
    }

    #[cold]
    #[inline(never)]
    fn wait_unlocked_slow(&self) {
        let mut spinwait = SpinWait::new();
        let mut state = self.state.load(Ordering::Relaxed);
        loop {
            // Grab the lock if it isn't locked, even if there is a queue on it
            if state & LOCKED_BIT == 0 {
                return;
            }

            // If there is no queue, try spinning a few times
            if state & PARKED_BIT == 0 && spinwait.spin() {
                state = self.state.load(Ordering::Relaxed);
                continue;
            }

            // Set the parked bit
            if state & PARKED_BIT == 0 {
                if let Err(x) = self.state.compare_exchange_weak(
                    state,
                    state | PARKED_BIT,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    state = x;
                    continue;
                }
            }

            // Park our thread until we are woken up by an unlock
            unsafe {
                let addr = self as *const _ as usize;
                let validate = || self.state.load(Ordering::Relaxed) == LOCKED_BIT | PARKED_BIT;
                let before_sleep = || {};
                let timed_out = |_, _| unimplemented!();
                match parking_lot_core::park(
                    addr,
                    validate,
                    before_sleep,
                    timed_out,
                    DEFAULT_PARK_TOKEN,
                    None,
                ) {
                    // We were unparked normally, try acquiring the lock again
                    ParkResult::Unparked(_) => return,

                    // The validation function failed, try locking again
                    ParkResult::Invalid => (),

                    // Timeout expired
                    ParkResult::TimedOut => {
                        debug_assert!(false);
                        return;
                    }
                }
            }

            // Loop back and try locking again
            spinwait.reset();
            state = self.state.load(Ordering::Relaxed);
        }
    }

    #[cold]
    #[inline(never)]
    fn unlock_slow(&self, force_fair: bool) {
        unsafe {
            let addr = self as *const _ as usize;
            drop(parking_lot_core::unpark_all(addr, TOKEN_NORMAL));
        }
    }

    #[cold]
    #[inline(never)]
    fn bump_slow(&self) {
        self.unlock_slow(true);
        self.lock();
    }
}
