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

use std::{cell::Cell, marker::PhantomData, mem::ManuallyDrop, ops::Deref, ptr::NonNull};

/// Types that can be stored in phoenix_tls's can implement this for optional callback hooks for
/// when they are created/destroyed.
///
/// A `Self` lives at the address passed into subscribe until unsubscribe is called.
pub trait PhoenixTarget: Default {
    fn subscribe(&mut self);
    fn unsubscribe(&mut self);
}

#[derive(Debug)]
#[repr(C)]
struct PhoenixImpl<T> {
    value:     T,
    ref_count: Cell<usize>,
}

#[derive(Debug)]
pub struct Phoenix<T: 'static + PhoenixTarget> {
    raw:     NonNull<PhoenixImpl<T>>,
    phantom: PhantomData<PhoenixImpl<T>>,
}

impl<T: 'static + PhoenixTarget> Clone for Phoenix<T> {
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

impl<T: 'static + PhoenixTarget> Drop for Phoenix<T> {
    #[inline]
    fn drop(&mut self) {
        let count = self.as_ref().ref_count.get();
        debug_assert!(count > 0, "double free on `Phoenix` attempted");
        self.as_ref().ref_count.set(count - 1);

        if unlikely!(count == 0) {
            // this is safe as long as the reference counting logic is safe
            unsafe {
                dealloc::<_>(self.raw);
            }

            #[inline(never)]
            #[cold]
            unsafe fn dealloc<T: 'static + PhoenixTarget>(mut this_ptr: NonNull<PhoenixImpl<T>>) {
                this_ptr.as_mut().value.unsubscribe();
                drop(Box::from_raw(this_ptr.as_ptr()));
            }
        }
    }
}

impl<T: 'static + PhoenixTarget> Phoenix<T> {
    #[inline]
    pub fn new(value: T) -> Self {
        let mut phoenix = Box::new(PhoenixImpl {
            value,
            ref_count: Cell::new(1),
        });
        phoenix.value.subscribe();
        let raw = unsafe { NonNull::new_unchecked(Box::into_raw(phoenix)) };
        Phoenix {
            raw,
            phantom: PhantomData,
        }
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
    };
    ($(#[$attr:meta])* $vis:vis $name:ident, $t:ty, $init:expr) => {
        
        $(#[$attr])* $vis const $name: $crate::internal::phoenix_tls::PhoenixKey<
            $t,
            $name,
        > = __phoenix_tls_inner!(@key $(#[$attr])* $vis $name, $t, $init);
    }
}
