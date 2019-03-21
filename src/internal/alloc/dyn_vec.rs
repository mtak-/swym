//! An contiguous container of Dynamically Sized Types.

use crate::internal::{
    alloc::{fvec::FVec, DefaultAlloc},
    pointer::PtrExt,
};
use std::{
    alloc::Alloc,
    borrow::{Borrow, BorrowMut},
    marker::{PhantomData, Unsize},
    mem::{self, ManuallyDrop},
    num::NonZeroUsize,
    ops::{Deref, DerefMut},
    ptr::{self, NonNull},
    raw::TraitObject,
};

const START_CAPACITY: usize = 1024;

#[repr(C)]
#[derive(Debug)]
pub struct DynVec<T: ?Sized, A: Alloc = DefaultAlloc> {
    data:    FVec<usize, A>,
    phantom: PhantomData<T>,
}

impl<T: ?Sized, A: Alloc> Drop for DynVec<T, A> {
    fn drop(&mut self) {
        self.clear()
    }
}

impl<T: ?Sized, A: Alloc> DynVec<T, A> {
    #[inline]
    pub fn with_alloc_and_capacity(allocator: A, capacity: NonZeroUsize) -> Self {
        DynVec {
            data:    FVec::with_alloc_and_capacity(allocator, capacity),
            phantom: PhantomData,
        }
    }

    #[inline]
    pub fn new() -> Self
    where
        A: Default,
    {
        DynVec::with_alloc_and_capacity(
            A::default(),
            NonZeroUsize::new(START_CAPACITY).expect("zero start capacities unsupported"),
        )
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
        self.data.next_n_pushes_allocates(
            NonZeroUsize::new(mem::size_of::<Elem<U>>() / mem::size_of::<usize>()).unwrap(),
        )
    }

    #[inline]
    pub fn push<U: Unsize<T>>(&mut self, u: U) {
        assert!(
            mem::align_of::<U>() <= mem::align_of::<usize>(),
            "overaligned types are currently unimplemented"
        );
        let elem = Elem::new::<T>(u);

        // extend_non_empty requires the slice to have non-zero length.
        // elem has a vtable pointer in it, so it's never zero sized.
        unsafe {
            self.data.extend_non_empty(elem.as_slice());
        }
        mem::forget(elem)
    }

    #[inline]
    pub unsafe fn push_unchecked<U: Unsize<T>>(&mut self, u: U) {
        assert!(
            mem::align_of::<U>() <= mem::align_of::<usize>(),
            "overaligned types are currently unimplemented"
        );
        let elem = Elem::new::<T>(u);
        self.data.extend_non_empty_unchecked(elem.as_slice());
        mem::forget(elem)
    }

    #[inline]
    pub fn clear(&mut self) {
        let i = IterMut {
            cur:     self.data.begin,
            end:     self.data.end,
            phantom: PhantomData::<&mut T>,
        };
        unsafe {
            self.data.clear();
            for mut x in i {
                ptr::drop_in_place::<T>(&mut *x);
            }
        }
    }

    #[inline]
    pub fn clear_no_drop(&mut self) {
        self.data.clear();
    }

    #[inline]
    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            cur:     self.data.begin,
            end:     self.data.end,
            phantom: PhantomData,
        }
    }

    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, T> {
        IterMut {
            cur:     self.data.begin,
            end:     self.data.end,
            phantom: PhantomData,
        }
    }

    #[inline]
    pub fn drain(&mut self) -> Drain<'_, T> {
        let end = self.data.end;
        let cur = self.data.begin;
        self.data.end = cur;
        Drain {
            cur,
            end,
            phantom: PhantomData,
        }
    }
}

impl<'a, T: ?Sized, A: Alloc> IntoIterator for &'a DynVec<T, A> {
    type IntoIter = Iter<'a, T>;
    type Item = &'a T;

    #[inline]
    fn into_iter(self) -> Iter<'a, T> {
        self.iter()
    }
}

impl<'a, T: ?Sized, A: Alloc> IntoIterator for &'a mut DynVec<T, A> {
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
        assert!(mem::size_of::<Self>() % mem::size_of::<usize>() == 0);
        assert_eq!(mem::size_of::<&T>(), mem::size_of::<TraitObject>());
        let t = &elem as &T;
        let vtable = unsafe { mem::transmute::<&&T, &TraitObject>(&t).vtable };
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
    cur:     NonNull<usize>,
    end:     NonNull<usize>,
    phantom: PhantomData<&'a T>,
}

impl<'a, T: ?Sized> Iterator for Iter<'a, T> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        debug_assert!(self.cur <= self.end, "read past the end of ArenaIter");
        if likely!(self.cur < self.end) {
            unsafe {
                let vtable = self.cur.read_aligned() as *mut ();
                let data_non_null = self.cur.add(1);
                let data = data_non_null.as_ptr() as *mut ();
                let result = {
                    let raw = TraitObject { data, vtable };
                    *mem::transmute::<&TraitObject, &&T>(&raw)
                };
                let size = mem::size_of_val(result);
                debug_assert!(
                    size % mem::size_of::<usize>() == 0,
                    "invalid size detected for dyn T"
                );
                self.cur = data_non_null.add(size / mem::size_of::<usize>());
                Some(result)
            }
        } else {
            None
        }
    }
}

pub struct IterMut<'a, T: ?Sized> {
    cur:     NonNull<usize>,
    end:     NonNull<usize>,
    phantom: PhantomData<&'a mut T>,
}

impl<'a, T: ?Sized> Iterator for IterMut<'a, T> {
    type Item = DynElemMut<'a, T>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        debug_assert!(self.cur <= self.end, "read past the end of ArenaIter");
        if likely!(self.cur < self.end) {
            unsafe {
                let vtable = self.cur.read_aligned() as *mut ();
                let data_non_null = self.cur.add(1);
                let data = data_non_null.as_ptr() as *mut ();
                let result = {
                    let raw = TraitObject { data, vtable };
                    &mut **mem::transmute::<*const TraitObject, *const *mut T>(&raw)
                };
                let size = mem::size_of_val(result);
                debug_assert!(
                    size % mem::size_of::<usize>() == 0,
                    "invalid size detected for dyn T"
                );
                self.cur = data_non_null.add(size / mem::size_of::<usize>());
                Some(DynElemMut { value: result })
            }
        } else {
            None
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
    cur:     NonNull<usize>,
    end:     NonNull<usize>,
    phantom: PhantomData<&'a mut T>,
}

impl<'a, T: ?Sized> Iterator for Drain<'a, T> {
    type Item = Owned<'a, T>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        debug_assert!(self.cur <= self.end, "read past the end of ArenaIter");
        if likely!(self.cur < self.end) {
            unsafe {
                let vtable = self.cur.read_aligned() as *mut ();
                let data_non_null = self.cur.add(1);
                let data = data_non_null.as_ptr() as *mut ();
                let value = {
                    let raw = TraitObject { data, vtable };
                    &mut **mem::transmute::<*const TraitObject, *const *mut T>(&raw)
                };
                let size = mem::size_of_val(value);
                debug_assert!(
                    size % mem::size_of::<usize>() == 0,
                    "invalid size detected for dyn T"
                );
                self.cur = data_non_null.add(size / mem::size_of::<usize>());
                Some(Owned { value })
            }
        } else {
            None
        }
    }
}
