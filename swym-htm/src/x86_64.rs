//! x86_64 hardware intrinsics

cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86")] {
        use core::arch::x86 as x86;
    } else if #[cfg(target_arch = "x86_64")] {
        use core::arch::x86_64 as x86;
    }
}
use x86::{
    _xabort, _xbegin, _xend, _xtest, _XABORT_CONFLICT, _XABORT_EXPLICIT, _XABORT_RETRY,
    _XBEGIN_STARTED,
};

#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub(super) struct BeginCode(u32);

impl BeginCode {
    #[inline]
    pub fn is_started(&self) -> bool {
        self.0 == _XBEGIN_STARTED
    }

    #[inline]
    pub fn is_explicit_abort(&self) -> bool {
        self.0 & _XABORT_EXPLICIT != 0
    }

    #[inline]
    pub fn is_retry(&self) -> bool {
        self.0 & _XABORT_RETRY != 0
    }

    #[inline]
    pub fn is_conflict(&self) -> bool {
        self.0 & _XABORT_CONFLICT != 0
    }

    #[inline]
    pub fn is_capacity(&self) -> bool {
        self.0 & _XABORT_CONFLICT != 0
    }
}

#[repr(transparent)]
#[derive(PartialEq, Eq, Ord, PartialOrd, Copy, Clone, Debug, Hash)]
pub(super) struct TestCode(u8);

impl TestCode {
    #[inline]
    pub fn in_transaction(&self) -> bool {
        self.0 != 0
    }

    #[inline]
    pub fn is_suspended(&self) -> bool {
        false
    }
}

#[target_feature(enable = "rtm")]
#[inline]
pub(super) unsafe fn begin() -> BeginCode {
    BeginCode(_xbegin())
}

#[target_feature(enable = "rtm")]
#[inline]
pub(super) unsafe fn abort() -> ! {
    _xabort(0);
    core::hint::unreachable_unchecked();
}

#[target_feature(enable = "rtm")]
#[inline]
pub(super) unsafe fn test() -> TestCode {
    TestCode(_xtest())
}

#[target_feature(enable = "rtm")]
#[inline]
pub(super) unsafe fn end() {
    _xend()
}

#[inline]
pub(super) fn htm_supported() -> bool {
    is_x86_feature_detected!("rtm")
}
