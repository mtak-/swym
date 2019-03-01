use crate::internal::{
    alloc::{fvec::Iter, FVec},
    epoch::QuiesceEpoch,
    stats,
    tcell_erased::TCellErased,
};
use std::{num::NonZeroUsize, ptr::NonNull, sync::atomic::Ordering::Relaxed};

const READ_SIZE: NonZeroUsize = unsafe { NonZeroUsize::new_unchecked(1024) };

#[derive(Debug)]
pub struct ReadLog {
    data: FVec<ReadEntry>,
}

impl ReadLog {
    #[inline]
    pub fn new() -> Self {
        ReadLog {
            data: FVec::with_capacity(READ_SIZE),
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
    pub fn push(&mut self, erased: &TCellErased) {
        self.data.push(ReadEntry::new(erased))
    }

    #[inline]
    pub unsafe fn push_unchecked(&mut self, erased: &TCellErased) {
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
    pub unsafe fn get_unchecked(&self, i: usize) -> &ReadEntry {
        self.data.get_unchecked(i)
    }

    #[inline]
    pub unsafe fn swap_erase_unchecked(&mut self, i: usize) {
        self.data.swap_erase_unchecked(i)
    }

    #[inline]
    pub fn iter(&self) -> Iter<'_, ReadEntry> {
        self.data.iter()
    }

    #[inline]
    pub unsafe fn validate_reads(&self, pin_epoch: QuiesceEpoch) -> bool {
        for logged_read in self.iter() {
            if unlikely!(!pin_epoch
                .read_write_valid_lockable(&logged_read.src.as_ref().current_epoch, Relaxed))
            {
                return false;
            }
        }
        true
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ReadEntry {
    pub src: NonNull<TCellErased>,
}

impl ReadEntry {
    #[inline]
    pub const fn new(src: &TCellErased) -> Self {
        unsafe {
            ReadEntry {
                src: NonNull::new_unchecked(src as *const _ as _),
            }
        }
    }
}
