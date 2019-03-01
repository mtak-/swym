use crate::internal::{
    alloc::DefaultAlloc,
    pointer::{PtrExt, PtrMutExt},
};
use std::{
    alloc::{Alloc, Layout},
    fmt::{self, Debug, Formatter},
    iter::TrustedLen,
    marker::PhantomData,
    mem,
    num::NonZeroUsize,
    ptr::{self, NonNull},
};

#[repr(C)]
pub struct FVec<T, A = DefaultAlloc>
where
    A: Alloc,
{
    pub(crate) end:     NonNull<T>,
    last_valid_address: NonNull<T>,
    pub(crate) begin:   NonNull<T>,
    _phantom:           PhantomData<T>,
    allocator:          A,
}

unsafe impl<T: Send, A: Alloc + Send> Send for FVec<T, A> {}
unsafe impl<T: Sync, A: Alloc + Sync> Sync for FVec<T, A> {}

unsafe impl<#[may_dangle] T, A: Alloc> Drop for FVec<T, A> {
    #[inline]
    fn drop(&mut self) {
        self.validate();
        unsafe {
            if mem::needs_drop::<T>() {
                for e in self.iter_mut() {
                    ptr::drop_in_place(e)
                }
            }
            self.allocator.dealloc(self.begin.cast(), self.layout());
        }
    }
}

impl<T, A: Alloc> FVec<T, A> {
    #[inline]
    pub fn new() -> FVec<T, A>
    where
        A: Default,
    {
        FVec::with_alloc_and_capacity(A::default(), START_SIZE)
    }

    #[inline]
    pub fn with_alloc_and_capacity(mut allocator: A, capacity: NonZeroUsize) -> FVec<T, A> {
        assert!(
            mem::size_of::<T>() > 0,
            "`FVec` does not support zero sized types"
        );

        unsafe {
            let layout = Layout::from_size_align_unchecked(
                mem::size_of::<T>() * capacity.get(),
                mem::align_of::<T>(),
            );
            let buf = match allocator.alloc(layout) {
                Ok(buf) => buf.cast(),
                Err(_) => std::alloc::handle_alloc_error(layout),
            };
            FVec {
                end: buf,
                last_valid_address: buf.add(capacity.get() - 1),
                begin: buf,
                _phantom: PhantomData,
                allocator,
            }
        }
    }

    #[inline]
    pub fn with_capacity(capacity: NonZeroUsize) -> FVec<T, A>
    where
        A: Default,
    {
        FVec::with_alloc_and_capacity(A::default(), capacity)
    }

    #[inline]
    pub fn with_alloc(alloc: A) -> FVec<T, A> {
        FVec::with_alloc_and_capacity(alloc, START_SIZE)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.end == self.begin
    }

    #[inline]
    pub fn len(&self) -> usize {
        unsafe { self.end.offset_from(self.begin) }
    }

    // capacity in terms of number of elements
    #[inline]
    pub fn capacity(&self) -> usize {
        unsafe { self.last_valid_address.add(1).offset_from(self.begin) }
    }

    #[inline]
    pub fn next_push_allocates(&self) -> bool {
        self.end == self.last_valid_address
    }

    #[inline]
    pub fn next_n_pushes_allocates(&self, n: NonZeroUsize) -> bool {
        // UB if we used ptr::add
        (self.end.as_ptr() as usize + (n.get() - 1) * mem::size_of::<T>())
            >= self.last_valid_address.as_ptr() as usize
    }

    #[inline]
    pub fn clear(&mut self) {
        let old_end = self.end;
        self.end = self.begin;
        if mem::needs_drop::<T>() {
            let i = IterMut {
                cur:     self.begin,
                end:     old_end,
                phantom: PhantomData::<&mut T>,
            };
            for e in i {
                unsafe { ptr::drop_in_place(e) }
            }
        }
    }

    #[inline]
    pub fn push(&mut self, t: T) {
        // don't change this to push_unchecked, it has an assert that will trigger with "develop" on
        let e = self.end;
        let last_valid = self.last_valid_address;
        unsafe { self.end = e.write_aligned(t) };

        if unlikely!(e == last_valid) {
            self.reserve_more();
        }
        self.validate();
    }

    #[inline]
    pub unsafe fn push_unchecked(&mut self, t: T) {
        debug_assert!(
            !self.next_push_allocates(),
            "`push_unchecked` called when an allocation is required"
        );
        let e = self.end;
        self.end = e.write_aligned(t);
        self.validate();
    }

    // this is faster than swap_erase
    #[inline]
    pub unsafe fn rswap_erase_unchecked(&mut self, index: usize) {
        debug_assert!(
            index < self.len(),
            "providing an index >= self.len() is undefined behavior in release"
        );
        self.end = self.end.sub(1);
        let dest = self.end.sub(index);
        if mem::needs_drop::<T>() {
            dest.drop_in_place_aligned()
        }
        self.end.move_to(dest);
    }

    #[inline]
    pub unsafe fn swap_erase_unchecked(&mut self, index: usize) {
        debug_assert!(
            index < self.len(),
            "providing an index >= self.len() is undefined behavior in release"
        );
        let dest = self.begin.add(index);
        self.end = self.end.sub(1);
        if mem::needs_drop::<T>() {
            dest.drop_in_place_aligned()
        }
        self.end.move_to(dest);
    }

    #[inline]
    pub unsafe fn get_unchecked(&self, index: usize) -> &T {
        debug_assert!(
            index < self.len(),
            "providing an index >= self.len() is undefined behavior in release"
        );
        &*self.begin.add(index).as_ptr()
    }

    #[inline]
    pub unsafe fn get_mut_unchecked(&mut self, index: usize) -> &mut T {
        debug_assert!(
            index < self.len(),
            "providing an index >= self.len() is undefined behavior in release"
        );
        &mut *self.begin.add(index).as_mut_ptr()
    }

    #[inline]
    pub unsafe fn rget_unchecked(&self, index: usize) -> &T {
        debug_assert!(
            index < self.len(),
            "providing an index >= self.len() is undefined behavior in release"
        );
        &*self.end.sub(index + 1).as_ptr()
    }

    #[inline]
    pub unsafe fn rget_mut_unchecked(&mut self, index: usize) -> &mut T {
        debug_assert!(
            index < self.len(),
            "providing an index >= self.len() is undefined behavior in release"
        );
        &mut *self.end.sub(index + 1).as_mut_ptr()
    }

    #[inline]
    pub unsafe fn back_unchecked(&mut self) -> &mut T {
        &mut *self.end.sub(1).as_ptr()
    }

    #[inline]
    pub fn pop(&mut self) -> Option<T> {
        if likely!(!self.is_empty()) {
            unsafe { Some(self.pop_unchecked()) }
        } else {
            None
        }
    }

    #[inline]
    pub unsafe fn pop_unchecked(&mut self) -> T {
        debug_assert!(
            !self.is_empty(),
            "`FVec::pop_unchecked` called on an empty FVec"
        );
        self.end = self.end.sub(1);
        self.end.read_aligned()
    }

    #[inline]
    pub fn iter<'a>(&'a self) -> Iter<'a, T> {
        Iter {
            cur:     self.begin,
            end:     self.end,
            phantom: PhantomData,
        }
    }

    #[inline]
    pub fn iter_mut<'a>(&'a mut self) -> IterMut<'a, T> {
        IterMut {
            cur:     self.begin,
            end:     self.end,
            phantom: PhantomData,
        }
    }

    #[inline]
    pub fn drain<'a>(&'a mut self) -> Drain<'a, T> {
        let end = self.end;
        let cur = self.begin;
        self.end = cur;
        self.validate();
        Drain {
            cur,
            end,
            phantom: PhantomData,
        }
    }

    #[inline]
    pub fn drain_while<'a, F: FnMut(&mut T) -> bool>(
        &'a mut self,
        f: F,
    ) -> DrainWhile<'a, T, F, A> {
        let end = self.end;
        let cur = self.begin;
        self.end = cur;
        self.validate();
        DrainWhile {
            cur,
            end,
            v: self,
            f,
        }
    }

    #[inline]
    fn next_capacity(&self) -> usize {
        2 * self.capacity()
    }

    #[inline]
    fn layout(&self) -> Layout {
        unsafe {
            Layout::from_size_align_unchecked(
                mem::size_of::<T>() * self.capacity(),
                mem::align_of::<T>(),
            )
        }
    }

    #[cold]
    #[inline(never)]
    fn reserve_exact(&mut self, next_cap: usize) {
        debug_assert!(
            next_cap > self.capacity(),
            "next capacity is not greater than the current capacity"
        );

        unsafe {
            let new_begin = {
                let layout = self.layout();
                match self.allocator.realloc(
                    self.begin.cast(),
                    layout,
                    next_cap * mem::size_of::<T>(),
                ) {
                    Ok(new) => new.cast(),
                    Err(_) => std::alloc::handle_alloc_error(Layout::from_size_align_unchecked(
                        mem::size_of::<T>() * next_cap,
                        mem::align_of::<T>(),
                    )),
                }
            };

            self.last_valid_address = new_begin.add(next_cap - 1);
            self.end = new_begin.add(self.len());
            self.begin = new_begin;
        }

        self.validate();
    }

    #[cold]
    #[inline(never)]
    fn reserve_more(&mut self) {
        self.reserve_exact(self.next_capacity());
    }

    #[inline(always)]
    fn validate(&self) {
        debug_assert!(
            self.capacity() >= self.len(),
            "`capacity()` < `len()` detected"
        );
        debug_assert!(self.end >= self.begin, "`end < begin` detected");
        debug_assert!(
            self.end <= self.last_valid_address,
            "`end > last_valid_address` detected"
        );
    }
}

impl<T: Copy, A: Alloc> FVec<T, A> {
    #[inline]
    pub unsafe fn extend_non_empty_unchecked(&mut self, slice: &[T]) {
        assume!(slice.len() > 0, "unexpected empty slice");
        debug_assert!(
            !self.next_n_pushes_allocates(NonZeroUsize::new_unchecked(slice.len())),
            "attempt to `extend_non_empty_unchecked` when there is not enough existing free space \
             to do so"
        );
        let l = slice.len();
        let e = self.end;
        self.end = e.add(l);
        slice.as_ptr().copy_to_n(e, l);
    }

    #[inline]
    pub unsafe fn extend_non_empty(&mut self, slice: &[T]) {
        assume!(slice.len() > 0, "unexpected empty slice");
        if likely!(!self.next_n_pushes_allocates(NonZeroUsize::new_unchecked(slice.len()))) {
            self.extend_non_empty_unchecked(slice)
        } else {
            self.extend_non_empty_slow(slice)
        }
    }

    #[inline(never)]
    #[cold]
    unsafe fn extend_non_empty_slow(&mut self, slice: &[T]) {
        let mut c = self.next_capacity();
        while c < self.len() + slice.len() {
            c *= 2;
        }
        self.reserve_exact(c);

        self.extend_non_empty_unchecked(slice)
    }
}

impl<T: Debug, A: Alloc + Debug> Debug for FVec<T, A> {
    #[inline(never)]
    #[cold]
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl<T, A: Alloc> IntoIterator for FVec<T, A> {
    type IntoIter = IntoIter<T, A>;
    type Item = T;

    #[inline]
    fn into_iter(self) -> IntoIter<T, A> {
        let result = IntoIter {
            cur:       self.begin,
            end:       self.end,
            begin:     self.begin,
            allocator: unsafe { ptr::read(&self.allocator) },
            phantom:   PhantomData,
        };
        mem::forget(self);
        result
    }
}

impl<'a, T, A: Alloc> IntoIterator for &'a FVec<T, A> {
    type IntoIter = Iter<'a, T>;
    type Item = &'a T;

    #[inline]
    fn into_iter(self) -> Iter<'a, T> {
        self.iter()
    }
}

impl<'a, T, A: Alloc> IntoIterator for &'a mut FVec<T, A> {
    type IntoIter = IterMut<'a, T>;
    type Item = &'a mut T;

    #[inline]
    fn into_iter(self) -> IterMut<'a, T> {
        self.iter_mut()
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Iter<'a, T> {
    cur:     NonNull<T>,
    end:     NonNull<T>,
    phantom: PhantomData<&'a T>,
}

impl<'a, T: 'a> Iterator for Iter<'a, T> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<&'a T> {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        unsafe {
            let next_ptr = self.cur.add(1);
            if likely!(next_ptr <= self.end) {
                Some(&*mem::replace(&mut self.cur, next_ptr).as_ptr())
            } else {
                None
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        let lower = unsafe { self.end.offset_from(self.cur) };
        (lower, Some(lower))
    }
}

impl<'a, T: 'a> DoubleEndedIterator for Iter<'a, T> {
    #[inline]
    fn next_back(&mut self) -> Option<&'a T> {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        if likely!(self.cur < self.end) {
            unsafe {
                self.end = self.end.sub(1);
                Some(&*self.end.as_ptr())
            }
        } else {
            None
        }
    }
}

impl<'a, T: 'a> ExactSizeIterator for Iter<'a, T> {
    #[inline]
    fn len(&self) -> usize {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        unsafe { self.end.offset_from(self.cur) }
    }
}

unsafe impl<'a, T> TrustedLen for Iter<'a, T> {}

pub struct IterMut<'a, T> {
    cur:     NonNull<T>,
    end:     NonNull<T>,
    phantom: PhantomData<&'a mut T>,
}

impl<'a, T: 'a> Iterator for IterMut<'a, T> {
    type Item = &'a mut T;

    #[inline]
    fn next(&mut self) -> Option<&'a mut T> {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        unsafe {
            let next_ptr = self.cur.add(1);
            if likely!(next_ptr <= self.end) {
                Some(&mut *mem::replace(&mut self.cur, next_ptr).as_ptr())
            } else {
                None
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        let lower = unsafe { self.end.offset_from(self.cur) };
        (lower, Some(lower))
    }
}

impl<'a, T: 'a> DoubleEndedIterator for IterMut<'a, T> {
    #[inline]
    fn next_back(&mut self) -> Option<&'a mut T> {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        if likely!(self.cur < self.end) {
            unsafe {
                self.end = self.end.sub(1);
                Some(&mut *self.end.as_ptr())
            }
        } else {
            None
        }
    }
}

impl<'a, T: 'a> ExactSizeIterator for IterMut<'a, T> {
    #[inline]
    fn len(&self) -> usize {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        unsafe { self.end.offset_from(self.cur) }
    }
}

unsafe impl<'a, T> TrustedLen for IterMut<'a, T> {}

pub struct Drain<'a, T> {
    cur:     NonNull<T>,
    end:     NonNull<T>,
    phantom: PhantomData<&'a mut T>,
}

impl<'a, T: 'a> Iterator for Drain<'a, T> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<T> {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        unsafe {
            let next_ptr = self.cur.add(1);
            if likely!(next_ptr <= self.end) {
                Some(mem::replace(&mut self.cur, next_ptr).read_aligned())
            } else {
                None
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        let lower = unsafe { self.end.offset_from(self.cur) };
        (lower, Some(lower))
    }
}

impl<'a, T: 'a> DoubleEndedIterator for Drain<'a, T> {
    #[inline]
    fn next_back(&mut self) -> Option<T> {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        if likely!(self.cur < self.end) {
            unsafe {
                self.end = self.end.sub(1);
                Some(self.end.read_aligned())
            }
        } else {
            None
        }
    }
}

impl<'a, T: 'a> ExactSizeIterator for Drain<'a, T> {
    #[inline]
    fn len(&self) -> usize {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        unsafe { self.end.offset_from(self.cur) }
    }
}

unsafe impl<'a, T> TrustedLen for Drain<'a, T> {}

#[derive(Debug)]
pub struct DrainWhile<'a, T, F: FnMut(&mut T) -> bool, A: Alloc> {
    cur: NonNull<T>,
    end: NonNull<T>,
    v:   &'a mut FVec<T, A>,
    f:   F,
}

unsafe impl<
        'a,
        #[may_dangle] T: 'a,
        #[may_dangle] F: FnMut(&mut T) -> bool,
        #[may_dangle] A: 'a + Alloc,
    > Drop for DrainWhile<'a, T, F, A>
{
    #[inline]
    fn drop(&mut self) {
        self.v.validate();
        unsafe {
            let offset = self.end.offset_from(self.cur);
            let begin = self.v.begin;
            self.cur.move_to_n(begin, offset);
            self.v.end = begin.add(offset);
        }
        self.v.validate();
    }
}

impl<'a, T, F, A> Iterator for DrainWhile<'a, T, F, A>
where
    A: Alloc,
    F: FnMut(&mut T) -> bool,
{
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        unsafe {
            let next_ptr = self.cur.add(1);
            // optimized for partitioned FVec's
            if likely!(next_ptr <= self.end) && likely!((self.f)(&mut *self.cur.as_ptr())) {
                Some(mem::replace(&mut self.cur, next_ptr).read_aligned())
            } else {
                None
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        let upper = unsafe { self.end.offset_from(self.cur) };
        (0, Some(upper))
    }
}

#[derive(Debug)]
pub struct IntoIter<T, A: Alloc> {
    cur:       NonNull<T>,
    end:       NonNull<T>,
    begin:     NonNull<T>,
    allocator: A,
    phantom:   PhantomData<T>,
}

unsafe impl<#[may_dangle] T, A: Alloc> Drop for IntoIter<T, A> {
    #[inline]
    fn drop(&mut self) {
        let mut cur = self.cur;
        let end = self.end;
        unsafe {
            if mem::needs_drop::<T>() {
                while cur != end {
                    cur.drop_in_place_aligned();
                    cur = cur.add(1);
                }
            }
            self.allocator.dealloc(
                self.begin.cast(),
                Layout::from_size_align_unchecked(
                    mem::size_of::<T>() * end.offset_from(self.begin),
                    mem::align_of::<T>(),
                ),
            );
        }
    }
}

impl<T, A: Alloc> Iterator for IntoIter<T, A> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<T> {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        unsafe {
            let next_ptr = self.cur.add(1);
            if likely!(next_ptr <= self.end) {
                Some(mem::replace(&mut self.cur, next_ptr).read_aligned())
            } else {
                None
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        let lower = unsafe { self.end.offset_from(self.cur) };
        (lower, Some(lower))
    }
}

impl<T, A: Alloc> DoubleEndedIterator for IntoIter<T, A> {
    #[inline]
    fn next_back(&mut self) -> Option<T> {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        if likely!(self.cur < self.end) {
            unsafe {
                self.end = self.end.sub(1);
                Some(self.end.read_aligned())
            }
        } else {
            None
        }
    }
}

impl<T, A: Alloc> ExactSizeIterator for IntoIter<T, A> {
    #[inline]
    fn len(&self) -> usize {
        debug_assert!(self.cur <= self.end, "iterated past the end");
        unsafe { self.end.offset_from(self.cur) }
    }
}

unsafe impl<T, A: Alloc> TrustedLen for IntoIter<T, A> {}

const START_SIZE: NonZeroUsize = unsafe { NonZeroUsize::new_unchecked(1024) };

// used by tests
#[allow(unused)]
macro_rules! fvec {
    ($($es:expr),* $(,)*) => {
        {
            let mut v = $crate::internal::alloc::fvec::FVec::<_, $crate::internal::alloc::DefaultAlloc>::new();
            $(v.push($es);)*
            v
        }
    };
}
