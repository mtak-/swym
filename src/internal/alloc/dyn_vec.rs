//! An contiguous container of Dynamically Sized Types.

use core::{
    borrow::{Borrow, BorrowMut},
    marker::PhantomData,
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
        let data = flat.as_ptr().add(1) as *mut ();
        TraitObject { data, vtable }
    }

    #[inline]
    pub unsafe fn cast<T: ?Sized>(self) -> NonNull<T> {
        debug_assert!(!self.data.is_null());
        debug_assert!(!self.vtable.is_null());
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

macro_rules! dyn_vec_decl {
    ($vis:vis struct $name:ident: $trait:path;) => {
        #[repr(C)]
        #[derive(Debug)]
        $vis struct $name<'a> {
            data:    $crate::internal::alloc::FVec<usize>,
            phantom: ::std::marker::PhantomData<dyn $trait + 'a>,
        }

        impl Drop for $name<'_> {
            fn drop(&mut self) {
                self.clear()
            }
        }

        #[allow(unused)]
        impl $name<'_> {
            #[inline]
            $vis fn new() -> Self {
                $name {
                    data:    $crate::internal::alloc::FVec::new(),
                    phantom: ::std::marker::PhantomData,
                }
            }

            #[inline]
            $vis fn with_capacity(capacity: usize) -> Self {
                $name {
                    data:    $crate::internal::alloc::FVec::with_capacity(capacity),
                    phantom: ::std::marker::PhantomData,
                }
            }

            #[inline]
            $vis fn is_empty(&self) -> bool {
                self.data.is_empty()
            }

            #[inline]
            $vis fn word_capacity(&self) -> usize {
                self.data.capacity()
            }

            #[inline]
            $vis fn word_len(&self) -> usize {
                self.data.len()
            }

            #[inline]
            $vis fn next_push_allocates<U: $trait>(&self) -> bool {
                assert!(
                    mem::align_of::<U>() <= mem::align_of::<usize>(),
                    "overaligned types are currently unimplemented"
                );
                debug_assert!(mem::size_of::<$crate::internal::alloc::dyn_vec::Elem<U>>() % mem::size_of::<usize>() == 0);
                self.data
                    .next_n_pushes_allocates(mem::size_of::<$crate::internal::alloc::dyn_vec::Elem<U>>() / mem::size_of::<usize>())
            }

            #[inline]
            $vis fn push<U: $trait>(&mut self, u: U) {
                assert!(
                    mem::align_of::<U>() <= mem::align_of::<usize>(),
                    "overaligned types are currently unimplemented"
                );
                let elem = $crate::internal::alloc::dyn_vec::Elem::new($crate::internal::alloc::dyn_vec::vtable(&u as &dyn $trait), u);
                self.data.extend(elem.as_slice());
                mem::forget(elem)
            }

            #[inline]
            $vis unsafe fn push_unchecked<U: $trait>(&mut self, u: U) {
                assert!(
                    mem::align_of::<U>() <= mem::align_of::<usize>(),
                    "overaligned types are currently unimplemented"
                );
                let elem = $crate::internal::alloc::dyn_vec::Elem::new($crate::internal::alloc::dyn_vec::vtable(&u as &dyn $trait), u);
                self.data.extend_unchecked(elem.as_slice());
                mem::forget(elem)
            }

            #[inline]
            $vis fn clear(&mut self) {
                self.drain().for_each(|_| {})
            }

            #[inline]
            $vis fn clear_no_drop(&mut self) {
                self.data.clear();
            }

            #[inline]
            $vis fn iter(&self) -> $crate::internal::alloc::dyn_vec::Iter<'_, dyn $trait> {
                unsafe {
                    $crate::internal::alloc::dyn_vec::Iter::new(
                        self.data.iter()
                    )
                }
            }

            #[inline]
            $vis fn iter_mut(&mut self) -> $crate::internal::alloc::dyn_vec::IterMut<'_, dyn $trait> {
                unsafe {
                    $crate::internal::alloc::dyn_vec::IterMut::new(
                        self.data.iter_mut()
                    )
                }
            }

            #[inline]
            $vis fn drain(&mut self) -> $crate::internal::alloc::dyn_vec::Drain<'_, dyn $trait> {
                let slice: &mut [_] = &mut self.data;
                let raw: ::std::ptr::NonNull<_> = slice.into();
                self.data.clear();

                unsafe {
                    $crate::internal::alloc::dyn_vec::Drain::new(
                        (*raw.as_ptr()).iter_mut()
                    )
                }
            }
        }

        impl<'a> IntoIterator for &'a $name<'_> {
            type IntoIter = $crate::internal::alloc::dyn_vec::Iter<'a, dyn $trait + 'static>;
            type Item = &'a (dyn $trait + 'static);

            #[inline]
            fn into_iter(self) -> $crate::internal::alloc::dyn_vec::Iter<'a, dyn $trait> {
                self.iter()
            }
        }

        impl<'a> IntoIterator for &'a mut $name<'_> {
            type IntoIter = $crate::internal::alloc::dyn_vec::IterMut<'a, dyn $trait>;
            type Item = $crate::internal::alloc::dyn_vec::DynElemMut<'a, dyn $trait>;

            #[inline]
            fn into_iter(self) -> $crate::internal::alloc::dyn_vec::IterMut<'a, dyn $trait> {
                self.iter_mut()
            }
        }
    };
}

#[repr(C)]
pub struct Elem<U> {
    vtable: *const (),
    elem:   U,
}

impl<U> Elem<U> {
    #[inline]
    pub fn new(vtable: *const (), elem: U) -> Self {
        Elem { vtable, elem }
    }

    #[inline]
    pub fn as_slice(&self) -> &[usize] {
        unsafe {
            std::slice::from_raw_parts(
                self as *const _ as _,
                mem::size_of::<Self>() / mem::size_of::<usize>(),
            )
        }
    }
}

#[derive(Debug)]
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
    pub unsafe fn assign_unchecked<U>(this: Self, new_vtable: *const (), rhs: U) {
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
            let mut raw = TraitObject::from_pointer(this.value.into());
            vtable_storage =
                (mem::replace(&mut raw.data, &mut punned as *mut _ as _) as *mut *const ()).sub(1);
            raw
        };
        vtable_storage.write(new_vtable);
        (this.value as *mut T as *mut U).write(rhs);

        ptr::drop_in_place(mem::transmute_copy::<_, *mut T>(&old_raw));
    }
}

#[derive(Debug)]
pub struct Iter<'a, T: ?Sized> {
    iter:    std::slice::Iter<'a, usize>,
    phantom: PhantomData<&'a T>,
}

impl<'a, T: ?Sized> Iter<'a, T> {
    pub unsafe fn new(iter: std::slice::Iter<'a, usize>) -> Self {
        Iter {
            iter,
            phantom: PhantomData,
        }
    }
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

#[derive(Debug)]
pub struct IterMut<'a, T: ?Sized> {
    iter:    std::slice::IterMut<'a, usize>,
    phantom: PhantomData<&'a mut T>,
}

impl<'a, T: ?Sized> IterMut<'a, T> {
    pub unsafe fn new(iter: std::slice::IterMut<'a, usize>) -> Self {
        IterMut {
            iter,
            phantom: PhantomData,
        }
    }
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

impl<'a, T: ?Sized> Drain<'a, T> {
    pub unsafe fn new(iter: std::slice::IterMut<'a, usize>) -> Self {
        Drain {
            iter:    IterMut::new(iter),
            phantom: PhantomData,
        }
    }
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
