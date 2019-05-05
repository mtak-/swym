use crate::internal::usize_aligned::ForcedUsizeAligned;
use core::{
    mem::{self, ManuallyDrop},
    ptr,
};

/// Trash that has been queued for dropping by the GC.
pub struct Queued<T: 'static + Send> {
    /// Queueds are stored in a DynVec which does not support > `usize` alignment.
    to_drop: ForcedUsizeAligned<ManuallyDrop<T>>,
}

impl<T: 'static + Send> Queued<T> {
    #[inline]
    pub fn new(to_drop: ManuallyDrop<T>) -> Self {
        debug_assert!(
            mem::needs_drop::<T>(),
            "attempt to queue garbage that doesn't need dropping"
        );
        Queued {
            to_drop: ForcedUsizeAligned::new(to_drop),
        }
    }
}

/// An in place FnOnce
pub trait FnOnceish {
    /// Unsafe to call more than once
    unsafe fn call(&mut self);
}

impl<T: 'static + Send> FnOnceish for Queued<T> {
    #[inline]
    unsafe fn call(&mut self) {
        // if T's actual alignment is greater than the alignment of a usize,
        // then we have to read the value out first before dropping.
        if mem::align_of::<T>() > mem::align_of::<usize>() {
            drop(ptr::read_unaligned::<T>(
                &mut self.to_drop as *mut _ as *mut T,
            ))
        } else {
            ptr::drop_in_place::<T>(&mut self.to_drop as *mut _ as *mut T)
        }
    }
}
