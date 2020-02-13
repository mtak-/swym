#[inline(always)]
#[cfg(feature = "nightly")]
pub unsafe fn _assume(b: bool) {
    core::intrinsics::assume(b)
}

#[inline(always)]
#[cfg(feature = "nightly")]
pub fn _unlikely(b: bool) -> bool {
    core::intrinsics::unlikely(b)
}

#[inline(always)]
#[cfg(feature = "nightly")]
pub fn _likely(b: bool) -> bool {
    core::intrinsics::likely(b)
}

#[inline(always)]
#[cfg(not(feature = "nightly"))]
pub unsafe fn _assume(_: bool) {}

#[inline(always)]
#[cfg(not(feature = "nightly"))]
pub fn _unlikely(b: bool) -> bool {
    b
}

#[inline(always)]
#[cfg(not(feature = "nightly"))]
pub fn _likely(b: bool) -> bool {
    b
}

#[cold]
pub fn _abort() -> ! {
    std::process::abort();
}

macro_rules! assume {
    ($e:expr $(, $t:tt)*) => {
        if cfg!(debug_assertions) {
            assert!($e $(, $t)*)
        } else {
            $crate::internal::optim::_assume($e)
        }
    };
}

macro_rules! unlikely {
    ($e:expr) => {
        $crate::internal::optim::_unlikely($e)
    };
}

macro_rules! likely {
    ($e:expr) => {{
        $crate::internal::optim::_likely($e)
    }};
}

macro_rules! abort {
    () => {
        $crate::internal::optim::_abort()
    };
}
