/// Lazy static that has less code bloat.
///
/// Implementation based on fast thread locals in libstd.

use std::sync::atomic::{
    AtomicPtr,
    Ordering::{Acquire, Relaxed},
};

#[derive(Copy, Clone)]
pub struct Fls<T: 'static> {
    init_: fn() -> &'static T,
    get_:  fn() -> &'static AtomicPtr<T>,
}

impl<T: 'static + Sync> Fls<T> {
    #[inline]
    pub const unsafe fn new(
        get_: fn() -> &'static AtomicPtr<T>,
        init_: fn() -> &'static T,
    ) -> Self {
        Fls { init_, get_ }
    }

    #[inline]
    pub fn get(self) -> &'static T {
        let raw = (self.get_)().load(Acquire);
        if likely!(!raw.is_null()) {
            // the singleton is never freed, so once initialized, it is always valid
            unsafe { &*raw }
        } else {
            (self.init_)()
        }
    }

    #[inline]
    pub unsafe fn get_unchecked(self) -> &'static T {
        let raw = (self.get_)().load(Relaxed);
        debug_assert!(
            !raw.is_null(),
            "`Fls::get_unchecked` called before singleton was created"
        );
        &*raw
    }
}

macro_rules! fast_lazy_static {
    // empty (base case for the recursion)
    () => {};

    // process multiple declarations
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => (
        __fast_lazy_static_inner!($(#[$attr])* $vis $name, $t, $init);
        fast_lazy_static!($($rest)*);
    );

    // handle a single declaration
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr) => (
        __fast_lazy_static_inner!($(#[$attr])* $vis $name, $t, $init);
    );
}

macro_rules! __fast_lazy_static_inner {
    (@key $(#[$attr:meta])* $vis:vis $name:ident, $t:ty, $init:expr) => {
        {
            #[inline(always)]
            fn __get() -> &'static std::sync::atomic::AtomicPtr<$t> {
                static __PTR: std::sync::atomic::AtomicPtr<$t> =
                    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
                &__PTR
            }

            #[inline(never)]
            #[cold]
            fn __init() -> &'static $t {
                // Once handles two threads racing to initialize the singleton
                static __INIT_ONCE: std::sync::Once = std::sync::Once::new();

                #[inline(never)]
                #[cold]
                fn do_init() {
                    __get().store(
                        Box::into_raw(Box::new($init)),
                        std::sync::atomic::Ordering::Release,
                    );
                }

                __INIT_ONCE.call_once(do_init);

                unsafe { &*__get().load(std::sync::atomic::Ordering::Relaxed) }
            }

            unsafe {
                $crate::internal::fast_lazy_static::Fls::new(__get, __init)
            }
        }
    };
    ($(#[$attr:meta])* $vis:vis $name:ident, $t:ty, $init:expr) => {
        $(#[$attr])* $vis const $name: $crate::internal::fast_lazy_static::Fls<$t> =
            __fast_lazy_static_inner!(@key $(#[$attr])* $vis $name, $t, $init);
    }
}
