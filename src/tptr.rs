//! A transactional memory primitive powering recursive data structures.
//!
//! # Motivation
//!
//! Let's see what happens when we try to build a simple transactional box using only `TCell`.
//!
//! ```compile_fail
//! use swym::{rw, tcell::TCell};
//! let x = TCell::new(Box::new(TCell::new(42)));
//!
//! rw!(|tx| {
//!     let box_ref = x.borrow(tx, Default::default())?;
//!
//!     // error borrowed value does not live long enough
//!     let fourty_two = box_ref.get(tx, Default::default())?;
//!     Ok(())
//! });
//! ```
//!
//! The problem is that data borrowed from any `TCell` (in the above example to outermost `TCell`)
//! may not come from shared memory, but instead the write set - which is speculative. Values in the
//! write set change throughout the course of a transaction, and may be overwritten.
//!
//! `TPtr` is the current experimental workaround akin to a raw pointer, but capable of publishing
//! and privatizing Box's.
//!
//! # EXPERIMENTAL
//!
//! `TPtr` type may have safety issues, or other flaws. The API is subject to change.

use crate::{
    tcell::TCell,
    tx::{Error, Ordering, Read, SetError, Write, _TValue},
};
use std::{mem, ptr};

#[repr(transparent)]
struct Ptr<T>(*const T);

// overly conservative?
unsafe impl<T: Send + Sync> Send for Ptr<T> {}
unsafe impl<T: Send + Sync> Sync for Ptr<T> {}

impl<T> Clone for Ptr<T> {
    #[inline]
    fn clone(&self) -> Self {
        Ptr(self.0)
    }
}

impl<T> Copy for Ptr<T> {}

impl<T> From<*mut T> for Ptr<T> {
    #[inline]
    fn from(ptr: *mut T) -> Self {
        Ptr(ptr)
    }
}

impl<T> From<*const T> for Ptr<T> {
    #[inline]
    fn from(ptr: *const T) -> Self {
        Ptr(ptr)
    }
}

impl<T> Into<*mut T> for Ptr<T> {
    #[inline]
    fn into(self) -> *mut T {
        self.0 as _
    }
}

impl<T> Into<*const T> for Ptr<T> {
    #[inline]
    fn into(self) -> *const T {
        self.0 as _
    }
}

/// Experimental building block for recursive data structures.
///
/// This should be used in places to where single threaded data structures would have
/// `Option<NonNull<T>>/*mut T`, and is similarly low level.
pub struct TPtr<T> {
    ptr: TCell<Ptr<T>>,
}

impl<T> Default for TPtr<T> {
    #[inline]
    fn default() -> Self {
        TPtr::null()
    }
}

impl<T> TPtr<T> {
    /// Constructs a new `TPtr` from the provided pointer.
    #[inline]
    pub const fn new(ptr: *const T) -> Self {
        TPtr {
            ptr: TCell::new(Ptr(ptr)),
        }
    }

    /// Constructs a `TPtr` initialized to null.
    #[inline]
    pub const fn null() -> Self {
        Self::new(ptr::null_mut())
    }

    /// Consumes the `TPtr` returning the underlying pointer.
    #[inline]
    pub fn into_inner(self) -> *const T {
        self.ptr.into_inner().into()
    }

    /// Gets mutable access to the `TPtr`.
    #[inline]
    pub fn borrow_mut(&mut self) -> &mut *const T {
        unsafe { &mut *(self.ptr.borrow_mut() as *mut Ptr<T> as *mut *const T) }
    }

    /// Retrieve the contained pointer.
    #[inline]
    pub fn as_ptr<'tcell>(
        &'tcell self,
        tx: &impl Read<'tcell>,
        ordering: Ordering,
    ) -> Result<*const T, Error> {
        self.ptr.get(tx, ordering).map(Into::into)
    }
}

impl<T: Send + Sync + 'static> TPtr<T> {
    /// Publishes a new pointer from an owned `Box`.
    ///
    /// # Note
    ///
    /// If the transaction fails at any later point, the memory is freed using `Box::from_raw`. No
    /// cleanup/deallocation of the previously contained pointer is performed.
    #[inline]
    pub fn publish_box<'tcell>(
        &'tcell self,
        tx: &mut impl Write<'tcell>,
        value: Box<T>,
    ) -> Result<(), SetError<Box<T>>> {
        self.publish(
            tx,
            Publisher::new(Box::into_raw(value), |ptr| unsafe {
                drop(Box::from_raw(ptr))
            }),
        )
        .map_err(|err| {
            err.map(|publisher| {
                let ptr = publisher.ptr;
                mem::forget(publisher);
                unsafe { Box::from_raw(ptr) }
            })
        })
    }

    /// Publishes a new pointer.
    ///
    /// # Note
    ///
    /// If the transaction fails at any later point, the desctructor with the pulisher is run.
    /// Publishing No cleanup/deallocation of the previously container pointer is performed.
    #[inline]
    pub fn publish<'tcell, F: FnOnce(*mut T) + Copy + 'static>(
        &'tcell self,
        tx: &mut impl Write<'tcell>,
        publisher: Publisher<T, F>,
    ) -> Result<(), SetError<Publisher<T, F>>> {
        let destructor = publisher.destructor;
        self.ptr.publish(tx, publisher).map_err(|err| {
            err.map(move |ptr| Publisher {
                ptr: ptr.into(),
                destructor,
            })
        })
    }

    /// Queues up drop_in_place/deallocation of the raw pointer to happen at some later time,
    /// if and only if the transaction succeeds.
    ///
    /// # Safety
    ///
    /// The raw pointer must have been previously allocated via `Box::new`, and not yet queued for
    /// privatization or deallocated via other means.
    #[inline]
    pub unsafe fn privatize_as_box<'tcell>(tx: &mut impl Write<'tcell>, value: *const T) {
        let ptr = Ptr(value);
        tx._privatize(move || drop(Box::from_raw(ptr.into())))
    }

    /// Queues up a custom desctructor to happen at some later time, if and only if the transaction
    /// succeeds.
    #[inline]
    pub unsafe fn privatize<'tcell, F: FnOnce(*mut T) + Copy + Send + 'static>(
        tx: &mut impl Write<'tcell>,
        value: *const T,
        privatizer: F,
    ) {
        let ptr = Ptr(value);
        tx._privatize(move || privatizer(ptr.into()))
    }

    /// Sets the contained pointer.
    ///
    /// # Note
    ///
    /// No cleanup/deallocation of the previously contained pointer is performed.
    #[inline]
    pub fn set<'tcell>(
        &'tcell self,
        tx: &mut impl Write<'tcell>,
        value: *const T,
    ) -> Result<(), Error> {
        Ok(self.publish(tx, Publisher::new(value as *mut _, |_| {}))?)
    }
}

pub struct Publisher<T, F: FnOnce(*mut T) + Copy + 'static> {
    ptr:        *mut T,
    destructor: F,
}

impl<T, F: FnOnce(*mut T) + Copy + 'static> Drop for Publisher<T, F> {
    #[inline]
    fn drop(&mut self) {
        (self.destructor)(self.ptr)
    }
}

impl<T: 'static, F: FnOnce(*mut T) + Copy + 'static> Publisher<T, F> {
    #[inline]
    pub fn new(ptr: *mut T, destructor: F) -> Self {
        assert_eq!(
            mem::size_of::<F>(),
            0,
            "Publisher requires the destructor to be zero sized"
        );
        Publisher { ptr, destructor }
    }
}

unsafe impl<T: 'static, F: FnOnce(*mut T) + Copy + 'static> _TValue<Ptr<T>> for Publisher<T, F> {
    const REQUEST_TCELL_LIFETIME: bool = true;
}
