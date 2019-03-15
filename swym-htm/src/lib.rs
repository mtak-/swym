#![feature(link_llvm_intrinsics)]

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "x86_64")]
use x86_64 as back;

#[cfg(not(target_arch = "x86_64"))]
pub mod unsupported;

#[cfg(not(target_arch = "x86_64"))]
use unsupported as back;

use std::mem;

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
    pub fn is_explicit(&self) -> bool {
        self.0.is_explicit()
    }

    #[inline]
    pub fn abort_code(&self) -> Option<AbortCode> {
        self.0.abort_code().map(AbortCode)
    }
}

#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct TestCode(back::TestCode);

#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct AbortCode(back::AbortCode);

#[inline]
pub unsafe fn begin() -> BeginCode {
    BeginCode(back::begin())
}

#[inline]
pub unsafe fn abort(abort: AbortCode) -> ! {
    back::abort(abort.0)
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
pub fn htm_supported() -> bool {
    back::htm_supported()
}

#[derive(Debug)]
pub struct HardwareTx {
    _private: (),
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
                    return Some(HardwareTx { _private: () });
                } else if !retry_handler(b) {
                    return None;
                }
            }
        } else {
            None
        }
    }

    #[inline]
    pub fn abort(self, code: AbortCode) {
        mem::forget(self);

        unsafe { abort(code) }
    }
}

#[test]
fn foo() {
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
