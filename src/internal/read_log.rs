use crate::internal::{alloc::FVec, epoch::QuiesceEpoch, stats, tcell_erased::TCellErased};
use std::{num::NonZeroUsize, sync::atomic::Ordering::Relaxed};

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
    pub fn clear(&mut self) {
        stats::read_size(self.len());
        self.data.clear()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    #[inline]
    pub unsafe fn get_unchecked(&self, i: usize) -> &TCellErased {
        self.data.get_unchecked(i)
    }

    #[inline]
    pub unsafe fn swap_erase_unchecked(&mut self, i: usize) {
        self.data.swap_erase_unchecked(i)
    }

    #[inline]
    fn iter<'a>(&'a self) -> impl Iterator<Item = &'a TCellErased> {
        self.data.iter().map(|v| *v)
    }

    #[inline]
    pub fn validate_reads(&self, pin_epoch: QuiesceEpoch) -> bool {
        for logged_read in self.iter() {
            if unlikely!(!pin_epoch.read_write_valid_lockable(&logged_read.current_epoch, Relaxed))
            {
                return false;
            }
        }
        true
    }
}
