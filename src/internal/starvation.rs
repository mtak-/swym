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
    num::NonZeroU32,
    sync::atomic::{self, AtomicUsize, Ordering::Relaxed},
};
use parking_lot_core::{self, FilterOp, ParkResult, ParkToken, UnparkResult, UnparkToken};
use std::thread;

const NO_STARVERS_EPOCH: usize = 0;
const SPIN_LIMIT: u32 = 6;
const YIELD_LIMIT: u32 = 10;

/// If a thread started a transaction this many epochs ago, the thread is considered to be starving.
///
/// Lower values result in more serialization under contention. Higher values result in more wasted
/// CPU cycles for large transactions.
const MAX_ELAPSED_EPOCHS: usize = 64 * TICK_SIZE;

static STARVATION: Starvation = Starvation {
    blocked_epoch: AtomicUsize::new(NO_STARVERS_EPOCH),
};

/// `Starvation` only uses `Relaxed` memory ` ordering.
struct Starvation {
    blocked_epoch: AtomicUsize,
}

impl Starvation {
    #[inline]
    fn starve_lock(&self, epoch: QuiesceEpoch) {
        if self
            .blocked_epoch
            .compare_exchange_weak(NO_STARVERS_EPOCH, epoch.get().get(), Relaxed, Relaxed)
            .is_err()
        {
            drop(self.starve_lock_slow(epoch));
        }
    }

    #[inline]
    fn starve_unlock<F: FnOnce()>(&self, non_inlined_work: F) {
        // If a thread is starving, unparking is usually gonna happen.
        //
        // There's also not much to gain from a fast path as starvation handling is already in a
        // deeply slow path.
        self.starve_unlock_slow(non_inlined_work);
    }

    #[inline]
    fn wait_for_starvers<F: FnMut() -> Option<QuiesceEpoch>>(&self, should_upgrade: F) {
        let blocked_epoch = self.blocked_epoch.load(Relaxed);
        if unlikely!(blocked_epoch != NO_STARVERS_EPOCH) {
            self.wait_for_starvers_slow(should_upgrade)
        }
    }

    #[cold]
    #[inline(never)]
    fn starve_lock_slow(&self, epoch: QuiesceEpoch) {
        let mut blocked_epoch = self.blocked_epoch.load(Relaxed);
        loop {
            if blocked_epoch == NO_STARVERS_EPOCH {
                match self.blocked_epoch.compare_exchange_weak(
                    NO_STARVERS_EPOCH,
                    epoch.get().get(),
                    Relaxed,
                    Relaxed,
                ) {
                    Ok(_) => return,
                    Err(x) => blocked_epoch = x,
                }
                continue;
            }

            // Park our thread until we are woken up by an unlock
            let addr = self as *const _ as usize;
            let validate = || self.blocked_epoch.load(Relaxed) != NO_STARVERS_EPOCH;
            let before_sleep = || {};
            let timed_out = |_, _| {};
            let park_token = ParkToken(epoch.get().get());
            match unsafe {
                parking_lot_core::park(addr, validate, before_sleep, timed_out, park_token, None)
            } {
                ParkResult::Unparked(UnparkToken(wakeup_epoch)) => {
                    debug_assert!(
                        wakeup_epoch != NO_STARVERS_EPOCH,
                        "unfairly unparking a starving thread"
                    );
                    if wakeup_epoch == epoch.get().get() {
                        debug_assert_eq!(
                            wakeup_epoch,
                            self.blocked_epoch.load(Relaxed),
                            "improperly set the blocked_epoch before handing off starvation \
                             control"
                        );
                        return;
                    }
                }
                ParkResult::Invalid => {}
                ParkResult::TimedOut => debug_assert!(false),
            }
            blocked_epoch = self.blocked_epoch.load(Relaxed);
        }
    }

    #[cold]
    #[inline(never)]
    fn wait_for_starvers_slow<F: FnMut() -> Option<QuiesceEpoch>>(&self, mut should_upgrade: F) {
        let mut blocked_epoch = self.blocked_epoch.load(Relaxed);
        loop {
            if blocked_epoch == NO_STARVERS_EPOCH {
                return;
            }

            // Park our thread until we are woken up by an unlock
            let addr = self as *const _ as usize;
            let validate = || self.blocked_epoch.load(Relaxed) != NO_STARVERS_EPOCH;
            let before_sleep = || {};
            let timed_out = |_, _| {};
            if let Some(epoch) = should_upgrade() {
                return self.starve_lock_slow(epoch);
            }
            match unsafe {
                parking_lot_core::park(
                    addr,
                    validate,
                    before_sleep,
                    timed_out,
                    ParkToken(NO_STARVERS_EPOCH),
                    None,
                )
            } {
                ParkResult::Unparked(UnparkToken(NO_STARVERS_EPOCH)) => {
                    return;
                }
                ParkResult::Unparked(_) => {}
                ParkResult::Invalid => {}
                ParkResult::TimedOut => debug_assert!(false),
            }
            blocked_epoch = self.blocked_epoch.load(Relaxed);
        }
    }

    #[cold]
    #[inline(never)]
    fn starve_unlock_slow<F: FnOnce()>(&self, non_inlined_work: F) {
        non_inlined_work();

        let addr = self as *const _ as usize;
        let starve_epoch = Cell::new(NO_STARVERS_EPOCH);
        let starve_epoch = &starve_epoch;

        // We don't know what thread we wish to unpark until we finish filtering. This means that
        // threads will sometimes be unparked without the possibility of making progress.
        let filter = |token: ParkToken| {
            let epoch = starve_epoch.get();
            if epoch == NO_STARVERS_EPOCH {
                starve_epoch.set(token.0);
                FilterOp::Unpark
            } else if token.0 == NO_STARVERS_EPOCH {
                FilterOp::Skip
            } else if token.0 < epoch {
                starve_epoch.set(token.0);
                FilterOp::Unpark
            } else {
                FilterOp::Skip
            }
        };
        let callback = |unpark_result: UnparkResult| {
            debug_assert_ne!(self.blocked_epoch.load(Relaxed), NO_STARVERS_EPOCH);
            let epoch = starve_epoch.get();
            self.blocked_epoch.store(epoch, Relaxed);
            debug_assert!(epoch == NO_STARVERS_EPOCH || unpark_result.unparked_threads > 0);
            UnparkToken(epoch)
        };

        let result = unsafe { parking_lot_core::unpark_filter(addr, filter, callback) };
        if starve_epoch.get() != NO_STARVERS_EPOCH {
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
            } if backoff.get() == 0 => {}
            _ => panic!("`Progress` dropped without having made progress"),
        }
    }
}

impl Progress {
    #[inline]
    pub fn new() -> Self {
        Progress {
            inner: Cell::new(ProgressImpl::NotStarving {
                first_failed_epoch: None,
                backoff:            NonZeroU32::new(1).unwrap(),
            }),
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
                } else {
                    thread::yield_now();
                }

                if backoff.get() <= YIELD_LIMIT {
                    self.inner.set(ProgressImpl::NotStarving {
                        first_failed_epoch: Some(epoch),
                        backoff:            unsafe { NonZeroU32::new_unchecked(backoff.get() + 1) },
                    });
                } else {
                    STARVATION.starve_lock(epoch);
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
            ProgressImpl::NotStarving { .. } => {
                STARVATION.wait_for_starvers(|| self.should_upgrade())
            }
            ProgressImpl::Starving => {}
        };
    }

    /// Called after progress has been made.
    #[inline]
    pub fn progressed(&self) {
        match self.inner.get() {
            ProgressImpl::NotStarving { .. } => {}
            ProgressImpl::Starving => {
                STARVATION.starve_unlock(|| {
                    self.inner.set(ProgressImpl::NotStarving {
                        first_failed_epoch: None,
                        backoff:            NonZeroU32::new(1).unwrap(),
                    })
                });
            }
        };
    }

    #[inline]
    fn should_upgrade(&self) -> Option<QuiesceEpoch> {
        None
        // match self.inner.get() {
        //     ProgressImpl::NotStarving {
        //         first_failed_epoch: Some(first_failed_epoch),
        //         backoff,
        //     } => {
        //         if backoff.get() <= SPIN_LIMIT {
        //             let now = EPOCH_CLOCK.now().unwrap_or_else(|| abort!());
        //             if now.get().get() - first_failed_epoch.get().get() >= MAX_ELAPSED_EPOCHS {
        //                 self.inner.set(ProgressImpl::NotStarving {
        //                     first_failed_epoch: Some(first_failed_epoch),
        //                     backoff:            unsafe {
        //                         NonZeroU32::new_unchecked(SPIN_LIMIT + 1)
        //                     },
        //                 });
        //                 thread::yield_now();
        //                 return None;
        //             } else {
        //                 for _ in 0..1 << backoff.get() {
        //                     atomic::spin_loop_hint();
        //                 }
        //             }
        //         } else {
        //             thread::yield_now();
        //         }

        //         if backoff.get() <= YIELD_LIMIT {
        //             self.inner.set(ProgressImpl::NotStarving {
        //                 first_failed_epoch: Some(first_failed_epoch),
        //                 backoff:            unsafe { NonZeroU32::new_unchecked(backoff.get() + 1)
        // },             });
        //         } else {
        //             epoch = Some(first_failed_epoch);
        //             self.inner.set(ProgressImpl::Starving);
        //         }
        //     }
        //     _ => {}
        // };
        // epoch
    }
}
