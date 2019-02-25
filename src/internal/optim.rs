#[doc(hidden)]
#[inline(always)]
pub unsafe fn _assume(b: bool) {
    std::intrinsics::assume(b)
}

#[doc(hidden)]
#[inline(always)]
pub fn _unlikely(b: bool) -> bool {
    unsafe { std::intrinsics::unlikely(b) }
}

#[inline(always)]
#[doc(hidden)]
pub fn _likely(b: bool) -> bool {
    unsafe { std::intrinsics::likely(b) }
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

macro_rules! unreach {
    ($($t:tt)*) => {{
        if cfg!(debug_assertions) {
            unreachable!($($t)*)
        } else {
            std::hint::unreachable_unchecked()
        }
    }};
}

#[doc(hidden)]
pub struct AssumeNoPanic([(); 0]);

impl AssumeNoPanic {
    #[doc(hidden)]
    #[inline(always)]
    pub const unsafe fn begin() -> Self {
        AssumeNoPanic([(); 0])
    }

    #[doc(hidden)]
    #[inline(always)]
    pub fn end(self) {
        std::mem::forget(self)
    }
}

impl Drop for AssumeNoPanic {
    #[doc(hidden)]
    #[inline(always)]
    fn drop(&mut self) {
        unsafe { unreach!("unexpected panic during `assume_no_panic`") }
    }
}

macro_rules! assume_no_panic {
    ($($t:tt)*) => {
        {
            let _no_panic = $crate::internal::optim::AssumeNoPanic::begin();
            let result = {
                $($t)*
            };
            $crate::internal::optim::AssumeNoPanic::end(_no_panic);
            result
        }
    };
}
