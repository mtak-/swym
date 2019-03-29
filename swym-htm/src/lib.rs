#![feature(link_llvm_intrinsics)]
#![feature(test)]

extern crate test;

#[cfg_attr(
    all(target_arch = "powerpc64", feature = "htm"),
    path = "./powerpc64.rs"
)]
#[cfg_attr(all(target_arch = "x86_64", feature = "rtm"), path = "./x86_64.rs")]
#[cfg_attr(
    not(any(
        all(target_arch = "x86_64", feature = "rtm"),
        all(target_arch = "powerpc64", feature = "htm")
    )),
    path = "./unsupported.rs"
)]
pub mod back;

pub mod htm_usize;

use std::marker::PhantomData;

#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct BeginCode(back::BeginCode);

impl BeginCode {
    #[inline]
    pub fn is_started(&self) -> bool {
        self.0.is_started()
    }

    #[inline]
    pub fn is_explicit_abort(&self) -> bool {
        self.0.is_explicit_abort()
    }

    #[inline]
    pub fn is_retry(&self) -> bool {
        self.0.is_retry()
    }

    #[inline]
    pub fn is_conflict(&self) -> bool {
        self.0.is_conflict()
    }

    #[inline]
    pub fn is_capacity(&self) -> bool {
        self.0.is_capacity()
    }
}

#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct TestCode(back::TestCode);

impl TestCode {
    #[inline]
    pub fn in_transaction(&self) -> bool {
        self.0.in_transaction()
    }

    #[inline]
    pub fn is_suspended(&self) -> bool {
        self.0.is_suspended()
    }
}

#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct AbortCode(back::AbortCode);

impl AbortCode {
    #[inline]
    pub fn new(code: i8) -> Self {
        AbortCode(back::AbortCode::new(code))
    }
}

#[inline]
pub unsafe fn begin() -> BeginCode {
    BeginCode(back::begin())
}

#[inline]
pub unsafe fn abort() -> ! {
    back::abort()
}

#[inline]
pub unsafe fn test() -> TestCode {
    TestCode(back::test())
}

#[inline]
pub unsafe fn end() {
    back::end()
}

#[inline]
pub const fn htm_supported() -> bool {
    back::htm_supported()
}

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
    #[inline]
    pub unsafe fn begin<F, E>(mut retry_handler: F) -> Result<Self, E>
    where
        F: FnMut(BeginCode) -> Result<(), E>,
    {
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

    #[inline(always)]
    pub fn abort(&self) {
        unsafe { abort() }
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
                        let tx = HardwareTx::begin(|_| -> Result<(), ()> { Err(()) });
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
bench_tx! {bench_tx0512, 512}
bench_tx! {bench_tx1024, 1024}
bench_tx! {bench_tx2048, 2048}
bench_tx! {bench_tx4096, 4096}
bench_tx! {bench_tx8192, 8192}

#[bench]
fn bench_abort(bench: &mut test::Bencher) {
    const ITER_COUNT: usize = 1_000_000;

    bench.iter(|| {
        for _ in 0..ITER_COUNT {
            unsafe {
                let tx = HardwareTx::begin(|code| -> Result<(), ()> {
                    if code.is_explicit_abort() {
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
            let _tx = HardwareTx::begin(|_| -> Result<(), ()> {
                fails += 1;
                Ok(())
            })
            .unwrap();
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
            let _tx = HardwareTx::begin(|_| -> Result<(), ()> { Ok(()) }).unwrap();
            assert!(test().in_transaction());
        }
    }
}

#[test]
fn begin_abort() {
    let mut i = 0i32;
    let mut abort_count = 0;
    loop {
        unsafe {
            let i = &mut i;
            *i += 1;
            let tx = HardwareTx::begin(|code| -> Result<(), ()> {
                if code.is_explicit_abort() {
                    abort_count += 1;
                    *i += 1;
                }
                Ok(())
            })
            .unwrap();
            if *i % 128 != 0 && *i != 1_000_000 {
                tx.abort();
            }
        }
        if i == 1_000_000 {
            break;
        }
    }
    assert_eq!(abort_count, 992187);
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
            let tx = HardwareTx::begin(|code| {
                let cap = code.is_capacity();
                if cap {
                    fail_count += 1;
                }
                if !cap || fail_count < 1000 {
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
