use core::ops::{Deref, DerefMut};

const START_SIZE: usize = 1024;

#[derive(Debug)]
pub struct FVec<T> {
    data: Vec<T>,
}

impl<T> Deref for FVec<T> {
    type Target = Vec<T>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<T> DerefMut for FVec<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl<T> FVec<T> {
    #[inline]
    pub fn new() -> Self {
        FVec::with_capacity(START_SIZE)
    }

    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        FVec {
            data: Vec::with_capacity(capacity),
        }
    }

    #[inline]
    pub fn next_push_allocates(&self) -> bool {
        self.capacity() == self.len()
    }

    #[inline]
    pub fn next_n_pushes_allocates(&self, n: usize) -> bool {
        self.capacity() < self.len() + n
    }

    #[inline]
    pub unsafe fn push_unchecked(&mut self, t: T) {
        if !self.next_push_allocates() {
            self.push(t)
        } else if cfg!(debug_assertions) {
            panic!("`push_unchecked` called when an allocation is required")
        } else {
            std::hint::unreachable_unchecked()
        }
    }

    #[inline]
    pub unsafe fn pop_unchecked(&mut self) -> T {
        if !self.is_empty() {
            self.pop().unwrap()
        } else if cfg!(debug_assertions) {
            panic!("`FVec::pop_unchecked` called on an empty FVec")
        } else {
            std::hint::unreachable_unchecked()
        }
    }

    #[inline]
    pub unsafe fn swap_remove_unchecked(&mut self, index: usize) -> T {
        if index < self.len() {
            self.swap_remove(index)
        } else if cfg!(debug_assertions) {
            panic!("providing an index >= self.len() is undefined behavior in release")
        } else {
            std::hint::unreachable_unchecked()
        }
    }

    #[inline]
    pub unsafe fn back_unchecked(&mut self) -> &mut T {
        let idx = self.len() - 1;
        self.get_unchecked_mut(idx)
    }
}

impl<T: Copy> FVec<T> {
    #[inline]
    pub unsafe fn extend_unchecked(&mut self, slice: &[T]) {
        debug_assert!(
            !self.next_n_pushes_allocates(slice.len()),
            "attempt to `extend_non_empty_unchecked` when there is not enough existing free space \
             to do so"
        );
        let slice_len = slice.len();
        let len = self.len();
        let new_len = len + slice_len;
        self.data.set_len(new_len);
        slice
            .as_ptr()
            .copy_to_nonoverlapping(self.get_unchecked_mut(len), slice_len);
    }
}

impl<T> IntoIterator for FVec<T> {
    type IntoIter = std::vec::IntoIter<T>;
    type Item = T;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.data.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a FVec<T> {
    type IntoIter = std::slice::Iter<'a, T>;
    type Item = &'a T;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut FVec<T> {
    type IntoIter = std::slice::IterMut<'a, T>;
    type Item = &'a mut T;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

// used by tests
#[allow(unused)]
macro_rules! fvec {
    ($($es:expr),* $(,)*) => {
        {
            let mut v = $crate::internal::alloc::fvec::FVec::<_>::new();
            $(v.push($es);)*
            v
        }
    };
}
