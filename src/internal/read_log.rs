//! ReadLog contains borrows of `TCell`'s that have been read from during a transaction.
//!
//! The only meaningful operations are filtering out writes from the ReadLog (see thread.rs), and
//! checking that the reads are still valid (validate_reads).

use crate::internal::{alloc::FVec, epoch::QuiesceEpoch, stats, tcell_erased::TCellErased};
use std::num::NonZeroUsize;

const READ_CAPACITY: usize = 1024;

#[derive(Debug)]
pub struct ReadLog<'tcell> {
    data: FVec<Option<&'tcell TCellErased>>,
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
    pub fn record(&mut self, erased: &'tcell TCellErased) {
        self.data.push(Some(erased))
    }

    #[inline]
    pub unsafe fn record_unchecked(&mut self, erased: &'tcell TCellErased) {
        self.data.push_unchecked(Some(erased))
    }

    /// After calling filter_in_place, it is unsafe to call again without calling clear first
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
                        std::hint::unreachable_unchecked()
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
    fn iter<'a>(&'a self) -> impl Iterator<Item = &'a TCellErased> {
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
}
