//! An contiguous container of Dynamically Sized Types.

use crate::internal::{alloc::fvec::FVec, pointer::PtrExt};
use core::{
    borrow::{Borrow, BorrowMut},
    marker::{PhantomData, Unsize},
    mem::{self, ManuallyDrop},
    ops::{Deref, DerefMut},
    ptr::{self, NonNull},
};

#[repr(C)]
#[derive(Copy, Clone)]
pub struct TraitObject {
    pub data:   *mut (),
    pub vtable: *mut (),
}

impl TraitObject {
    #[inline]
    pub fn from_pointer<T: ?Sized>(fat: NonNull<T>) -> Self {
        assert_eq!(mem::size_of::<Self>(), mem::size_of::<NonNull<T>>());
        unsafe { mem::transmute_copy::<NonNull<T>, Self>(&fat) }
    }

    #[inline]
    pub unsafe fn from_flat(flat: NonNull<usize>) -> Self {
        let vtable = (*flat.as_ref()) as *mut ();
        let data = flat.add(1).cast().as_ptr();
        TraitObject { data, vtable }
    }

    #[inline]
    pub unsafe fn cast<T: ?Sized>(self) -> NonNull<T> {
        assert_eq!(mem::size_of::<Self>(), mem::size_of::<NonNull<T>>());
        let result = mem::transmute_copy::<Self, NonNull<T>>(&self);
        debug_assert!(mem::align_of_val(result.as_ref()) <= mem::align_of::<usize>());
        result
    }
}

#[inline]
pub fn vtable<T: ?Sized>(value: &T) -> *mut () {
    TraitObject::from_pointer(value.into()).vtable
}

#[repr(C)]
#[derive(Debug)]
pub struct DynVec<T: ?Sized> {
    data:    FVec<usize>,
    phantom: PhantomData<T>,
}

impl<T: ?Sized> Drop for DynVec<T> {
    fn drop(&mut self) {
        self.clear()
    }
}

impl<T: ?Sized> DynVec<T> {
    #[inline]
    pub fn new() -> Self {
        DynVec {
            data:    FVec::new(),
            phantom: PhantomData,
        }
    }

    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        DynVec {
            data:    FVec::with_capacity(capacity),
            phantom: PhantomData,
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    #[inline]
    pub fn word_capacity(&self) -> usize {
        self.data.capacity()
    }

    #[inline]
    pub fn word_len(&self) -> usize {
        self.data.len()
    }

    #[inline]
    pub fn next_push_allocates<U: Unsize<T>>(&self) -> bool {
        assert!(
            mem::align_of::<U>() <= mem::align_of::<usize>(),
            "overaligned types are currently unimplemented"
        );
        debug_assert!(mem::size_of::<Elem<U>>() % mem::size_of::<usize>() == 0);
        self.data
            .next_n_pushes_allocates(mem::size_of::<Elem<U>>() / mem::size_of::<usize>())
    }

    #[inline]
    pub fn push<U: Unsize<T>>(&mut self, u: U) {
        assert!(
            mem::align_of::<U>() <= mem::align_of::<usize>(),
            "overaligned types are currently unimplemented"
        );
        let elem = Elem::new::<T>(u);
        self.data.extend(elem.as_slice());
        mem::forget(elem)
    }

    #[inline]
    pub unsafe fn push_unchecked<U: Unsize<T>>(&mut self, u: U) {
        assert!(
            mem::align_of::<U>() <= mem::align_of::<usize>(),
            "overaligned types are currently unimplemented"
        );
        let elem = Elem::new::<T>(u);
        self.data.extend_unchecked(elem.as_slice());
        mem::forget(elem)
    }

    #[inline]
    pub fn clear(&mut self) {
        self.drain().for_each(|_| {})
    }

    #[inline]
    pub fn clear_no_drop(&mut self) {
        self.data.clear();
    }

    #[inline]
    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            iter:    self.data.iter(),
            phantom: PhantomData,
        }
    }

    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, T> {
        IterMut {
            iter:    self.data.iter_mut(),
            phantom: PhantomData,
        }
    }

    #[inline]
    pub fn drain(&mut self) -> Drain<'_, T> {
        let slice: &mut [_] = &mut self.data;
        let raw: NonNull<_> = slice.into();
        self.data.clear();

        Drain {
            iter:    IterMut {
                iter:    unsafe { &mut *raw.as_ptr() }.iter_mut(),
                phantom: PhantomData,
            },
            phantom: PhantomData,
        }
    }
}

impl<'a, T: ?Sized> IntoIterator for &'a DynVec<T> {
    type IntoIter = Iter<'a, T>;
    type Item = &'a T;

    #[inline]
    fn into_iter(self) -> Iter<'a, T> {
        self.iter()
    }
}

impl<'a, T: ?Sized> IntoIterator for &'a mut DynVec<T> {
    type IntoIter = IterMut<'a, T>;
    type Item = DynElemMut<'a, T>;

    #[inline]
    fn into_iter(self) -> IterMut<'a, T> {
        self.iter_mut()
    }
}

#[repr(C)]
struct Elem<U> {
    vtable: *const (),
    elem:   U,
}

impl<U> Elem<U> {
    #[inline]
    fn new<T: ?Sized>(elem: U) -> Self
    where
        U: Unsize<T>,
    {
        let vtable = vtable(&elem as &T);
        Elem { vtable, elem }
    }

    #[inline]
    fn as_slice(&self) -> &[usize] {
        unsafe {
            std::slice::from_raw_parts(
                self as *const _ as _,
                mem::size_of::<Self>() / mem::size_of::<usize>(),
            )
        }
    }
}

pub struct DynElemMut<'a, T: ?Sized> {
    value: &'a mut T,
}

impl<'a, T: ?Sized> Deref for DynElemMut<'a, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<'a, T: ?Sized> DerefMut for DynElemMut<'a, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        self.value
    }
}

impl<'a, T: ?Sized> DynElemMut<'a, T> {
    #[inline]
    pub unsafe fn assign_unchecked<U: Unsize<T>>(this: Self, rhs: U) {
        debug_assert_eq!(
            mem::size_of_val(this.value),
            mem::size_of::<U>(),
            "attempt to assign DynElemMut a value with a different size"
        );
        debug_assert!(
            mem::align_of_val(this.value) >= mem::align_of::<U>(),
            "attempt to assign DynElemMut a value with an incompatible alignment"
        );
        debug_assert!(
            mem::align_of_val(this.value) <= mem::align_of::<*const ()>(),
            "attempt to assign DynElemMut a value with an incompatible alignment"
        );

        // not the safest code ever
        let mut punned = ManuallyDrop::new(ptr::read(this.value as *const T as *const U));
        let vtable_storage;
        let old_raw = {
            let mut raw = mem::transmute_copy::<&mut T, TraitObject>(&this.value);
            vtable_storage =
                (mem::replace(&mut raw.data, &mut punned as *mut _ as _) as *mut *const ()).sub(1);
            raw
        };
        let new_vtable = {
            let null = ptr::null_mut::<U>() as *mut T;
            let raw: TraitObject = mem::transmute_copy(&null);
            raw.vtable
        };
        vtable_storage.write(new_vtable);
        (this.value as *mut T as *mut U).write(rhs);

        ptr::drop_in_place(mem::transmute_copy::<_, *mut T>(&old_raw));
    }
}

pub struct Iter<'a, T: ?Sized> {
    iter:    std::slice::Iter<'a, usize>,
    phantom: PhantomData<&'a T>,
}

impl<'a, T: ?Sized> Iterator for Iter<'a, T> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            let raw = TraitObject::from_flat(self.iter.next()?.into());
            let result = &*raw.cast().as_ptr();
            let size = mem::size_of_val(result);
            debug_assert!(
                size % mem::size_of::<usize>() == 0,
                "invalid size detected for dyn T"
            );
            for _ in 0..size / mem::size_of::<usize>() {
                match self.iter.next() {
                    None => std::hint::unreachable_unchecked(),
                    _ => {}
                }
            }
            Some(result)
        }
    }
}

pub struct IterMut<'a, T: ?Sized> {
    iter:    std::slice::IterMut<'a, usize>,
    phantom: PhantomData<&'a mut T>,
}

impl<'a, T: ?Sized> Iterator for IterMut<'a, T> {
    type Item = DynElemMut<'a, T>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            let raw = TraitObject::from_flat(self.iter.next()?.into());
            let result = &mut *raw.cast().as_ptr();
            let size = mem::size_of_val(result);
            debug_assert!(
                size % mem::size_of::<usize>() == 0,
                "invalid size detected for dyn T"
            );
            for _ in 0..size / mem::size_of::<usize>() {
                match self.iter.next() {
                    None => std::hint::unreachable_unchecked(),
                    _ => {}
                }
            }
            Some(DynElemMut { value: result })
        }
    }
}

pub struct Owned<'a, T: ?Sized> {
    value: &'a mut T,
}

impl<'a, T: ?Sized> Drop for Owned<'a, T> {
    #[inline]
    fn drop(&mut self) {
        unsafe { ptr::drop_in_place(self.value) }
    }
}

impl<'a, T: ?Sized> Borrow<T> for Owned<'a, T> {
    #[inline]
    fn borrow(&self) -> &T {
        self.value
    }
}

impl<'a, T: ?Sized> BorrowMut<T> for Owned<'a, T> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        self.value
    }
}

impl<'a, T: ?Sized> Deref for Owned<'a, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, T: ?Sized> DerefMut for Owned<'a, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value
    }
}

pub struct Drain<'a, T: ?Sized> {
    iter:    IterMut<'a, T>,
    phantom: PhantomData<Box<T>>,
}

impl<'a, T: 'a + ?Sized> Iterator for Drain<'a, T> {
    type Item = Owned<'a, T>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|DynElemMut { value }| Owned { value })
    }
}

#[cfg(test)]
mod trait_object {
    #[cfg(feature = "unstable")]
    #[test]
    fn layout() {
        use super::TraitObject;
        use std::{mem, raw::TraitObject as StdTraitObject};

        assert_eq!(
            mem::size_of::<TraitObject>(),
            mem::size_of::<StdTraitObject>()
        );
        assert_eq!(
            mem::align_of::<TraitObject>(),
            mem::align_of::<StdTraitObject>()
        );
        let x = String::from("hello there");
        unsafe {
            let y: &dyn std::fmt::Debug = &x;
            let std = mem::transmute::<&dyn std::fmt::Debug, StdTraitObject>(y);
            let raw = TraitObject::from_pointer(y.into());
            assert_eq!(raw.vtable, std.vtable);
            assert_eq!(raw.data, std.data);
        }
    }
}