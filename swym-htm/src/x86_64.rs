mod intrinsics {
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
    intrinsics::xbegin()
}

#[inline]
pub unsafe fn xend() {
    intrinsics::xend()
}

#[inline]
pub unsafe fn xabort(a: i8) -> ! {
    intrinsics::xabort(a);
    std::hint::unreachable_unchecked()
}

#[inline]
pub unsafe fn xtest() -> i32 {
    intrinsics::xtest()
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

#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct BeginCode(i32);

impl BeginCode {
    pub const STARTED: Self = BeginCode(_XBEGIN_STARTED);
    pub const RETRY: Self = BeginCode(_XABORT_RETRY);
    pub const CONFLICT: Self = BeginCode(_XABORT_CONFLICT);
    pub const CAPACITY: Self = BeginCode(_XABORT_CAPACITY);
    pub const DEBUG: Self = BeginCode(_XABORT_DEBUG);
    pub const NESTED: Self = BeginCode(_XABORT_NESTED);

    #[inline]
    pub fn is_explicit(&self) -> bool {
        self.0 & _XABORT_EXPLICIT != 0
    }

    #[inline]
    pub fn abort_code(&self) -> Option<AbortCode> {
        if self.is_explicit() {
            Some(AbortCode(_XABORT_CODE(self.0) as _))
        } else {
            None
        }
    }
}

#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct TestCode(i32);

#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub struct AbortCode(i8);

#[inline]
pub unsafe fn begin() -> BeginCode {
    BeginCode(xbegin())
}

#[inline]
pub unsafe fn abort(a: AbortCode) -> ! {
    xabort(a.0)
}

#[inline]
pub unsafe fn test() -> TestCode {
    TestCode(xtest())
}

#[inline]
pub unsafe fn end() {
    xend()
}

#[inline]
pub const fn htm_supported() -> bool {
    true
}
