//! Lazy static that has less code bloat.
//!
//! Implementation based on fast thread locals in libstd.

use std::sync::atomic::{
    AtomicPtr,
    Ordering::{Acquire, Relaxed},
};

/// Fast lazy static.
///
/// One of the more subtle overheads with STMs is code bloat. lazy_static! generates a lot of code
/// when accessed. It should just be a simple "if" check with a `call` for the slow path
/// (uninitialized). That's what Fls does.
#[derive(Copy, Clone)]
pub struct Fls<T: 'static> {
    init_: fn() -> &'static T,
    get_:  fn() -> &'static AtomicPtr<T>,
}

impl<T: 'static + Sync> Fls<T> {
    /// Creates a new fast lazy static.
    ///
    /// # Safety
    ///
    /// Once initialized the value should never be destructed or deallocated. The `fast_lazy_static`
    /// macro is a safe wrapper for this.
    #[inline]
    pub const unsafe fn new(
        get_: fn() -> &'static AtomicPtr<T>,
        init_: fn() -> &'static T,
    ) -> Self {
        Fls { init_, get_ }
    }

    /// Returns a reference to the global, initializing it on first access.
    #[inline]
    pub fn get(self) -> &'static T {
        let raw = (self.get_)().load(Acquire);
        if likely!(!raw.is_null()) {
            // the singleton is never freed, so once initialized, it is always valid
            unsafe { &*raw }
        } else {
            // slow path initialize it
            (self.init_)()
        }
    }

    /// Returns a reference to the global, assumes it has already been initialized.
    ///
    /// # Safety
    ///
    /// Requires `get` has been called atleast once.
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

/// Like lazy_static! but is much more friendly to inlining (less code bloat on fast path).
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
                        // matching acquire in `Fls::get`
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
        // const makes the function pointers inlineable
        $(#[$attr])* $vis const $name: $crate::internal::fast_lazy_static::Fls<$t> =
            __fast_lazy_static_inner!(@key $(#[$attr])* $vis $name, $t, $init);
    }
}
