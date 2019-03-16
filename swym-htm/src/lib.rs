#![feature(link_llvm_intrinsics)]
#![feature(test)]

extern crate test;

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "x86_64")]
use x86_64 as back;

#[cfg(not(target_arch = "x86_64"))]
pub mod unsupported;

#[cfg(not(target_arch = "x86_64"))]
use unsupported as back;

use std::marker::PhantomData;

#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct BeginCode(back::BeginCode);

impl BeginCode {
    pub const STARTED: Self = BeginCode(back::BeginCode::STARTED);
    pub const RETRY: Self = BeginCode(back::BeginCode::RETRY);
    pub const CONFLICT: Self = BeginCode(back::BeginCode::CONFLICT);
    pub const CAPACITY: Self = BeginCode(back::BeginCode::CAPACITY);
    pub const DEBUG: Self = BeginCode(back::BeginCode::DEBUG);
    pub const NESTED: Self = BeginCode(back::BeginCode::NESTED);

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
    pub unsafe fn begin<F: FnMut(BeginCode) -> bool>(mut retry_handler: F) -> Option<Self> {
        if htm_supported() {
            loop {
                let b = begin();
                if b == BeginCode::STARTED {
                    return Some(HardwareTx {
                        _private: PhantomData,
                    });
                } else if !retry_handler(b) {
                    return None;
                }
            }
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn abort(&self) {
        unsafe { abort() }
    }
}

#[test]
fn begin_end() {
    let mut fails = 0;
    for _ in 0..1000000 {
        unsafe {
            let _tx = HardwareTx::begin(|_| {
                fails += 1;
                true
            })
            .unwrap();
        }
    }
    println!("fails {}", fails);
}

#[test]
fn test_in_transaction() {
    for _ in 0..1000000 {
        unsafe {
            assert!(!test().in_transaction());
            let _tx = HardwareTx::begin(|_| true).unwrap();
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
            let tx = HardwareTx::begin(|code| {
                if code.is_explicit_abort() {
                    abort_count += 1;
                    *i += 1;
                }
                true
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
    let mut fail_count = 0;
    for max in 0..end {
        fail_count = 0;
        unsafe {
            let capacity = &mut capacity;
            let tx = HardwareTx::begin(|mut code| {
                let cap = code == BeginCode::CAPACITY;
                if cap {
                    fail_count += 1;
                }
                !cap || fail_count < 1000
            });
            let tx = match tx {
                Some(tx) => tx,
                None => break,
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
