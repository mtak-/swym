use core::ops::{Deref, DerefMut};

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

#[cfg(test)]
mod test {
    use super::*;
    use core::mem;

    #[test]
    fn alignment() {
        let x = 0i8;
        let x_usize_aligned = UsizeAligned::new(x);
        assert_eq!(
            mem::align_of_val(&x_usize_aligned),
            mem::align_of::<usize>()
        );

        let x = ();
        let x_usize_aligned = UsizeAligned::new(x);
        assert_eq!(
            mem::align_of_val(&x_usize_aligned),
            mem::align_of::<usize>()
        );

        #[repr(align(1024))]
        struct OverAligned;

        let x = OverAligned;
        let x_usize_aligned = UsizeAligned::new(x);
        assert!(mem::align_of_val(&x_usize_aligned) > mem::align_of::<usize>());

        let x = OverAligned;
        let x_force_usize_aligned = ForcedUsizeAligned::new(x);
        assert_eq!(
            mem::align_of_val(&x_force_usize_aligned),
            mem::align_of::<usize>()
        );
    }
}
