use std::{
    mem::{self, MaybeUninit},
    num::NonZeroUsize,
    ops::{Deref, DerefMut},
};

#[cfg(target_pointer_width = "64")]
#[repr(align(8))]
#[derive(Copy, Clone, Debug)]
pub struct UsizeAligned<T: ?Sized>(T);

#[cfg(target_pointer_width = "32")]
#[repr(align(4))]
#[derive(Copy, Clone, Debug)]
pub struct UsizeAligned<T: ?Sized>(T);

impl<T> UsizeAligned<T> {
    #[inline]
    pub const fn new(value: T) -> Self {
        UsizeAligned(value)
    }

    #[inline]
    pub fn into_inner(self) -> T {
        self.0
    }

    #[inline]
    pub fn len() -> NonZeroUsize {
        NonZeroUsize::new(mem::size_of::<Self>() / mem::size_of::<usize>())
            .expect("can't call len on zero sized UsizeAligned")
    }
}

impl<T> UsizeAligned<MaybeUninit<T>> {
    #[inline]
    pub unsafe fn as_mut_slice(&mut self) -> &mut [usize] {
        std::slice::from_raw_parts_mut(self.as_mut_ptr() as _, UsizeAligned::<T>::len().get())
    }
}

impl<T: ?Sized> Deref for UsizeAligned<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: ?Sized> DerefMut for UsizeAligned<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

#[repr(packed)]
struct Unaligned<T>(T);

pub struct ForcedUsizeAligned<T>(UsizeAligned<Unaligned<T>>);

impl<T> ForcedUsizeAligned<T> {
    #[inline]
    pub const fn new(value: T) -> Self {
        ForcedUsizeAligned(UsizeAligned(Unaligned(value)))
    }
}
