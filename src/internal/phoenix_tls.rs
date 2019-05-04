//! Utilities for creating always present thread locals.
//!
//! `phoenix_tls` combines everything in this module to create a `thread_local!` style variable that
//! is lazily initialized. If the `thread_local` is accessed after it's destroyed, a new temporary
//! will be created using the same initialization expression (that's where the phoenix comes from).
//!
//! All phoenix thread locals (Phoenix) are internally reference counted heap allocated structures.
//!
//! Additionally the user type receives two callbacks `subscribe`/`unsubscribe`, which are invoked
//! at creation/desctruction. The address is stable between those two calls.

use core::{cell::Cell, marker::PhantomData, mem::ManuallyDrop, ops::Deref, ptr::NonNull};

/// Types that can be stored in phoenix_tls's can implement this for optional callback hooks for
/// when they are created/destroyed.
///
/// A `Self` lives at the address passed into subscribe until unsubscribe is called.
pub trait PhoenixTarget: Default {
    #[inline]
    fn subscribe(&mut self) {}

    #[inline]
    fn unsubscribe(&mut self) {}
}

#[derive(Debug)]
#[repr(C)]
struct PhoenixImpl<T> {
    value:     T,
    ref_count: Cell<usize>,
    clear_tls: Option<fn()>,
}

#[derive(Debug)]
pub struct Phoenix<T: 'static + PhoenixTarget> {
    raw:     NonNull<PhoenixImpl<T>>,
    phantom: PhantomData<PhoenixImpl<T>>,
}

impl<T: 'static + PhoenixTarget> Default for Phoenix<T> {
    #[inline(never)]
    #[cold]
    fn default() -> Self {
        Phoenix::new(None)
    }
}

impl<T: 'static + PhoenixTarget> Clone for Phoenix<T> {
    #[inline]
    fn clone(&self) -> Self {
        let count = self.as_ref().ref_count.get();
        debug_assert!(count > 0, "attempt to clone a deallocated `Phoenix`");

        let new_count = count + 1;
        self.as_ref().ref_count.set(new_count);

        // We must check for overflow because users can mem::forget(x.clone())
        // repeatedly.
        if unlikely!(new_count == usize::max_value()) {
            abort!()
        }

        Phoenix {
            raw:     self.raw,
            phantom: PhantomData,
        }
    }
}

impl<T: 'static + PhoenixTarget> Drop for Phoenix<T> {
    #[inline]
    fn drop(&mut self) {
        let count = self.as_ref().ref_count.get();
        debug_assert!(count > 0, "double free on `Phoenix` attempted");
        self.as_ref().ref_count.set(count - 1);

        if unlikely!(count == 1) {
            // this is safe as long as the reference counting logic is safe
            unsafe {
                dealloc::<_>(self.raw);
            }

            #[inline(never)]
            #[cold]
            unsafe fn dealloc<T: 'static + PhoenixTarget>(this_ptr: NonNull<PhoenixImpl<T>>) {
                let mut this = Box::from_raw(this_ptr.as_ptr());

                // Must clear out any cached tls pointer before accessing T mutably; otherwise,
                // there would be potential aliasing issues if the unsubscribe/drop attempts to read
                // from the thread local.
                let _: Option<()> = this.clear_tls.map(|f| f());

                this.value.unsubscribe();
            }
        }
    }
}

impl<T: 'static + PhoenixTarget> Phoenix<T> {
    #[inline(never)]
    #[cold]
    pub fn new(clear_tls: Option<fn()>) -> Self {
        let mut phoenix = Box::new(PhoenixImpl {
            value: T::default(),
            ref_count: Cell::new(1),
            clear_tls,
        });
        phoenix.value.subscribe();
        let raw = unsafe { NonNull::new_unchecked(Box::into_raw(phoenix)) };
        Phoenix {
            raw,
            phantom: PhantomData,
        }
    }

    #[inline]
    pub unsafe fn clone_raw(raw: NonNull<T>) -> Self {
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

impl<T: 'static + PhoenixTarget> Deref for Phoenix<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.as_ref().value
    }
}

macro_rules! phoenix_tls {
    // empty (base case for the recursion)
    () => {};

    // process multiple declarations
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty; $($rest:tt)*) => (
        phoenix_tls!{
            $(#[$attr])* $vis static $name: $t
        }
        phoenix_tls!($($rest)*);
    );

    // handle a single declaration
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty) => (
        #[allow(non_camel_case_types)]
        $vis struct $name;

        impl $name {
            #[inline]
            $vis fn get(self) -> $crate::internal::phoenix_tls::Phoenix<$t> {
                self.with(|x| unsafe {
                    $crate::internal::phoenix_tls::Phoenix::clone_raw(x.into())
                })
            }

            #[inline]
            $vis fn with<F: FnOnce(&$t) -> O, O>(self, f: F) -> O {
                thread_local!{
                    $(#[$attr])* $vis static __SLOW: $crate::internal::phoenix_tls::Phoenix<$t> =
                        $crate::internal::phoenix_tls::Phoenix::new(Some(|| with(|x| x.set(None))));
                }

                #[inline(never)]
                #[cold]
                unsafe fn init<F: FnOnce(&$t) -> O, O>(f: F) -> O {
                    match __SLOW.try_with(|x| {
                        let result = (&**x).into();
                        with(|x| x.set(Some(result)));
                        result
                    }).ok() {
                        Some(nn) => f(nn.as_ref()),
                        None => f(&*$crate::internal::phoenix_tls::Phoenix::<$t>::default())
                    }
                }

                // TLS access through POD is faster. Access through #[thread_local] POD is even faster.
                cfg_if::cfg_if!{
                    if #[cfg(all(feature = "nightly", target_thread_local))] {
                        #[thread_local]
                        $(#[$attr])* $vis static $name: core::cell::Cell<Option<core::ptr::NonNull<$t>>> =
                            core::cell::Cell::new(None);

                        #[inline]
                        fn with<F: FnOnce(&core::cell::Cell<Option<core::ptr::NonNull<$t>>>) -> O, O>(f: F) -> O {
                            f(&$name)
                        }
                    } else {
                        thread_local!{
                            $(#[$attr])* $vis static $name: core::cell::Cell<Option<core::ptr::NonNull<$t>>> =
                                core::cell::Cell::new(None);
                        }

                        #[inline]
                        fn with<F: FnOnce(&core::cell::Cell<Option<core::ptr::NonNull<$t>>>) -> O, O>(f: F) -> O {
                            $name.with(f)
                        }
                    }
                }

                with(|x| unsafe {
                    match x.get() {
                        Some(v) => f(v.as_ref()),
                        None => init(f),
                    }
                })
            }
        }
    );
}
