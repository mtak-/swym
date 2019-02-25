use std::{mem, ptr::NonNull};

macro_rules! default_aligned_structs {
    ($name:ident!{$($params:tt),*}) => {
        $name!{
            $($params,)*
            A1:     1,
            A2:     2,
            A4:     4,
            A8:     8,
            A16:    16,
            A64:    64,
            A128:   128,
            A256:   256,
            A512:   512,
            A1024:  1024,
            A2048:  2048,
            A4096:  4096,
            A8192:  8192,
            A16384: 16384,
            A32768: 32768,
            A65536: 65536
        }
    };
}

macro_rules! aligned_struct {
    ($name:ident: $align:expr) => {
        #[repr(align($align))]
        struct $name {
            _unused: [u8; $align],
        }
    };
    ($($name:ident: $align:expr),*) => {
        $(aligned_struct!{$name: $align})*
    };
}

default_aligned_structs! {aligned_struct!{}}

macro_rules! read_as_gt_helper {
    ($self:ident, $u:ident, $($name:ident: $align:expr),*) => {
        $(if mem::align_of::<Self::Pointee>() == mem::align_of::<$name>() {
            debug_assert_eq!(mem::size_of::<U>() % mem::size_of::<$name>(), 0);
            ($self.as_ptr() as *const $name).copy_to_nonoverlapping(
                &mut $u as *mut U as *mut $name,
                mem::size_of::<U>() / mem::size_of::<$name>(),
            );
        } else)* {
            panic!("alignment not supported please file a bug report");
        }
    };
}

macro_rules! write_as_gt_helper {
    ($self:ident, $value:ident, $($name:ident: $align:expr),*) => {
        $(if mem::align_of::<Self::Pointee>() == mem::align_of::<$name>() {
            debug_assert_eq!(mem::size_of::<U>() % mem::size_of::<$name>(), 0);
            (&$value as *const U as *const $name).copy_to_nonoverlapping(
                $self.as_mut_ptr() as *mut $name,
                mem::size_of::<U>() / mem::size_of::<$name>(),
            );
            mem::forget($value)
        } else)* {
            panic!("alignment not supported please file a bug report");
        }
    };
}

pub trait PtrExt: Copy {
    type Pointee: Sized;

    /// Converts self into a raw pointer.
    fn as_ptr(self) -> *const Self::Pointee;

    /// Calculates a new pointer from an offset via addition c-style.
    unsafe fn add(self, value: usize) -> Self;

    /// Calculates a new pointer from an offset via subtraction c-style.
    unsafe fn sub(self, value: usize) -> Self;

    /// Calculates a new pointer aligned to `Self::Pointee`. If the pointer is already aligned, it
    /// is returned unchanged.
    unsafe fn align_next(self) -> Self;

    #[inline]
    unsafe fn assume_aligned(self) -> Self {
        assume!(self.is_aligned(), "expected aligned pointer");
        self
    }

    /// Calculates the offset, in *bytes*, that needs to be applied to the pointer in order to make
    /// it aligned to align.
    #[inline]
    unsafe fn align_offset_bytes(self, align: usize) -> usize {
        (self.as_ptr() as *const u8).align_offset(align)
    }

    /// Checks the alignment of the pointer
    #[inline]
    fn is_aligned(self) -> bool {
        (self.as_ptr() as usize) % mem::align_of::<Self::Pointee>() == 0
    }

    /// Reads the value from self without moving it. This leaves the memory in self unchanged.
    #[inline]
    unsafe fn read_aligned(self) -> Self::Pointee {
        self.assume_aligned().as_ptr().read()
    }

    /// Reads the value from self without moving it. This leaves the memory in self unchanged.
    ///
    /// Unlike `read_aligned`, the pointer may be unaligned.
    #[inline]
    unsafe fn read_unaligned(self) -> Self::Pointee {
        self.as_ptr().read_unaligned()
    }

    /// Reads the value from self without moving it. This leaves the memory in self unchanged.
    ///
    /// Requires that the alignment of `self.as_ptr()` is `align_of::<U>()` or greater.
    #[inline]
    unsafe fn read_as_aligned<U>(self) -> U {
        let u_ptr = self.as_ptr() as *const U;
        u_ptr.assume_aligned().read()
    }

    /// Reads the value from self without moving it. This leaves the memory in self unchanged.
    ///
    /// This makes no alignment assumptions, but provides worse performance.
    #[inline]
    unsafe fn read_as_unaligned<U>(self) -> U {
        let u_ptr = self.as_ptr() as *const U;
        u_ptr.read_unaligned()
    }

    /// Reads the value from self without moving it. This leaves the memory in self unchanged.
    ///
    /// This assumes that the alignment of `Self::Pointee` matches the alignment of `self.as_ptr()`.
    #[inline]
    unsafe fn read_as<U>(self) -> U {
        assume!(self.is_aligned(), "attempt to read from misaligned pointer");
        if mem::align_of::<U>() > mem::align_of::<Self::Pointee>() {
            let mut u: U = mem::uninitialized();

            default_aligned_structs! {read_as_gt_helper!{self, u}}
            u
        } else {
            self.read_as_aligned()
        }
    }

    /// Calculates an offset via pointer subtraction c-style.
    #[inline]
    unsafe fn offset_from<U: PtrExt<Pointee = Self::Pointee>>(self, origin: U) -> usize {
        assume!(
            self.as_ptr() >= origin.as_ptr(),
            "attempt to calculate a negative `offset_from`"
        );
        assume!(
            self.as_ptr().is_aligned(),
            "attempt to calculate offset from a misaligned pointer"
        );
        assume!(
            origin.as_ptr().is_aligned(),
            "attempt to calculate offset from a misaligned pointer"
        );
        self.as_ptr().offset_from(origin.as_ptr()) as usize
    }

    /// Copies a value from one pointer to another without bounds checking (i.e. memcpy).
    #[inline]
    unsafe fn copy_to<U: PtrMutExt<Pointee = Self::Pointee>>(self, dest: U) {
        self.copy_to_n(dest.as_mut_ptr(), 1)
    }

    /// Copies a value from one pointer to another with bounds checking (i.e. memmove).
    #[inline]
    unsafe fn move_to<U: PtrMutExt<Pointee = Self::Pointee>>(self, dest: U) {
        self.move_to_n(dest.as_mut_ptr(), 1)
    }

    /// Copies values from one pointer to another without bounds checking (i.e. memcpy).
    #[inline]
    unsafe fn copy_to_n<U: PtrMutExt<Pointee = Self::Pointee>>(self, dest: U, n: usize) {
        assume!(
            self.as_ptr().is_aligned(),
            "attempt to copy from a misaligned pointer"
        );
        assume!(
            dest.as_ptr().is_aligned(),
            "attempt to copy to a misaligned pointer"
        );
        self.as_ptr().copy_to_nonoverlapping(dest.as_mut_ptr(), n)
    }

    /// Copies values from one pointer to another with bounds checking (i.e. memmove).
    #[inline]
    unsafe fn move_to_n<U: PtrMutExt<Pointee = Self::Pointee>>(self, dest: U, n: usize) {
        assume!(
            self.as_ptr().is_aligned(),
            "attempt to move from a misaligned pointer"
        );
        assume!(
            dest.as_ptr().is_aligned(),
            "attempt to move to a misaligned pointer"
        );
        self.as_ptr().copy_to(dest.as_mut_ptr(), n)
    }
}

pub trait PtrMutExt: PtrExt {
    /// Converts self into a raw mut pointer.
    #[inline]
    fn as_mut_ptr(self) -> *mut Self::Pointee {
        self.as_ptr() as *mut _
    }

    /// Overwrites a memory location with the given value without reading or dropping the old value.
    #[inline]
    unsafe fn write_aligned(self, value: Self::Pointee) -> Self {
        self.as_mut_ptr().assume_aligned().write(value);
        self.add(1)
    }

    /// Overwrites a memory location with the given value without reading or dropping the old value.
    ///
    /// Unlike `write_aligned`, the pointer may be unaligned.
    #[inline]
    unsafe fn write_unaligned(self, value: Self::Pointee) {
        self.as_mut_ptr().write_unaligned(value)
    }

    /// Overwrites a memory location with the given value without reading or dropping the old value.
    #[inline]
    unsafe fn write_as_aligned<U>(self, value: U) {
        let u_ptr = self.as_ptr() as *mut U;
        u_ptr.assume_aligned().write(value)
    }

    /// Overwrites a memory location with the given value without reading or dropping the old value.
    #[inline]
    unsafe fn write_as_unaligned<U>(self, value: U) {
        let u_ptr = self.as_ptr() as *mut U;
        u_ptr.write_unaligned(value)
    }

    /// Overwrites a memory location with the given value without reading or dropping the old value.
    #[inline]
    unsafe fn write_as<U>(self, value: U) {
        assume!(
            self.is_aligned(),
            "attempt to write to a misaligned pointer"
        );
        if mem::align_of::<U>() > mem::align_of::<Self::Pointee>() {
            default_aligned_structs! {write_as_gt_helper!{self, value}}
        } else {
            self.write_as_aligned(value)
        }
    }

    /// Executes the destructor (if any) of the pointed-to value.
    #[inline]
    unsafe fn drop_in_place_aligned(self) {
        self.as_mut_ptr().assume_aligned().drop_in_place()
    }
}

impl<T> PtrExt for *const T {
    type Pointee = T;

    #[inline]
    fn as_ptr(self) -> *const T {
        self
    }

    #[inline]
    unsafe fn add(self, value: usize) -> Self {
        assume!(
            (self.as_ptr() as usize).checked_add(value).is_some(),
            "overflow on `PtrExt::add`"
        );
        self.add(value)
    }

    #[inline]
    unsafe fn sub(self, value: usize) -> Self {
        assume!(
            (self.as_ptr() as usize).checked_sub(value).is_some(),
            "overflow on `PtrExt::sub`"
        );
        self.sub(value)
    }

    #[inline]
    unsafe fn align_next(self) -> Self {
        let offset = self.align_offset_bytes(mem::align_of::<T>());
        PtrExt::add(self as *const u8, offset) as _
    }
}

impl<T> PtrExt for *mut T {
    type Pointee = T;

    #[inline]
    fn as_ptr(self) -> *const T {
        (self as *const T).as_ptr()
    }

    #[inline]
    unsafe fn add(self, value: usize) -> Self {
        assume!(
            (self.as_ptr() as usize).checked_add(value).is_some(),
            "overflow on `PtrExt::add`"
        );
        self.add(value)
    }

    #[inline]
    unsafe fn sub(self, value: usize) -> Self {
        assume!(
            (self.as_ptr() as usize).checked_sub(value).is_some(),
            "overflow on `PtrExt::sub`"
        );
        self.sub(value)
    }

    #[inline]
    unsafe fn align_next(self) -> Self {
        self.as_ptr().align_next() as _
    }
}

impl<T> PtrMutExt for *mut T {}

impl<T> PtrExt for NonNull<T> {
    type Pointee = T;

    #[inline]
    fn as_ptr(self) -> *const T {
        NonNull::as_ptr(self)
    }

    #[inline]
    unsafe fn add(self, value: usize) -> Self {
        NonNull::new_unchecked(PtrExt::add(self.as_ptr(), value))
    }

    #[inline]
    unsafe fn sub(self, value: usize) -> Self {
        NonNull::new_unchecked(PtrExt::sub(self.as_ptr(), value))
    }

    #[inline]
    unsafe fn align_next(self) -> Self {
        NonNull::new_unchecked(self.as_ptr().align_next())
    }
}

impl<T> PtrMutExt for NonNull<T> {}
