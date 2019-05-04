#[inline(always)]
#[cfg(feature = "nightly")]
pub unsafe fn _assume(b: bool) {
    std::intrinsics::assume(b)
}

#[inline(always)]
#[cfg(feature = "nightly")]
pub fn _unlikely(b: bool) -> bool {
    // not actually unsafe to say a bool is probably false
    unsafe { std::intrinsics::unlikely(b) }
}

#[inline(always)]
#[cfg(feature = "nightly")]
pub fn _likely(b: bool) -> bool {
    // not actually unsafe to say a bool is probably true
    unsafe { std::intrinsics::likely(b) }
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
