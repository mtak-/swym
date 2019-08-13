//! Hardware transactional memory primitives

#![cfg_attr(feature = "htm", feature(link_llvm_intrinsics))]
#![cfg_attr(feature = "nightly", feature(stdsimd))]
#![cfg_attr(feature = "nightly", feature(rtm_target_feature))]
#![feature(test)]
#![warn(missing_docs)]

#[cfg(test)]
extern crate test;

cfg_if::cfg_if! {
    if #[cfg(all(target_arch = "powerpc64", feature = "htm"))] {
        pub mod powerpc64;
        use powerpc64 as back;
    } else if #[cfg(all(any(target_arch = "x86", target_arch = "x86_64"), feature = "nightly"))] {
        pub mod x86_64;
        use x86_64 as back;
    } else {
        pub mod unsupported;
        use unsupported as back;
    }
}

use core::{
    cell::UnsafeCell,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::atomic::AtomicUsize,
};

/// Returns true if the platform supports hardware transactional memory.
#[inline]
pub fn htm_supported() -> bool {
    back::htm_supported()
}

/// Attempts to begin a hardware transaction.
///
/// Control is returned to the point where begin was called on a failed transaction, only the
/// `BeginCode` now contains the reason for the failure.
///
/// # Safety
///
/// It is unsafe to always retry the transaction after a failure. It is also unsafe to
/// never subsequently call `end`.
#[inline]
pub unsafe fn begin() -> BeginCode {
    BeginCode(back::begin())
}

/// Aborts an in progress hardware transaction.
///
/// # Safety
///
/// There must be an in progress hardware transaction.
#[inline]
pub unsafe fn abort() -> ! {
    back::abort()
}

/// Tests the current transactional state of the thread.
#[inline]
pub unsafe fn test() -> TestCode {
    TestCode(back::test())
}

/// Ends and commits an in progress hardware transaction.
///
/// # Safety
///
/// There must be an in progress hardware transaction.
#[inline]
pub unsafe fn end() {
    back::end()
}

/// The result of calling `begin`.
#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct BeginCode(back::BeginCode);

impl BeginCode {
    /// Returns true if the `BeginCode` represents a successfully started transaction.
    #[inline]
    pub fn is_started(&self) -> bool {
        self.0.is_started()
    }

    /// Returns true if the `BeginCode` represents a transaction that was explicitly `abort`ed.
    #[inline]
    pub fn is_explicit_abort(&self) -> bool {
        self.0.is_explicit_abort()
    }

    /// Returns true if retrying the hardware transaction is suggested.
    #[inline]
    pub fn is_retry(&self) -> bool {
        self.0.is_retry()
    }

    /// Returns true if the transaction aborted due to a memory conflict.
    #[inline]
    pub fn is_conflict(&self) -> bool {
        self.0.is_conflict()
    }

    /// Returns true if the transaction aborted due to running out of capacity.
    ///
    /// Hardware transactions are typically bounded by L1 cache sizes.
    #[inline]
    pub fn is_capacity(&self) -> bool {
        self.0.is_capacity()
    }
}

/// The result of calling `test`.
#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct TestCode(back::TestCode);

impl TestCode {
    /// Returns true if the current thread is in a hardware transaction.
    #[inline]
    pub fn in_transaction(&self) -> bool {
        self.0.in_transaction()
    }

    /// Returns true if the current thread is in a suspended hardware transaction.
    #[inline]
    pub fn is_suspended(&self) -> bool {
        self.0.is_suspended()
    }
}

/// A hardware memory transaction.
///
/// On drop, the transaction is committed.
#[derive(Debug)]
pub struct HardwareTx {
    _private: PhantomData<*mut ()>,
}

impl Drop for HardwareTx {
    #[inline]
    fn drop(&mut self) {
        unsafe { end() }
    }
}

impl HardwareTx {
    /// Starts a new hardware transaction.
    ///
    /// Takes a retry handler which is called on transaction abort. If the retry handler returns
    /// `Ok(())`, the transaction is retried. Any `Err` is passed back to the location where `new`
    /// was called.
    ///
    /// The retry handler is never called with `BeginCode`s where `code.is_started() == true`.
    ///
    /// # Safety
    ///
    /// It is unsafe to pass in a retry handler that never returns `Err`.
    #[inline]
    pub unsafe fn new<F, E>(retry_handler: F) -> Result<Self, E>
    where
        F: FnMut(BeginCode) -> Result<(), E>,
    {
        debug_assert!(
            htm_supported(),
            "Hardware transactional memory is not supported on this target. Check `htm_supported` \
             before attempting a transaction"
        );
        let b = begin();
        if nudge::likely(b.is_started()) {
            return Ok(HardwareTx {
                _private: PhantomData,
            });
        } else {
            #[inline(never)]
            #[cold]
            unsafe fn do_retry<F, E>(mut retry_handler: F, b: BeginCode) -> Result<HardwareTx, E>
            where
                F: FnMut(BeginCode) -> Result<(), E>,
            {
                retry_handler(b)?;
                loop {
                    let b = begin();
                    if b.is_started() {
                        return Ok(HardwareTx {
                            _private: PhantomData,
                        });
                    } else {
                        retry_handler(b)?
                    }
                }
            }

            do_retry(retry_handler, b)
        }
    }

    /// Starts a new hardware transaction with a default bounded retry handler.
    #[inline]
    pub fn bounded(failure_count: &mut u8, max_failures: u8) -> Result<Self, BoundedHtxErr> {
        unsafe {
            HardwareTx::new(move |code| {
                if code.is_explicit_abort() || code.is_conflict() && !code.is_retry() {
                    Err(BoundedHtxErr::AbortOrConflict)
                } else if code.is_retry() && *failure_count < max_failures {
                    *failure_count += 1;
                    Ok(())
                } else {
                    Err(BoundedHtxErr::SoftwareFallback)
                }
            })
        }
    }

    /// Aborts the current transaction.
    ///
    /// Aborting a hardware transaction will effectively reset the thread/call stack to the location
    /// where new was called, passing control to the retry handler.
    ///
    /// Even though this never returns, it does **not** panic.
    #[inline(always)]
    #[cold]
    pub fn abort(&self) -> ! {
        unsafe { abort() }
    }
}

/// Error type for default HardwareTx instances.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum BoundedHtxErr {
    /// The hardware transaction requests a software fallback.
    SoftwareFallback,

    /// The hardware transaction had a conflict or explicit abort.
    AbortOrConflict,
}

/// An atomic and hardware transactional usize.
#[derive(Debug)]
#[repr(transparent)]
pub struct HtmUsize {
    inner: UnsafeCell<AtomicUsize>,
}

unsafe impl Send for HtmUsize {}
unsafe impl Sync for HtmUsize {}

impl HtmUsize {
    /// Creates a new hardware transactional cell.
    #[inline]
    pub const fn new(value: usize) -> Self {
        HtmUsize {
            inner: UnsafeCell::new(AtomicUsize::new(value)),
        }
    }

    /// # Safety
    ///
    /// This is unsafe because AtomicUsize already allows mutation through immutable reference.
    /// Therefore, the returned mutable reference cannot escape this module.
    #[inline(always)]
    unsafe fn as_raw(&self, _: &HardwareTx) -> &mut AtomicUsize {
        &mut *self.inner.get()
    }

    /// Get the contained value transactionally.
    #[inline(always)]
    pub fn get(&self, htx: &HardwareTx) -> usize {
        unsafe { *self.as_raw(htx).get_mut() }
    }

    /// Set the contained value transactionally.
    #[inline(always)]
    pub fn set(&self, htx: &HardwareTx, value: usize) {
        unsafe { *self.as_raw(htx).get_mut() = value }
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

macro_rules! bench_tx {
    ($name:ident, $count:expr) => {
        #[bench]
        fn $name(bench: &mut test::Bencher) {
            const ITER_COUNT: usize = 1_000_000;
            const WORDS_WRITTEN: usize = $count;

            #[repr(align(4096))]
            struct AlignedArr([usize; WORDS_WRITTEN]);

            let mut arr = AlignedArr([0usize; WORDS_WRITTEN]);

            for (i, elem) in arr.0.iter_mut().enumerate() {
                unsafe { std::ptr::write_volatile(elem, test::black_box(elem.wrapping_add(i))) };
                test::black_box(elem);
            }

            bench.iter(move || {
                for _ in 0..ITER_COUNT {
                    unsafe {
                        let tx = HardwareTx::new(|_| -> Result<(), ()> { Err(()) });
                        for i in 0..arr.0.len() {
                            *arr.0.get_unchecked_mut(i) =
                                arr.0.get_unchecked_mut(i).wrapping_add(1);
                        }
                        drop(tx);
                    }
                }
            });
        }
    };
}

bench_tx! {bench_tx0000, 0}
bench_tx! {bench_tx0001, 1}
bench_tx! {bench_tx0002, 2}
bench_tx! {bench_tx0004, 4}
bench_tx! {bench_tx0008, 8}
bench_tx! {bench_tx0016, 16}
bench_tx! {bench_tx0024, 24}
bench_tx! {bench_tx0032, 32}
bench_tx! {bench_tx0040, 40}
bench_tx! {bench_tx0048, 48}
bench_tx! {bench_tx0056, 56}
bench_tx! {bench_tx0064, 64}
bench_tx! {bench_tx0072, 72}
bench_tx! {bench_tx0080, 80}
bench_tx! {bench_tx0112, 112}
bench_tx! {bench_tx0120, 120}
bench_tx! {bench_tx0128, 128}
bench_tx! {bench_tx0256, 256}

#[bench]
fn bench_abort(bench: &mut test::Bencher) {
    const ITER_COUNT: usize = 1_000_000;

    bench.iter(|| {
        for _ in 0..ITER_COUNT {
            unsafe {
                let mut fail_count = 0;
                let tx = HardwareTx::new(|code| -> Result<(), ()> {
                    fail_count += 1;
                    if code.is_explicit_abort() || fail_count > 3 {
                        Err(())
                    } else {
                        Ok(())
                    }
                });
                drop(tx.map(|tx| tx.abort()));
            }
        }
    });
}

#[test]
fn begin_end() {
    const ITER_COUNT: usize = 1_000_000;

    let mut fails = 0;
    for _ in 0..ITER_COUNT {
        unsafe {
            let mut this_fail_count = 0;
            let tx = HardwareTx::new(|_| -> Result<(), ()> {
                fails += 1;
                this_fail_count += 1;
                if this_fail_count < 4 {
                    Ok(())
                } else {
                    Err(())
                }
            });
            drop(tx);
        }
    }
    println!(
        "fail rate {:.4}%",
        fails as f64 * 100.0 / (ITER_COUNT + fails) as f64
    );
}

#[test]
fn test_in_transaction() {
    for _ in 0..1000000 {
        unsafe {
            assert!(!test().in_transaction());
            let mut fail_count = 0;
            let tx = HardwareTx::new(|_| -> Result<(), ()> {
                fail_count += 1;
                if fail_count < 4 {
                    Ok(())
                } else {
                    Err(())
                }
            });
            if let Ok(_tx) = tx {
                assert!(test().in_transaction());
            } else {
                assert!(!test().in_transaction());
            }
        }
    }
}

#[test]
fn begin_abort() {
    let mut i = 0i32;
    let mut abort_count = 0;
    loop {
        let i = &mut i;
        *i += 1;
        unsafe {
            let mut fail_count = 0;
            let tx = HardwareTx::new(|code| -> Result<(), ()> {
                fail_count += 1;
                *i += 1;
                if code.is_explicit_abort() && fail_count < 4 {
                    abort_count += 1;
                    Ok(())
                } else {
                    Err(())
                }
            });
            if let Ok(tx) = tx {
                if *i % 128 != 0 && *i != 1_000_000 {
                    tx.abort();
                }
            }
        }
        if *i >= 1_000_000 {
            break;
        }
    }
    println!("abort count: {}", abort_count);
}

#[test]
fn capacity_check() {
    use std::mem;

    const CACHE_LINE_SIZE: usize = 64 / mem::size_of::<usize>();

    let mut data = vec![0usize; 1000000];
    let mut capacity = 0;
    let end = data.len() / CACHE_LINE_SIZE;
    for i in (0..end).rev() {
        data[i * CACHE_LINE_SIZE] = data[i * CACHE_LINE_SIZE].wrapping_add(1);
        test::black_box(&mut data[i * CACHE_LINE_SIZE]);
    }
    for max in 0..end {
        let mut fail_count = 0;
        unsafe {
            let tx = HardwareTx::new(|_| {
                fail_count += 1;
                if fail_count < 1000 {
                    Ok(())
                } else {
                    Err(())
                }
            });
            let tx = match tx {
                Ok(tx) => tx,
                Err(()) => break,
            };
            for i in 0..max {
                let elem = data.get_unchecked_mut(i * CACHE_LINE_SIZE);
                *elem = elem.wrapping_add(1);
            }
            drop(tx);
        }
        capacity = max;
    }
    test::black_box(&mut data);
    println!("sum: {}", data.iter().sum::<usize>());
    // println!("Data: {:?}", data);
    println!(
        "Capacity: {}",
        capacity * mem::size_of::<usize>() * CACHE_LINE_SIZE
    );
}

#[test]
fn supported() {
    let supported = htm_supported();
    println!("runtime support check: {}", supported);
}

#[bench]
fn increment_array(b: &mut test::Bencher) {
    const U: HtmUsize = HtmUsize::new(0);
    let x: [HtmUsize; 64] = [
        U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U,
        U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U, U,
        U, U, U, U,
    ];
    b.iter(|| {
        let mut fail_count = 0;
        let tx = unsafe {
            HardwareTx::new(|code| {
                fail_count += 1;
                if code.is_retry() && fail_count < 4 {
                    Ok(())
                } else {
                    Err(())
                }
            })
        };
        match tx {
            Ok(tx) => {
                for elem in x.iter() {
                    elem.set(&tx, elem.get(&tx) + 1)
                }
            }
            Err(_) => {
                for elem in x.iter() {
                    elem.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
    });
    let all = x[0].load(std::sync::atomic::Ordering::Relaxed);
    for elem in x.iter() {
        assert_eq!(elem.load(std::sync::atomic::Ordering::Relaxed), all);
    }
}
