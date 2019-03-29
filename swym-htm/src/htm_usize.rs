use crate::HardwareTx;
use std::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::AtomicUsize,
};

#[derive(Debug)]
#[repr(transparent)]
pub struct HtmUsize {
    inner: UnsafeCell<AtomicUsize>,
}

unsafe impl Send for HtmUsize {}
unsafe impl Sync for HtmUsize {}

impl HtmUsize {
    #[inline]
    pub const fn new(value: usize) -> Self {
        HtmUsize {
            inner: UnsafeCell::new(AtomicUsize::new(value)),
        }
    }

    #[inline(always)]
    fn as_raw(&self, _: &HardwareTx) -> *mut usize {
        self.inner.get() as *mut usize
    }

    #[inline(always)]
    pub fn get(&self, htx: &HardwareTx) -> usize {
        unsafe { *self.as_raw(htx) }
    }

    #[inline(always)]
    pub fn set(&self, htx: &HardwareTx, value: usize) {
        unsafe { *self.as_raw(htx) = value }
    }
}

impl Deref for HtmUsize {
    type Target = AtomicUsize;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.inner.get() }
    }
}

impl DerefMut for HtmUsize {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.inner.get() }
    }
}
