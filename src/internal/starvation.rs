//! `Starvation` is a private type used for blocking other threads in order to finish some work that
//! was unable to be performed speculatively in a finite amount of time. It assumes a fair
//! scheduler.
//!
//! `Progress` contains the logic of when to signal that a thread is starving, and waits for other
//! threads that are starving.
//!
//! http://raiith.iith.ac.in/3530/1/1709.01033.pdf
//!
//! Based on RawMutex in parking_lot.
//!
//! https://github.com/Amanieu/parking_lot

use crate::{
    internal::epoch::{QuiesceEpoch, EPOCH_CLOCK, TICK_SIZE},
    stats,
};
use core::{
    cell::Cell,
    num::{NonZeroU32, NonZeroUsize},
    sync::atomic::{self, AtomicUsize, Ordering::Relaxed},
};
use parking_lot_core::{self, FilterOp, ParkResult, ParkToken, UnparkResult, UnparkToken};
use std::thread;

const NO_STARVERS: usize = 0;
const SPIN_LIMIT: u32 = 6;
const YIELD_LIMIT: u32 = 10;

/// If a thread started a transaction this many epochs ago, the thread is considered to be starving.
///
/// Lower values result in more serialization under contention. Higher values result in more wasted
/// CPU cycles for large transactions.
const MAX_ELAPSED_EPOCHS: usize = 64 * TICK_SIZE;

static STARVATION: Starvation = Starvation {
    starved_token: AtomicUsize::new(NO_STARVERS),
};

/// `Starvation` only uses `Relaxed` memory ` ordering.
struct Starvation {
    starved_token: AtomicUsize,
}

impl Starvation {
    #[inline]
    fn starve_lock(&self, token: NonZeroUsize) {
        if self
            .starved_token
            .compare_exchange_weak(NO_STARVERS, token.get(), Relaxed, Relaxed)
            .is_err()
        {
            drop(self.starve_lock_slow(token));
        }
    }

    #[inline]
    fn starve_unlock<F: FnOnce(), G: FnMut(NonZeroUsize) -> bool, U: FnOnce(NonZeroUsize)>(
        &self,
        non_inlined_work: F,
        should_upgrade: G,
        upgrade: U,
    ) {
        // If a thread is starving, unparking is usually gonna happen.
        //
        // There's also not much to gain from a fast path as starvation handling is already in a
        // deeply slow path.
        self.starve_unlock_slow(non_inlined_work, should_upgrade, upgrade);
    }

    #[inline]
    fn wait_for_starvers(&self, token: NonZeroUsize) {
        let starved_token = self.starved_token.load(Relaxed);
        if unlikely!(starved_token != NO_STARVERS) {
            self.wait_for_starvers_slow(token)
        }
    }

    #[cold]
    #[inline(never)]
    fn starve_lock_slow(&self, token: NonZeroUsize) {
        let mut starved_token = self.starved_token.load(Relaxed);
        loop {
            if starved_token == NO_STARVERS {
                match self.starved_token.compare_exchange_weak(
                    NO_STARVERS,
                    token.get(),
                    Relaxed,
                    Relaxed,
                ) {
                    Ok(_) => return,
                    Err(x) => starved_token = x,
                }
                continue;
            }

            // Park our thread until we are woken up by an unlock
            let addr = self as *const _ as usize;
            let validate = || self.starved_token.load(Relaxed) != NO_STARVERS;
            let before_sleep = || {};
            let timed_out = |_, _| {};
            let park_token = ParkToken(token.get());
            match unsafe {
                parking_lot_core::park(addr, validate, before_sleep, timed_out, park_token, None)
            } {
                ParkResult::Unparked(UnparkToken(wakeup_token)) => {
                    debug_assert!(
                        wakeup_token != NO_STARVERS,
                        "unfairly unparking a starving thread"
                    );
                    if wakeup_token == token.get() {
                        debug_assert_eq!(
                            wakeup_token,
                            self.starved_token.load(Relaxed),
                            "improperly set the starved_token before handing off starvation \
                             control"
                        );
                        return;
                    }
                }
                ParkResult::Invalid => {}
                ParkResult::TimedOut => debug_assert!(false),
            }
            starved_token = self.starved_token.load(Relaxed);
        }
    }

    #[cold]
    #[inline(never)]
    fn wait_for_starvers_slow(&self, token: NonZeroUsize) {
        let mut starved_token = self.starved_token.load(Relaxed);
        loop {
            if starved_token == NO_STARVERS {
                return;
            }

            // Park our thread until we are woken up by an unlock
            let addr = self as *const _ as usize;
            let validate = || self.starved_token.load(Relaxed) != NO_STARVERS;
            let before_sleep = || {};
            let timed_out = |_, _| {};
            match unsafe {
                parking_lot_core::park(
                    addr,
                    validate,
                    before_sleep,
                    timed_out,
                    ParkToken(token.get()),
                    None,
                )
            } {
                ParkResult::Unparked(UnparkToken(NO_STARVERS)) => {
                    return;
                }
                ParkResult::Unparked(UnparkToken(wakeup_token)) => {
                    if wakeup_token == token.get() {
                        // this thread has been upgraded to a starver
                        debug_assert_eq!(
                            wakeup_token,
                            self.starved_token.load(Relaxed),
                            "improperly set the starved_token before handing off starvation \
                             control"
                        );
                    }
                    return;
                }
                ParkResult::Invalid => {}
                ParkResult::TimedOut => debug_assert!(false),
            }
            starved_token = self.starved_token.load(Relaxed);
        }
    }

    #[cold]
    #[inline(never)]
    fn starve_unlock_slow<F: FnOnce(), G: FnMut(NonZeroUsize) -> bool, U: FnOnce(NonZeroUsize)>(
        &self,
        non_inlined_work: F,
        mut should_upgrade: G,
        upgrade: U,
    ) {
        non_inlined_work();

        let addr = self as *const _ as usize;
        let next_starved_token = Cell::new(NO_STARVERS);
        let next_starved_token = &next_starved_token;

        // We don't know what thread we wish to unpark until we finish filtering. This means that
        // threads will sometimes be unparked without the possibility of making progress.
        let filter = move |token: ParkToken| {
            debug_assert!(token.0 != NO_STARVERS, "invalid ParkToken detected");
            let next_starved = next_starved_token.get();
            if next_starved == NO_STARVERS {
                if should_upgrade(unsafe { NonZeroUsize::new_unchecked(token.0) }) {
                    next_starved_token.set(token.0);
                }
                FilterOp::Unpark
            } else {
                FilterOp::Skip
            }
        };
        let callback = |unpark_result: UnparkResult| {
            debug_assert_ne!(self.starved_token.load(Relaxed), NO_STARVERS);
            let next_starved = next_starved_token.get();
            self.starved_token.store(next_starved, Relaxed);
            drop(NonZeroUsize::new(next_starved).map(|this| upgrade(this)));
            debug_assert!(next_starved == NO_STARVERS || unpark_result.unparked_threads > 0);
            UnparkToken(next_starved)
        };

        let result = unsafe { parking_lot_core::unpark_filter(addr, filter, callback) };
        if next_starved_token.get() != NO_STARVERS {
            stats::starvation_handoff();
        }
        stats::blocked_by_starvation(result.unparked_threads)
    }
}

#[derive(Copy, Clone)]
enum ProgressImpl {
    NotStarving {
        first_failed_epoch: Option<QuiesceEpoch>,
        backoff:            NonZeroU32,
    },
    Starving,
}

impl ProgressImpl {
    #[inline]
    fn new() -> Self {
        ProgressImpl::NotStarving {
            first_failed_epoch: None,
            backoff:            unsafe { NonZeroU32::new_unchecked(1) },
        }
    }

    #[inline]
    fn should_starve(&self) -> bool {
        match self {
            ProgressImpl::NotStarving {
                first_failed_epoch: Some(_),
                backoff,
            } => backoff.get() >= YIELD_LIMIT,
            ProgressImpl::NotStarving {
                first_failed_epoch: None,
                ..
            } => false,
            ProgressImpl::Starving => {
                debug_assert!(false);
                false
            }
        }
    }
}

pub struct Progress {
    inner: Cell<ProgressImpl>,
}

#[cfg(debug_assertions)]
impl Drop for Progress {
    fn drop(&mut self) {
        match self.inner.get() {
            ProgressImpl::NotStarving {
                first_failed_epoch: None,
                backoff,
            } if backoff.get() == 1 => {}
            _ => panic!("`Progress` dropped without having made progress"),
        }
    }
}

impl Progress {
    #[inline]
    pub fn new() -> Self {
        Progress {
            inner: Cell::new(ProgressImpl::new()),
        }
    }

    /// Called when a thread has failed either the optimistic phase of concurrency, or the
    /// pessimistic phase of concurrency.
    #[cold]
    pub fn failed_to_progress(&self, epoch: QuiesceEpoch) {
        match self.inner.get() {
            ProgressImpl::NotStarving {
                first_failed_epoch,
                backoff,
            } => {
                let epoch = first_failed_epoch.unwrap_or(epoch);
                if backoff.get() <= SPIN_LIMIT {
                    let now = EPOCH_CLOCK.now().unwrap_or_else(|| abort!());
                    if now.get().get() - epoch.get().get() >= MAX_ELAPSED_EPOCHS {
                        // long transaction detected, `spin_loop_hint` is probably a bad backoff
                        // strategy.
                        self.inner.set(ProgressImpl::NotStarving {
                            first_failed_epoch: Some(epoch),
                            backoff:            unsafe {
                                NonZeroU32::new_unchecked(SPIN_LIMIT + 1)
                            },
                        });
                        thread::yield_now();
                        return;
                    } else {
                        for _ in 0..1 << backoff.get() {
                            atomic::spin_loop_hint();
                        }
                    }
                } else if backoff.get() <= YIELD_LIMIT {
                    thread::yield_now();

                    self.inner.set(ProgressImpl::NotStarving {
                        first_failed_epoch: Some(epoch),
                        backoff:            unsafe { NonZeroU32::new_unchecked(backoff.get() + 1) },
                    });
                } else {
                    STARVATION.starve_lock(self.to_token());
                    self.inner.set(ProgressImpl::Starving)
                }
            }
            ProgressImpl::Starving => {}
        };
    }

    /// Called when a thread has finished the optimistic phase of concurrency, and is about to enter
    /// a pessimistic phase where the threads progress will be published.
    #[inline]
    pub fn wait_for_starvers(&self) {
        match self.inner.get() {
            ProgressImpl::NotStarving { .. } => STARVATION.wait_for_starvers(self.to_token()),
            ProgressImpl::Starving => {}
        };
    }

    /// Called after progress has been made.
    #[inline]
    pub fn progressed(&self) {
        match self.inner.get() {
            ProgressImpl::NotStarving { .. } => {}
            ProgressImpl::Starving => {
                STARVATION.starve_unlock(
                    || self.inner.set(ProgressImpl::new()),
                    |this| {
                        unsafe { Self::from_token(this) }
                            .inner
                            .get()
                            .should_starve()
                    },
                    |this| {
                        unsafe { Self::from_token(this) }
                            .inner
                            .set(ProgressImpl::Starving)
                    },
                );
            }
        };
    }

    #[inline]
    fn to_token(&self) -> NonZeroUsize {
        unsafe { NonZeroUsize::new_unchecked(self as *const Self as usize) }
    }

    #[inline]
    unsafe fn from_token(this: NonZeroUsize) -> &'static Self {
        &*(this.get() as *const Self)
    }
}
