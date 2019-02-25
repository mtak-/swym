use std::{
    alloc::{Alloc, AllocErr, CannotReallocInPlace, Excess, GlobalAlloc, Layout},
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
};

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Default, Debug)]
pub struct DebugAlloc<T>(pub T);

#[cfg(test)]
pub fn alloc_count() -> usize {
    ALLOC_COUNT.load(Relaxed)
}

unsafe impl<T: GlobalAlloc> GlobalAlloc for DebugAlloc<T> {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let res = self.0.alloc(layout);
        if likely!(!res.is_null()) {
            ALLOC_COUNT.fetch_add(1, Relaxed);
        }
        res
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOC_COUNT.fetch_sub(1, Relaxed);
        self.0.dealloc(ptr, layout)
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        self.0.realloc(ptr, layout, new_size)
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let res = self.0.alloc_zeroed(layout);
        if likely!(!res.is_null()) {
            ALLOC_COUNT.fetch_add(1, Relaxed);
        }
        res
    }
}

unsafe impl<'a, T: Alloc> Alloc for DebugAlloc<T> {
    #[inline]
    unsafe fn alloc(&mut self, layout: Layout) -> Result<NonNull<u8>, AllocErr> {
        let res = self.0.alloc(layout)?;
        ALLOC_COUNT.fetch_add(1, Relaxed);
        Ok(res)
    }

    #[inline]
    unsafe fn dealloc(&mut self, ptr: NonNull<u8>, layout: Layout) {
        ALLOC_COUNT.fetch_sub(1, Relaxed);
        self.0.dealloc(ptr, layout)
    }

    #[inline]
    fn usable_size(&self, layout: &Layout) -> (usize, usize) {
        self.0.usable_size(layout)
    }

    #[inline]
    unsafe fn realloc(
        &mut self,
        ptr: NonNull<u8>,
        layout: Layout,
        new_size: usize,
    ) -> Result<NonNull<u8>, AllocErr> {
        self.0.realloc(ptr, layout, new_size)
    }

    #[inline]
    unsafe fn alloc_zeroed(&mut self, layout: Layout) -> Result<NonNull<u8>, AllocErr> {
        let res = self.0.alloc_zeroed(layout)?;
        ALLOC_COUNT.fetch_add(1, Relaxed);
        Ok(res)
    }

    #[inline]
    unsafe fn alloc_excess(&mut self, layout: Layout) -> Result<Excess, AllocErr> {
        let res = self.0.alloc_excess(layout)?;
        ALLOC_COUNT.fetch_add(1, Relaxed);
        Ok(res)
    }

    #[inline]
    unsafe fn realloc_excess(
        &mut self,
        ptr: NonNull<u8>,
        layout: Layout,
        new_size: usize,
    ) -> Result<Excess, AllocErr> {
        self.0.realloc_excess(ptr, layout, new_size)
    }

    #[inline]
    unsafe fn grow_in_place(
        &mut self,
        ptr: NonNull<u8>,
        layout: Layout,
        new_size: usize,
    ) -> Result<(), CannotReallocInPlace> {
        self.0.grow_in_place(ptr, layout, new_size)
    }

    #[inline]
    unsafe fn shrink_in_place(
        &mut self,
        ptr: NonNull<u8>,
        layout: Layout,
        new_size: usize,
    ) -> Result<(), CannotReallocInPlace> {
        self.0.shrink_in_place(ptr, layout, new_size)
    }

    #[inline]
    fn alloc_one<U>(&mut self) -> Result<NonNull<U>, AllocErr> {
        let res = self.0.alloc_one::<U>()?;
        ALLOC_COUNT.fetch_add(1, Relaxed);
        Ok(res)
    }

    #[inline]
    unsafe fn dealloc_one<U>(&mut self, ptr: NonNull<U>) {
        ALLOC_COUNT.fetch_sub(1, Relaxed);
        self.0.dealloc_one(ptr)
    }

    #[inline]
    fn alloc_array<U>(&mut self, n: usize) -> Result<NonNull<U>, AllocErr> {
        let res = self.0.alloc_array::<U>(n)?;
        ALLOC_COUNT.fetch_add(1, Relaxed);
        Ok(res)
    }

    #[inline]
    unsafe fn realloc_array<U>(
        &mut self,
        ptr: NonNull<U>,
        n_old: usize,
        n_new: usize,
    ) -> Result<NonNull<U>, AllocErr> {
        self.0.realloc_array::<U>(ptr, n_old, n_new)
    }

    #[inline]
    unsafe fn dealloc_array<U>(&mut self, ptr: NonNull<U>, n: usize) -> Result<(), AllocErr> {
        ALLOC_COUNT.fetch_sub(1, Relaxed);
        self.0.dealloc_array::<U>(ptr, n)
    }
}
