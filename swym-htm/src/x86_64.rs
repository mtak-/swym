#![cfg(target_arch = "x86_64")]

mod ffi {
    extern "C" {
        #[link_name = "llvm.x86.xbegin"]
        pub fn xbegin() -> i32;

        #[link_name = "llvm.x86.xend"]
        pub fn xend() -> ();

        #[link_name = "llvm.x86.xabort"]
        pub fn xabort(_: i8) -> ();

        #[link_name = "llvm.x86.xtest"]
        pub fn xtest() -> i32;
    }
}

#[inline]
pub unsafe fn xbegin() -> i32 {
    ffi::xbegin()
}

#[inline]
pub unsafe fn xend() {
    ffi::xend()
}

#[inline]
pub unsafe fn xabort(a: i8) -> ! {
    ffi::xabort(a);
    std::hint::unreachable_unchecked()
}

#[inline]
pub unsafe fn xtest() -> i32 {
    ffi::xtest()
}

pub const _XBEGIN_STARTED: i32 = !0 as i32;
pub const _XABORT_EXPLICIT: i32 = 1i32 << 0;
pub const _XABORT_RETRY: i32 = 1i32 << 1;
pub const _XABORT_CONFLICT: i32 = 1i32 << 2;
pub const _XABORT_CAPACITY: i32 = 1i32 << 3;
pub const _XABORT_DEBUG: i32 = 1i32 << 4;
pub const _XABORT_NESTED: i32 = 1i32 << 5;

#[allow(non_snake_case)]
#[inline(always)]
pub const fn _XABORT_CODE(x: i32) -> i32 {
    ((x) >> 24) & 0xFFi32
}
