//! ReadLog contains borrows of `TCell`'s that have been read from during a transaction.
//!
//! The only meaningful operations are filtering out writes from the ReadLog (see thread.rs), and
//! checking that the reads are still valid (validate_reads).

use crate::{
    internal::{
        alloc::FVec,
        epoch::{EpochLock, QuiesceEpoch},
        tcell_erased::TCellErased,
    },
    stats,
};
use swym_htm::HardwareTx;

const READ_CAPACITY: usize = 0;

#[derive(Debug)]
pub struct ReadLog<'tcell> {
    data: FVec<Option<&'tcell TCellErased>>,
}

impl<'tcell> ReadLog<'tcell> {
    #[inline]
    pub fn new() -> Self {
        ReadLog {
            data: FVec::with_capacity(READ_CAPACITY),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    #[inline]
    pub fn next_push_allocates(&self) -> bool {
        self.data.next_push_allocates()
    }

    #[inline]
    pub fn record(&mut self, erased: &'tcell TCellErased) {
        self.data.push(Some(erased))
    }

    #[inline]
    pub unsafe fn record_unchecked(&mut self, erased: &'tcell TCellErased) {
        self.data.push_unchecked(Some(erased))
    }

    /// After calling filter_in_place, it is unsafe to call again without calling clear first.
    /// Additionally, it is unsafe to call validate_reads_htm after calling this.
    #[inline]
    pub unsafe fn filter_in_place(&mut self, mut filter: impl FnMut(&'tcell TCellErased) -> bool) {
        for elem in self.data.iter_mut() {
            let tcell = match *elem {
                Some(tcell) => tcell,
                None => {
                    if cfg!(debug_assertions) {
                        panic!("unreachable code reached")
                    } else {
                        // we want this fast since every RW transaction runs it
                        core::hint::unreachable_unchecked()
                    }
                }
            };
            if unlikely!(!filter(tcell)) {
                *elem = None;
            }
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        stats::read_size(self.len());
        self.data.clear()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    #[inline]
    pub fn epoch_locks<'a>(&'a self) -> impl DoubleEndedIterator<Item = &'a EpochLock> {
        self.data.iter().flatten().map(|x| &x.current_epoch)
    }

    #[inline]
    pub fn validate_reads(&self, pin_epoch: QuiesceEpoch) -> bool {
        for epoch_lock in self.epoch_locks() {
            if unlikely!(!pin_epoch.read_write_valid_lockable(epoch_lock)) {
                return false;
            }
        }
        true
    }

    #[inline]
    pub fn validate_reads_htm(&self, pin_epoch: QuiesceEpoch, htx: &HardwareTx) {
        for logged_read in self.data.iter().rev() {
            let logged_read = match *logged_read {
                Some(logged_read) => logged_read,
                None => unsafe {
                    if cfg!(debug_assertions) {
                        panic!("unreachable code reached")
                    } else {
                        // we want this fast since every HTM RW transaction runs it
                        //
                        // the only way this code can be hit is if the rules for filter_in_place are
                        // not followed.
                        core::hint::unreachable_unchecked()
                    }
                },
            };
            if unlikely!(!pin_epoch.read_write_valid_lockable(&logged_read.current_epoch)) {
                htx.abort()
            }
        }
    }
}
