use core::ops::{Deref, DerefMut};

const START_SIZE: usize = 0;

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
        debug_assert!(
            self.len().checked_add(n).is_some(),
            "overflow in `next_n_pushes_allocates`"
        );
        self.capacity() < self.len() + n
    }

    #[inline]
    pub unsafe fn push_unchecked(&mut self, t: T) {
        if !self.next_push_allocates() {
            let len = self.len();
            self.set_len(len + 1);
            core::ptr::write(self.get_unchecked_mut(len), t);
        } else if cfg!(debug_assertions) {
            panic!("`push_unchecked` called when an allocation is required")
        } else {
            core::hint::unreachable_unchecked()
        }
    }

    #[inline]
    pub unsafe fn pop_unchecked(&mut self) -> T {
        if !self.is_empty() {
            let len = self.len() - 1;
            self.set_len(len);
            core::ptr::read(self.get_unchecked_mut(len))
        } else if cfg!(debug_assertions) {
            panic!("`FVec::pop_unchecked` called on an empty FVec")
        } else {
            core::hint::unreachable_unchecked()
        }
    }

    #[inline]
    pub unsafe fn swap_remove_unchecked(&mut self, index: usize) -> T {
        if index < self.len() {
            self.swap_remove(index)
        } else if cfg!(debug_assertions) {
            panic!("providing an index >= self.len() is undefined behavior in release")
        } else {
            core::hint::unreachable_unchecked()
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
    type IntoIter = core::slice::Iter<'a, T>;
    type Item = &'a T;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut FVec<T> {
    type IntoIter = core::slice::IterMut<'a, T>;
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

#[cfg(test)]
mod test {
    use super::FVec;

    #[test]
    fn next_push_allocates() {
        let v = FVec::<usize>::with_capacity(0);
        assert!(v.next_push_allocates());
        assert!(!v.next_n_pushes_allocates(0));
        assert!(v.next_n_pushes_allocates(1));
        assert!(v.next_n_pushes_allocates(100));

        let mut v = FVec::<usize>::with_capacity(1);
        assert!(!v.next_push_allocates());
        assert!(!v.next_n_pushes_allocates(1));
        assert!(v.next_n_pushes_allocates(2));
        assert!(v.next_n_pushes_allocates(100));

        v.push(0);
        assert!(v.next_push_allocates());
        assert!(!v.next_n_pushes_allocates(0));
        assert!(v.next_n_pushes_allocates(1));
        assert!(v.next_n_pushes_allocates(100));
    }

    #[test]
    fn push_unchecked() {
        let mut v = FVec::<usize>::with_capacity(1);
        assert_eq!(v.len(), 0);
        unsafe { v.push_unchecked(0) };
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn pop_unchecked() {
        let mut v = FVec::<usize>::with_capacity(1);
        v.push(42);
        assert_eq!(v.len(), 1);
        let fourty_two = unsafe { v.pop_unchecked() };
        assert_eq!(fourty_two, 42);
        assert_eq!(v.len(), 0);
    }

    #[test]
    fn swap_remove_unchecked() {
        let mut v = FVec::<usize>::with_capacity(0);
        v.push(42);
        v.push(43);
        v.push(44);
        v.push(45);
        unsafe {
            let removed = v.swap_remove_unchecked(1);
            assert_eq!(removed, 43);
            assert_eq!(v[0], 42);
            assert_eq!(v[1], 45);
            assert_eq!(v[2], 44);

            let removed = v.swap_remove_unchecked(2);
            assert_eq!(removed, 44);
            assert_eq!(v[0], 42);
            assert_eq!(v[1], 45);
        }
    }

    #[test]
    fn back_unchecked() {
        let mut v = FVec::<usize>::with_capacity(0);
        v.push(42);
        v.push(43);
        v.push(44);
        v.push(45);
        unsafe {
            assert_eq!(*v.back_unchecked(), 45);
            v.pop();
            assert_eq!(*v.back_unchecked(), 44);
            v.pop();
            assert_eq!(*v.back_unchecked(), 43);
            v.pop();
            assert_eq!(*v.back_unchecked(), 42);
        }
    }

    #[test]
    fn extend_unchecked() {
        let mut v = FVec::<usize>::with_capacity(32);
        unsafe {
            v.extend_unchecked(&[42; 32]);
            assert_eq!(v.len(), 32);
            assert_eq!(v.capacity(), 32);
            for x in &v {
                assert_eq!(*x, 42);
            }

            v.extend_unchecked(&[42; 0]);
            assert_eq!(v.len(), 32);
            assert_eq!(v.capacity(), 32);
            for x in &v {
                assert_eq!(*x, 42);
            }

            v.clear();
            v.extend_unchecked(&[42; 0]);
            assert_eq!(v.len(), 0);
            assert_eq!(v.capacity(), 32);
        }
    }
}
