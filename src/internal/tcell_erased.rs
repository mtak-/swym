use crate::internal::{epoch::EpochLock, pointer::PtrExt, usize_aligned::UsizeAligned};
use std::{
    mem::ManuallyDrop,
    ptr::{self, NonNull},
    sync::atomic::Ordering::Acquire,
};

// A "dynamic" type that can have references to instances of it put into a collection and still have
// meaning. The type the TCell contains is not recoverable, but it's ok to load from, or store to
// it, as long as you know the type (through some other means) or the len respectively.
//
// This relies heavily on repr() and the layout of TCell. In order to handle overaligned types
// (align_of::<T>() > align_of::<usize>()) TCellErased is stored after UsizeAligned<T> in the TCell.
// A nice side benefit is that reads always read T first then the EpochLock, so this layout is
// likely better for the cache.
#[repr(transparent)]
#[derive(Debug)]
pub struct TCellErased {
    pub current_epoch: EpochLock,
}

impl TCellErased {
    #[inline]
    pub const fn new() -> TCellErased {
        TCellErased {
            current_epoch: EpochLock::first(),
        }
    }

    #[inline]
    pub unsafe fn optimistic_read_acquire<T>(&self) -> ManuallyDrop<T> {
        let result = self.optimistic_read_relaxed();
        std::sync::atomic::fence(Acquire);
        result
    }

    #[inline]
    pub unsafe fn optimistic_read_relaxed<T>(&self) -> ManuallyDrop<T> {
        ptr::read_volatile(
            NonNull::from(self)
                .cast::<usize>()
                // UsizeAligned<T> is immediately _before_ TCellErased in memory
                .sub(UsizeAligned::<T>::len().get())
                .cast()
                .assume_aligned()
                .as_ptr(),
        )
    }
}
