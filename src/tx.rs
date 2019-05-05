//! Functionality for working with transactions.

use crate::tcell::{Ref, TCell};
use core::{
    cell::UnsafeCell,
    fmt::{self, Debug, Formatter},
    ops::{Deref, DerefMut},
};

#[derive(PartialEq, Eq)]
enum ErrorKind {
    Conflict,
    Retry,
}

/// Error type indicating that the transaction has failed.
///
/// It is typical to route this error back to [`ThreadKey::rw`] or [`ThreadKey::read`] where the
/// transaction will be retried, however, this is not required.
///
/// # Notes
///
/// Any additional operations on any [`TCell`] that has returned `Error` will continue to return
/// errors for the remainder of the transaction.
///
/// [`ThreadKey::read`]: ../thread_key/struct.ThreadKey.html#method.read
/// [`ThreadKey::rw`]: ../thread_key/struct.ThreadKey.html#method.rw
#[derive(PartialEq, Eq)]
pub struct Error {
    kind:     ErrorKind,
    _private: (),
}

impl Debug for Error {
    #[cold]
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.pad("Error { .. }")
    }
}

impl<T> From<SetError<T>> for Error {
    #[inline]
    fn from(set_error: SetError<T>) -> Self {
        set_error.error
    }
}

impl Error {
    /// Error value requesting a retry of the current transaction.
    ///
    /// # Notes
    ///
    /// Returning `RETRY` to [`ThreadKey::read`] or [`ThreadKey::rw`] will immediately restart the
    /// transaction. This can cause the thread to spin, hurting the performance of other
    /// threads. In the future, the behavior of `RETRY` may change.
    ///
    /// # Examples
    ///
    /// ```
    /// use swym::{tcell::TCell, thread_key, tx::Error};
    ///
    /// let thread_key = thread_key::get();
    /// let locked = TCell::new(false);
    ///
    /// thread_key.rw(|tx| {
    ///     if locked.get(tx, Default::default())? {
    ///         Err(Error::RETRY)
    ///     } else {
    ///         Ok(locked.set(tx, true)?)
    ///     }
    /// })
    /// ```
    ///
    /// [`ThreadKey::read`]: ../thread_key/struct.ThreadKey.html#method.read
    /// [`ThreadKey::rw`]: ../thread_key/struct.ThreadKey.html#method.rw
    pub const RETRY: Self = Error {
        kind:     ErrorKind::Retry,
        _private: (),
    };

    pub(crate) const CONFLICT: Self = Error {
        kind:     ErrorKind::Conflict,
        _private: (),
    };
}

/// Error type indicating that the transaction has failed to [`set`] a value.
///
/// It is typical to convert this error [`into`] a [`Error`] and route it back to [`ThreadKey::rw`]
/// where the transaction will be retried, however, this is not required.
///
/// # Notes
///
/// Any additional operations on a [`TCell`] that has returned `SetError` will continue to return
/// errors for the remainder of the transaction.
///
/// [`set`]: ../tcell/struct.TCell.html#method.set
/// [`into`]: struct.SetError.html#implementations
/// [`ThreadKey::rw`]: ../thread_key/struct.ThreadKey.html#method.rw
#[derive(Debug)]
pub struct SetError<T> {
    /// The value that was failed to be set.
    pub value: T,

    /// The reason for the transaction failure.
    pub error: Error,
}

impl<T> SetError<T> {
    #[inline]
    pub fn map<F: FnOnce(T) -> U, U>(self, f: F) -> SetError<U> {
        SetError {
            value: f(self.value),
            error: self.error,
        }
    }
}

/// Transactional memory orderings.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ordering {
    /// Ensures that values read were set before the transaction started, and haven't been modified
    /// by another thread before writing the results of the transaction.
    ///
    /// This is the strongest memory Ordering and using this for all reads guarantees transactions
    /// are _serializable_.
    ReadWrite,

    /// Ensures that values read were set before the transaction started.
    ///
    /// Crucially, reads using this `Ordering` are not validated before the transaction writes its
    /// results. `Read` does not guarantee that transactions are serializable; however, certain
    /// algorithms can take advantage of this `Ordering` while still guaranteeing
    /// serializability. This can significantly reduce the number of failed transactions
    /// resulting in noticeable performance improvement under heavy contention.
    ///
    /// # Note
    ///
    /// Profile before even considering this memory ordering. Use of `Ordering::Read` when combined
    /// with only safe code, will always be safe, but it can still cause extremely subtle bugs, as
    /// transactions will no longer behave as though a single global lock were acquired for the
    /// duration of the transaction.
    ///
    /// # Examples
    ///
    /// In the following example, the resulting values for (X,Y) will always be (N, N + 1) under
    /// `Ordering::ReadWrite`. This is relatively easy to tell by glancing at the critical
    /// sections. However, under `Ordering::Read`, new "in-between" results are possible.
    ///
    /// ```
    /// use swym::{tcell::TCell, thread_key, tx::Ordering};
    ///
    /// static X: TCell<i32> = TCell::new(0);
    /// static Y: TCell<i32> = TCell::new(2);
    ///
    /// let thread_key = thread_key::get();
    /// let o = Ordering::ReadWrite; // or Ordering::Read;
    ///
    /// // thread 1
    /// thread_key.rw(|tx| {
    ///     let a = X.get(tx, o)? + 1;
    ///     Y.set(tx, a)?;
    ///     Ok(())
    /// });
    ///
    /// // thread 2
    /// thread_key.rw(|tx| {
    ///     let a = Y.get(tx, o)? - 1;
    ///     X.set(tx, a)?;
    ///     Ok(())
    /// });
    ///
    /// // valid (X,Y) results for o = Ordering::ReadWrite
    /// // (0,1), (1,2)
    ///
    /// // valid (X,Y) results for o = Ordering::Read
    /// // (0,1), (1,2), (1,1)
    /// ```
    Read,

    #[doc(hidden)]
    _NonExhaustive { _private: () },
}

impl Default for Ordering {
    #[inline]
    fn default() -> Self {
        Ordering::ReadWrite
    }
}

/// Trait for types that represent transactions with the ability to read.
///
/// # Notes
///
/// Don't implement this trait.
pub trait Read<'tcell> {
    #[doc(hidden)]
    fn borrow<'tx, T: Borrow>(
        &'tx self,
        tcell: &'tcell TCell<T>,
        ordering: Ordering,
    ) -> Result<Ref<'tx, T>, Error>;
}

/// Trait for types that represent transactions with the ability to write.
///
/// # Notes
///
/// Don't implement this trait.
pub trait Write<'tcell> {
    #[doc(hidden)]
    fn set<T: Send + 'static>(
        &mut self,
        tcell: &'tcell TCell<T>,
        src: impl _TValue<T>,
    ) -> Result<(), SetError<T>>;

    #[doc(hidden)]
    fn _privatize<F: FnOnce() + Copy + Send + 'static>(&mut self, privatizer: F);
}

/// Trait for types that represent transactions with the ability to read and write.
pub trait Rw<'tcell>: Read<'tcell> + Write<'tcell> {}
impl<'tcell, T: Read<'tcell> + Write<'tcell>> Rw<'tcell> for T {}

#[doc(hidden)]
pub unsafe trait _TValue<T: 'static>: 'static {
    const REQUEST_TCELL_LIFETIME: bool;
}
unsafe impl<T: 'static> _TValue<T> for T {
    const REQUEST_TCELL_LIFETIME: bool = false;
}

/// Auto trait for types lacking direct interior mutability.
///
/// These types can have a snapshot (memcpy style) taken of the current state as long as the
/// original value is not dropped. See [`TCell::borrow`].
///
/// The list of manual implementations is conservative, and will likely be expanded in the future.
/// As long as the interior mutability resides on the heap (through a pointer), then the type can
/// manually implement `Borrow`.
pub unsafe auto trait Borrow {}
impl<T: ?Sized> !Borrow for UnsafeCell<T> {}
unsafe impl<T: ?Sized> Borrow for Box<T> {}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub(crate) struct AssertBorrow<T> {
    value: T,
}

unsafe impl<T> Borrow for AssertBorrow<T> {}

impl<T> core::borrow::Borrow<T> for AssertBorrow<T> {
    #[inline]
    fn borrow(&self) -> &T {
        &self.value
    }
}

impl<T> core::borrow::BorrowMut<T> for AssertBorrow<T> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

impl<T> Deref for AssertBorrow<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> DerefMut for AssertBorrow<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}
