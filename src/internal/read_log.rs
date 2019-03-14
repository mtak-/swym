use crate::internal::{
    alloc::{fvec::Iter, FVec},
    epoch::QuiesceEpoch,
    stats,
    tcell_erased::TCellErased,
};
use std::{num::NonZeroUsize, sync::atomic::Ordering::Relaxed};

const READ_CAPACITY: usize = 1024;

#[derive(Debug)]
pub struct ReadLog<'tcell> {
    data: FVec<ReadEntry<'tcell>>,
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
        self.data.push(ReadEntry::new(erased))
    }

    #[inline]
    pub unsafe fn push_unchecked(&mut self, erased: &'tcell TCellErased) {
        self.data.push_unchecked(ReadEntry::new(erased))
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
    pub unsafe fn get_unchecked(&self, i: usize) -> &ReadEntry<'tcell> {
        self.data.get_unchecked(i)
    }

    #[inline]
    pub unsafe fn swap_erase_unchecked(&mut self, i: usize) {
        self.data.swap_erase_unchecked(i)
    }

    #[inline]
    fn iter(&self) -> Iter<'_, ReadEntry<'tcell>> {
        self.data.iter()
    }

    #[inline]
    pub fn validate_reads(&self, pin_epoch: QuiesceEpoch) -> bool {
        for logged_read in self.iter() {
            if unlikely!(
                !pin_epoch.read_write_valid_lockable(&logged_read.src.current_epoch, Relaxed)
            ) {
                return false;
            }
        }
        true
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ReadEntry<'tcell> {
    pub src: &'tcell TCellErased,
}

impl<'tcell> ReadEntry<'tcell> {
    #[inline]
    pub const fn new(src: &'tcell TCellErased) -> Self {
        ReadEntry { src }
    }
}
