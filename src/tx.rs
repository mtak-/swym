//! Functionality for working with transactions.

use crate::tcell::{Ref, TCell};
use core::fmt::{self, Debug, Formatter};
use freeze::Freeze;

#[derive(PartialEq, Eq)]
enum ErrorKind {
    Conflict,
}

/// An error type indicating that the transaction has failed.
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
    kind: ErrorKind,
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
    pub(crate) const CONFLICT: Self = Error {
        kind: ErrorKind::Conflict,
    };
}

/// An error type indicating that the transaction has failed to [`set`] a value.
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

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum InternalStatus {
    Error(Error),
    Retry,
}

/// A type representing that the transaction does not wish to continue.
///
/// `Status` may represent a transaction error, or that the transaction wishes to
/// [`AWAIT_RETRY`](Status::AWAIT_RETRY).
///
/// It is typical to route this back to [`ThreadKey::rw`] where the transaction will be retried,
/// however, this is not required.
///
/// [`ThreadKey::rw`]: ../thread_key/struct.ThreadKey.html#method.rw
#[derive(Debug, PartialEq, Eq)]
pub struct Status {
    pub(crate) kind: InternalStatus,
}

impl From<Error> for Status {
    #[inline]
    fn from(rhs: Error) -> Self {
        Status {
            kind: InternalStatus::Error(rhs),
        }
    }
}

impl<T> From<SetError<T>> for Status {
    #[inline]
    fn from(set_error: SetError<T>) -> Self {
        set_error.error.into()
    }
}

impl Status {
    /// `Status` value requesting a retry of the current transaction after a change to the read set.
    ///
    /// Returning `AWAIT_RETRY` to [`ThreadKey::rw`] will block the thread, until another
    /// transaction successfully modifies a `TCell` in this transactions read set.
    ///
    /// # Examples
    ///
    /// ```
    /// use swym::{tcell::TCell, thread_key, tx::Status};
    ///
    /// let thread_key = thread_key::get();
    /// let locked = TCell::new(false);
    ///
    /// thread_key.rw(|tx| {
    ///     if locked.get(tx, Default::default())? {
    ///         Err(Status::AWAIT_RETRY)
    ///     } else {
    ///         Ok(locked.set(tx, true)?)
    ///     }
    /// })
    /// ```
    ///
    /// # Warning
    ///
    /// `AWAIT_RETRY` introduces the possibility of true deadlocks into `swym`. A program which does
    /// not use `AWAIT_RETRY` will never deadlock - atleast due to `swym`. This is considered
    /// "worth it" because many powerful abstractions can be built upon `AWAIT_RETRY`.
    ///
    /// A transaction which returns `AWAIT_RETRY` with an empty read set is considered a logic
    /// error. In debug builds it will panic, and in release builds it will park the thread forever.
    ///
    /// [`ThreadKey::rw`]: ../thread_key/struct.ThreadKey.html#method.rw
    pub const AWAIT_RETRY: Self = Status {
        kind: InternalStatus::Retry,
    };
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
    /// # Warning
    ///
    /// Profile before even considering this memory ordering. Use of `Ordering::Read` when combined
    /// with only safe code, will always be safe, but it can still cause extremely subtle bugs, as
    /// transactions will no longer behave as though a single global lock were acquired for the
    /// duration of the transaction.
    ///
    /// Additionally, reads using `Ordering::Read` will _not_ be waited on when using
    /// [`AWAIT_RETRY`](crate::tx::Status::AWAIT_RETRY).
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
    fn borrow<'tx, T: Freeze>(
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
