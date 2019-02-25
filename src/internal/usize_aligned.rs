use std::{
    mem,
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
    pub const unsafe fn len() -> NonZeroUsize {
        NonZeroUsize::new_unchecked(mem::size_of::<Self>() / mem::size_of::<usize>())
    }

    #[inline]
    pub unsafe fn as_mut(&mut self) -> &mut [usize] {
        std::slice::from_raw_parts_mut(self as *mut _ as _, UsizeAligned::<T>::len().get())
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
