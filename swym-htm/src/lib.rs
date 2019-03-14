#![feature(link_llvm_intrinsics)]

pub mod x86_64;

use std::mem;

#[derive(Debug)]
pub struct HardwareTx {
    _private: (),
}

impl Drop for HardwareTx {
    #[inline]
    fn drop(&mut self) {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            crate::x86_64::xend()
        }
    }
}

impl HardwareTx {
    #[inline]
    pub unsafe fn begin<F: FnMut(i32) -> bool>(mut retry_handler: F) -> Option<Self> {
        #[cfg(target_arch = "x86_64")]
        loop {
            let b = crate::x86_64::xbegin();
            if b == crate::x86_64::_XBEGIN_STARTED {
                return Some(HardwareTx { _private: () });
            } else if !retry_handler(b) {
                return None;
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        unimplemented!()
    }

    #[inline]
    pub fn abort(self, code: i8) {
        mem::forget(self);

        #[cfg(target_arch = "x86_64")]
        unsafe {
            crate::x86_64::xabort(code)
        }
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
