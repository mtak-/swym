//! Utilities for creating always present thread locals.
//!
//! `phoenix_tls` combines everything in this module to create a `thread_local!` style variable that
//! is lazily initialized. If the `thread_local` is accessed after it's destroyed, a new temporary
//! will be created using the same initialization expression (that's where the phoenix comes from).
//!
//! #[thread_local] is used to cache a pointer to the underlying thread_local! to improve access
//! times for the fast path.
//!
//! All phoenix thread locals (Phoenix) are internally reference counted heap allocated structures.
//!
//! Additionally the user type receives two callbacks `subscribe`/`unsubscribe`, which are invoked
//! at creation/desctruction. The address is stable between those two calls.

use std::{
    cell::Cell,
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::Deref,
    ptr::{self, NonNull},
};

/// The #[thread_local] type used to improve access times on platforms that support it.
pub struct FastTls<T> {
    fast_ptr: Cell<Option<NonNull<T>>>,
}

impl<T> FastTls<T> {
    /// Creates a new zero initialized FastTls var.
    #[inline]
    pub const fn none() -> Self {
        FastTls {
            fast_ptr: Cell::new(None),
        }
    }

    /// Initializes the pointer to the thread local.
    ///
    /// Note: Does nothing if #[thread_local] is not supported.
    #[inline]
    fn initialize(&self, value: &T) {
        if cfg!(target_thread_local) {
            debug_assert!(
                self.fast_ptr.get().is_none(),
                "attempted to have two phoenix TLS vars at once"
            );
            self.fast_ptr.set(Some(value.into()))
        }
    }

    /// Gets the pointer to the actual thread local.
    ///
    /// Note: Returns None if #[thread_local] is not supported.
    #[inline]
    fn get(&self) -> Option<NonNull<T>> {
        if cfg!(target_thread_local) {
            self.fast_ptr.get()
        } else {
            None
        }
    }

    /// Clears the pointer to the thread local.
    #[inline]
    fn clear(&self, value: NonNull<T>) {
        if cfg!(target_thread_local) {
            debug_assert!(self.fast_ptr.get().is_some(), "double free on tls var");
            debug_assert!(
                ptr::eq(self.fast_ptr.get().unwrap().as_ptr(), value.as_ptr()),
                "clearing tls var that is not set correctly"
            );
            self.fast_ptr.set(None)
        }
    }
}

/// Types that can be stored in phoenix_tls's can implement this for optional callback hooks for
/// when they are created/destroyed.
///
/// A `Self` lives at the address passed into subscribe until unsubscribe is called.
pub trait PhoenixTarget {
    fn subscribe(&mut self);
    fn unsubscribe(&mut self);
}

/// Unfortunately this is required to get access to the actual thread local at destruction time.
///
/// Having FastTls is probably unnecessary if std's thread_local were more optimized.
pub trait PhoenixTlsApply: Sized {
    type Item: PhoenixTarget;

    /// Applies a function to the inner #[thread_local]
    fn apply_fast_tls<F: FnOnce(&FastTls<Self::Item>) -> O, O>(f: F) -> O;

    /// Creates a new `Phoenix<Self>` without assigning the #[thread_local]
    fn init() -> Phoenix<Self::Item, Self>;
}

#[derive(Debug)]
#[repr(C)]
struct PhoenixImpl<T> {
    value:     T,
    ref_count: Cell<usize>,
}

#[derive(Debug)]
pub struct Phoenix<T: 'static + PhoenixTarget, D: PhoenixTlsApply<Item = T>> {
    raw:     NonNull<PhoenixImpl<T>>,
    phantom: PhantomData<(PhoenixImpl<T>, D)>,
}

impl<T: 'static + PhoenixTarget, D: PhoenixTlsApply<Item = T>> Clone for Phoenix<T, D> {
    #[inline]
    fn clone(&self) -> Self {
        let count = self.as_ref().ref_count.get();
        debug_assert!(count > 0, "attempt to clone a deallocated `Phoenix`");
        self.as_ref().ref_count.set(count + 1);
        Phoenix {
            raw:     self.raw,
            phantom: PhantomData,
        }
    }
}

impl<T: 'static + PhoenixTarget, D: PhoenixTlsApply<Item = T>> Drop for Phoenix<T, D> {
    #[inline]
    fn drop(&mut self) {
        let count = self.as_ref().ref_count.get();
        debug_assert!(count > 0, "double free on `Phoenix` attempted");
        if likely!(count != 1) {
            self.as_ref().ref_count.set(count - 1)
        } else {
            // this is safe as long as the reference counting logic is safe
            unsafe {
                dealloc::<_, D>(self.raw);
            }

            #[inline(never)]
            #[cold]
            unsafe fn dealloc<T: 'static + PhoenixTarget, D: PhoenixTlsApply<Item = T>>(
                mut this_ptr: NonNull<PhoenixImpl<T>>,
            ) {
                this_ptr.as_mut().value.unsubscribe();
                D::apply_fast_tls(move |tls| tls.clear((&this_ptr.as_ref().value).into()));
                drop(Box::from_raw(this_ptr.as_ptr()));
            }
        }
    }
}

impl<T: 'static + PhoenixTarget, D: PhoenixTlsApply<Item = T>> Phoenix<T, D> {
    #[inline]
    pub fn new(value: T) -> Self {
        let mut phoenix = Box::new(PhoenixImpl {
            value,
            ref_count: Cell::new(1),
        });
        phoenix.value.subscribe();
        let raw = Box::into_raw_non_null(phoenix);
        D::apply_fast_tls(move |tls| {
            let phoenix = Phoenix {
                raw,
                phantom: PhantomData,
            };
            tls.initialize(&phoenix);
            phoenix
        })
    }

    #[inline]
    unsafe fn clone_raw(raw: NonNull<T>) -> Self {
        let result = ManuallyDrop::new(Phoenix {
            raw:     raw.cast::<PhoenixImpl<T>>(),
            phantom: PhantomData,
        });
        (*result).clone()
    }

    #[inline]
    fn as_ref(&self) -> &PhoenixImpl<T> {
        // this is safe as long as the reference counting logic is safe
        unsafe { self.raw.as_ref() }
    }
}

impl<T: 'static + PhoenixTarget, D: PhoenixTlsApply<Item = T>> Deref for Phoenix<T, D> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.as_ref().value
    }
}

#[derive(Copy, Clone, Debug)]
pub struct PhoenixKey<T: 'static + PhoenixTarget, D: PhoenixTlsApply<Item = T>> {
    phantom: PhantomData<(PhoenixImpl<T>, D)>,
}

impl<T: 'static + PhoenixTarget, D: PhoenixTlsApply<Item = T>> PhoenixKey<T, D> {
    #[inline]
    pub const fn new() -> Self {
        PhoenixKey {
            phantom: PhantomData,
        }
    }

    #[inline]
    pub fn get(self) -> Phoenix<T, D> {
        D::apply_fast_tls(|tls| match tls.get() {
            Some(phoenix_ptr) => unsafe { Phoenix::clone_raw(phoenix_ptr) },
            None => D::init(),
        })
    }
}

macro_rules! phoenix_tls {
    // empty (base case for the recursion)
    () => {};

    // process multiple declarations
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => (
        __phoenix_tls_inner!($(#[$attr])* $vis $name, $t, $init);
        phoenix_tls!($($rest)*);
    );

    // handle a single declaration
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr) => (
        __phoenix_tls_inner!($(#[$attr])* $vis $name, $t, $init);
    );
}

macro_rules! __phoenix_tls_inner {
    (@key $(#[$attr:meta])* $vis:vis $name:ident, $t:ty, $init:expr) => {
        {
            impl $crate::internal::phoenix_tls::PhoenixTlsApply for $name {
                type Item = $t;

                #[inline]
                fn apply_fast_tls<F: FnOnce(&$crate::internal::phoenix_tls::FastTls<Self::Item>) -> O, O>(f: F) -> O {
                    #[cfg(target_thread_local)]
                    #[thread_local]
                    static TLS: $crate::internal::phoenix_tls::FastTls<$t> = $crate::internal::phoenix_tls::FastTls::none();

                    #[cfg(not(target_thread_local))]
                    const TLS: $crate::internal::phoenix_tls::FastTls<$t> = $crate::internal::phoenix_tls::FastTls::none();

                    f(&TLS)
                }

                #[cfg_attr(target_thread_local, inline(never))]
                #[cfg_attr(target_thread_local, cold)]
                #[cfg_attr(not(target_thread_local), inline)]
                fn init() -> $crate::internal::phoenix_tls::Phoenix<Self::Item, Self> {
                    thread_local!{
                        static TLS: $crate::internal::phoenix_tls::Phoenix<$t, $name>
                            = $crate::internal::phoenix_tls::Phoenix::new($init);
                    }
                    TLS.try_with(|tls| {
                        tls.clone()
                    }).unwrap_or_else(|_| {
                        $crate::internal::phoenix_tls::Phoenix::new($init)
                    })
                }
            }

            $crate::internal::phoenix_tls::PhoenixKey::new()
        }
    };
    ($(#[$attr:meta])* $vis:vis $name:ident, $t:ty, $init:expr) => {
        #[allow(non_camel_case_types)]
        $vis enum $name {}
        $(#[$attr])* $vis const $name: $crate::internal::phoenix_tls::PhoenixKey<
            $t,
            $name,
        > = __phoenix_tls_inner!(@key $(#[$attr])* $vis $name, $t, $init);
    }
}
