use crate::internal::{
    pointer::{PtrExt, PtrMutExt},
    usize_aligned::ForcedUsizeAligned,
};
use std::mem::{self, ManuallyDrop};

pub struct Queued<T: 'static + Send> {
    to_drop: ForcedUsizeAligned<ManuallyDrop<T>>,
}

impl<T: 'static + Send> Queued<T> {
    #[inline]
    pub unsafe fn new(to_drop: ManuallyDrop<T>) -> Self {
        debug_assert!(
            mem::needs_drop::<T>(),
            "attempt to queue garbage that doesn't need dropping"
        );
        Queued {
            to_drop: ForcedUsizeAligned::new(to_drop),
        }
    }
}

pub unsafe trait FnOnceish {
    unsafe fn call(&mut self);
}

unsafe impl<T: 'static + Send> FnOnceish for Queued<T> {
    #[inline]
    unsafe fn call(&mut self) {
        if mem::align_of::<T>() > mem::align_of::<usize>() {
            drop(PtrExt::read_as::<T>(&mut self.to_drop as *mut _))
        } else {
            PtrMutExt::drop_in_place_aligned(&mut self.to_drop as *mut _ as *mut T)
        }
    }
}
