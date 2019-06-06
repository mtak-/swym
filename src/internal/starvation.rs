//! `Starvation` is a type used for blocking other threads in order to finish some work that was
//! unable to be performed speculatively in a finite amount of time.
//!
//! Based on RawMutex in parking_lot.
//!
//! https://github.com/Amanieu/parking_lot

use crate::stats;
use core::{
    cell::Cell,
    num::NonZeroU32,
    sync::atomic::{self, AtomicBool, Ordering::Relaxed},
};
use parking_lot_core::{self, FilterOp, ParkResult, ParkToken, UnparkResult, UnparkToken};
use std::thread;

const NO_STARVERS: UnparkToken = UnparkToken(0);
const STARVE_HANDOFF: UnparkToken = UnparkToken(1);
const STARVE_TOKEN: ParkToken = ParkToken(0);
const WAIT_TOKEN: ParkToken = ParkToken(1);
const SPIN_LIMIT: u32 = 6;
const YIELD_LIMIT: u32 = 10;

static STARVATION: Starvation = Starvation {
    state: AtomicBool::new(false),
};

pub struct Starvation {
    state: AtomicBool,
}

impl Starvation {
    #[inline]
    pub fn starve_lock(&self) {
        if self
            .state
            .compare_exchange_weak(false, true, Relaxed, Relaxed)
            .is_err()
        {
            drop(self.starve_lock_slow());
        }
    }

    #[inline]
    pub fn starve_unlock(&self) {
        // If a thread is starving, unparking is usually gonna happen.
        //
        // There's also not much to gain from a fast path as starvation handling is already in a
        // deeply slow path.
        self.starve_unlock_slow();
    }

    #[inline]
    pub fn wait_for_starvers(&self) {
        let state = self.state.load(Relaxed);
        if unlikely!(state) {
            self.wait_for_starvers_slow()
        }
    }

    #[cold]
    #[inline(never)]
    fn starve_lock_slow(&self) {
        let mut state = self.state.load(Relaxed);
        loop {
            if !state {
                match self
                    .state
                    .compare_exchange_weak(false, true, Relaxed, Relaxed)
                {
                    Ok(_) => return,
                    Err(x) => state = x,
                }
                continue;
            }

            // Park our thread until we are woken up by an unlock
            let addr = self as *const _ as usize;
            let validate = || self.state.load(Relaxed);
            let before_sleep = || {};
            let timed_out = |_, _| {};
            match unsafe {
                parking_lot_core::park(addr, validate, before_sleep, timed_out, STARVE_TOKEN, None)
            } {
                ParkResult::Unparked(STARVE_HANDOFF) => return,
                ParkResult::Unparked(_) => {
                    if cfg!(debug_assertions) {
                        panic!("unfairly unparking a starving thread")
                    }
                }
                ParkResult::Invalid => {}
                ParkResult::TimedOut => {
                    debug_assert!(false);
                    return;
                }
            }
            state = self.state.load(Relaxed);
        }
    }

    #[cold]
    #[inline(never)]
    fn wait_for_starvers_slow(&self) {
        let mut state = self.state.load(Relaxed);
        loop {
            if !state {
                return;
            }

            // Park our thread until we are woken up by an unlock
            let addr = self as *const _ as usize;
            let validate = || self.state.load(Relaxed);
            let before_sleep = || {};
            let timed_out = |_, _| {};
            match unsafe {
                parking_lot_core::park(addr, validate, before_sleep, timed_out, WAIT_TOKEN, None)
            } {
                ParkResult::Unparked(NO_STARVERS) => return,
                ParkResult::Unparked(_) => {}
                ParkResult::Invalid => {}
                ParkResult::TimedOut => {
                    debug_assert!(false);
                    return;
                }
            }
            state = self.state.load(Relaxed);
        }
    }

    #[cold]
    #[inline(never)]
    fn starve_unlock_slow(&self) {
        let addr = self as *const _ as usize;
        let starvers = Cell::new(false);
        let starvers = &starvers;
        let filter = |token| {
            if starvers.get() {
                return FilterOp::Stop;
            }
            starvers.set(token == STARVE_TOKEN);
            FilterOp::Unpark
        };
        let callback = |unpark_result: UnparkResult| {
            if starvers.get() {
                debug_assert!(unpark_result.unparked_threads > 0);
                STARVE_HANDOFF
            } else {
                self.state.store(false, Relaxed);
                NO_STARVERS
            }
        };

        let result = unsafe { parking_lot_core::unpark_filter(addr, filter, callback) };
        stats::blocked_by_starvation(result.unparked_threads)
    }
}

#[derive(Copy, Clone)]
enum ProgressImpl {
    NotStarving(NonZeroU32),
    Starving,
}

pub struct Progress {
    inner: Cell<ProgressImpl>,
}

#[cfg(debug_assertions)]
impl Drop for Progress {
    fn drop(&mut self) {
        match self.inner.get() {
            ProgressImpl::NotStarving(_) => {}
            ProgressImpl::Starving => panic!("Progress dropped while Starving"),
        }
    }
}

impl Progress {
    #[inline]
    pub fn new() -> Self {
        Progress {
            inner: Cell::new(ProgressImpl::NotStarving(NonZeroU32::new(1).unwrap())),
        }
    }

    #[cold]
    pub fn failed_to_progress(&self) {
        match self.inner.get() {
            ProgressImpl::NotStarving(count) => {
                if count.get() <= SPIN_LIMIT {
                    for _ in 0..1 << count.get() {
                        atomic::spin_loop_hint();
                    }
                } else {
                    thread::yield_now();
                }

                if count.get() <= YIELD_LIMIT {
                    self.inner.set(ProgressImpl::NotStarving(unsafe {
                        NonZeroU32::new_unchecked(count.get() + 1)
                    }));
                } else {
                    STARVATION.starve_lock();
                    self.inner.set(ProgressImpl::Starving)
                }
            }
            ProgressImpl::Starving => {}
        };
    }

    #[inline]
    pub fn wait_for_starvers(&self) {
        match self.inner.get() {
            ProgressImpl::NotStarving(_) => STARVATION.wait_for_starvers(),
            ProgressImpl::Starving => {}
        };
    }

    #[inline]
    pub fn progressed(&self) {
        match self.inner.get() {
            ProgressImpl::NotStarving(_) => {}
            ProgressImpl::Starving => {
                self.inner
                    .set(ProgressImpl::NotStarving(NonZeroU32::new(1).unwrap()));
                STARVATION.starve_unlock();
            }
        };
    }
}
