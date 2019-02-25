use crate::internal::pointer::PtrExt;
use std::{
    num::NonZeroUsize,
    ptr::NonNull,
    sync::atomic::{
        AtomicUsize,
        Ordering::{self, Acquire, Relaxed, Release},
    },
};

#[inline(always)]
unsafe fn store_nonoverlapping_impl(dest_ptr: NonNull<AtomicUsize>, src: &[usize], o: Ordering) {
    // non-overlapping
    let len = src.len();
    assume!(
        len > 0,
        "`seqlock_store_nonoverlapping_impl` does not work for zero sized types"
    );
    let len = NonZeroUsize::new_unchecked(len);

    {
        let src_ptr: NonNull<_> = src.get_unchecked(0).into();
        assume!(
            src_ptr.add(len.get()) <= dest_ptr.cast() || src_ptr >= dest_ptr.add(len.get()).cast(),
            "overlapping pointers passed in to `seqlock_store_nonoverlapping_impl`"
        );
    }

    for offset in 0..len.get() {
        dest_ptr
            .add(offset)
            .as_ref()
            .store(*src.get_unchecked(offset), o);
    }
}

#[inline(always)]
unsafe fn load_nonoverlapping_impl(src_ptr: NonNull<AtomicUsize>, dest: &mut [usize], o: Ordering) {
    // non-overlapping
    let len = dest.len();
    assume!(
        len > 0,
        "`seqlock_store_nonoverlapping_impl` does not work for zero sized types"
    );
    let len = NonZeroUsize::new_unchecked(len);

    {
        let dest_ptr: NonNull<_> = dest.get_unchecked(0).into();
        assume!(
            src_ptr.add(len.get()) <= dest_ptr.cast() || src_ptr >= dest_ptr.add(len.get()).cast(),
            "overlapping pointers passed in to `seqlock_store_nonoverlapping_impl`"
        );
    }

    for offset in 0..len.get() {
        *dest.get_unchecked_mut(offset) = src_ptr.add(offset).as_ref().load(o);
    }
}

#[inline]
pub unsafe fn store_nonoverlapping_release(dest_ptr: NonNull<AtomicUsize>, src: &[usize]) {
    store_nonoverlapping_impl(dest_ptr, src, Release);
}

#[inline]
pub unsafe fn load_nonoverlapping_acquire(src_ptr: NonNull<AtomicUsize>, dest: &mut [usize]) {
    load_nonoverlapping_impl(src_ptr, dest, Acquire);
}

#[inline]
pub unsafe fn load_nonoverlapping_relaxed(src_ptr: NonNull<AtomicUsize>, dest: &mut [usize]) {
    load_nonoverlapping_impl(src_ptr, dest, Relaxed);
}
