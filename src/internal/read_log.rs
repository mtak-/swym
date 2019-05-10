//! ReadLog contains borrows of `TCell`'s that have been read from during a transaction.
//!
//! The only meaningful operations are filtering out writes from the ReadLog (see thread.rs), and
//! checking that the reads are still valid (validate_reads).

use crate::{
    internal::{alloc::FVec, epoch::QuiesceEpoch, tcell_erased::TCellErased},
    stats,
};
use core::ptr;
use swym_htm::HardwareTx;

const READ_CAPACITY: usize = 1024;

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
    fn iter<'a>(&'a self) -> impl DoubleEndedIterator<Item = &'a TCellErased> {
        self.data.iter().flat_map(|v| *v)
    }

    #[inline]
    pub fn validate_reads(&self, pin_epoch: QuiesceEpoch) -> bool {
        for logged_read in self.iter() {
            if unlikely!(!pin_epoch.read_write_valid_lockable(&logged_read.current_epoch)) {
                return false;
            }
        }
        true
    }

    #[inline]
    pub fn validate_reads_htm(&self, pin_epoch: QuiesceEpoch, htm: &HardwareTx) {
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
                htm.abort()
            }
        }
    }

    #[inline]
    pub fn try_clear_unpark_bits(&self, pin_epoch: QuiesceEpoch) -> bool {
        for logged_read in self.iter() {
            if !logged_read.current_epoch.try_clear_unpark_bit(pin_epoch) {
                // TODO: don't think this is correct
                self.set_unpark_bits_until(logged_read);
                return false;
            }
        }
        true
    }

    #[inline]
    fn set_unpark_bits_until(&self, end: &TCellErased) {
        for logged_read in self.iter().take_while(|read| !ptr::eq(*read, end)) {
            logged_read.current_epoch.set_unpark_bit()
        }
    }

    #[inline]
    pub fn set_unpark_bits(&self) {
        for logged_read in self.iter() {
            logged_read.current_epoch.set_unpark_bit()
        }
    }
}
