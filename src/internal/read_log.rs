//! ReadLog contains borrows of `TCell`'s that have been read from during a transaction.
//!
//! The only meaningful operations are filtering out writes from the ReadLog (see thread.rs), and
//! checking that the reads are still valid (validate_reads).

use crate::internal::{alloc::FVec, epoch::QuiesceEpoch, stats, tcell_erased::TCellErased};
use std::num::NonZeroUsize;

const READ_CAPACITY: usize = 1024;

#[derive(Debug)]
pub struct ReadLog<'tcell> {
    data: FVec<&'tcell TCellErased>,
}

impl<'tcell> ReadLog<'tcell> {
    #[inline]
    pub fn new() -> Self {
        ReadLog {
            data: FVec::with_capacity(NonZeroUsize::new(READ_CAPACITY).unwrap()),
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
    pub fn push(&mut self, erased: &'tcell TCellErased) {
        self.data.push(erased)
    }

    #[inline]
    pub unsafe fn push_unchecked(&mut self, erased: &'tcell TCellErased) {
        self.data.push_unchecked(erased)
    }

    #[inline]
    pub fn filter_in_place(&mut self, filter: impl FnMut(&mut &'tcell TCellErased) -> bool) {
        self.data.filter_in_place(filter)
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
    fn iter<'a>(&'a self) -> impl Iterator<Item = &'a TCellErased> {
        self.data.iter().map(|v| *v)
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
}
