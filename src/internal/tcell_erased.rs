use crate::internal::{
    epoch::EpochLock, pointer::PtrExt, seq_storage, usize_aligned::UsizeAligned,
};
use std::{
    mem::{self, ManuallyDrop},
    num::NonZeroUsize,
    ptr::NonNull,
    sync::atomic::{
        AtomicUsize,
        Ordering::{self, Acquire, Relaxed},
    },
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
    unsafe fn read_usize<T>(&self, ordering: Ordering) -> ManuallyDrop<T> {
        debug_assert!(
            mem::size_of::<T>() <= mem::size_of::<usize>(),
            "attempt to read a > sizeof(usize) type as a usized type"
        );
        let ptr = self.as_atomic_ptr(NonZeroUsize::new_unchecked(1));
        let a: UsizeAligned<ManuallyDrop<T>> = mem::transmute_copy(&ptr.as_ref().load(ordering));
        a.into_inner()
    }

    #[inline]
    pub unsafe fn read_inconsistent<T>(&self) -> ManuallyDrop<T> {
        self.read_usize::<T>(Relaxed)
    }

    #[inline]
    pub unsafe fn read_acquire<T>(&self) -> ManuallyDrop<T> {
        // optimizes much better than slices
        if mem::size_of::<T>() <= mem::size_of::<usize>() {
            self.read_usize::<T>(Acquire)
        } else {
            let mut a: UsizeAligned<ManuallyDrop<T>> = mem::uninitialized();
            self.load_acquire(a.as_mut());
            a.into_inner()
        }
    }

    #[inline]
    pub unsafe fn read_relaxed<T>(&self) -> ManuallyDrop<T> {
        if mem::size_of::<T>() <= mem::size_of::<usize>() {
            self.read_inconsistent()
        } else {
            let mut a: UsizeAligned<ManuallyDrop<T>> = mem::uninitialized();
            self.load_relaxed(a.as_mut());
            a.into_inner()
        }
    }

    #[inline]
    unsafe fn load_acquire(&self, dest: &mut [usize]) {
        let len = dest.len();
        assume!(len > 0, "`load_acquire` does not work for zero sized types");
        let len = NonZeroUsize::new_unchecked(len);
        seq_storage::load_nonoverlapping_acquire(self.as_atomic_ptr(len), dest)
    }

    #[inline]
    unsafe fn load_relaxed(&self, dest: &mut [usize]) {
        let len = dest.len();
        assume!(len > 0, "`load_relaxed` does not work for zero sized types");
        let len = NonZeroUsize::new_unchecked(len);
        seq_storage::load_nonoverlapping_relaxed(self.as_atomic_ptr(len), dest)
    }

    #[inline]
    pub unsafe fn store_release(&self, src: &[usize]) {
        let len = src.len();
        assume!(
            len > 0,
            "`store_release` does not work for zero sized types"
        );
        let len = NonZeroUsize::new_unchecked(len);
        seq_storage::store_nonoverlapping_release(self.as_atomic_ptr(len), src)
    }

    // UsizeAligned<T> is immediately _before_ TCellErased in memory
    #[inline]
    pub unsafe fn as_atomic_ptr(&self, len: NonZeroUsize) -> NonNull<AtomicUsize> {
        assume!(len.get() > 0);
        NonNull::from(self).cast().sub(len.get())
    }
}