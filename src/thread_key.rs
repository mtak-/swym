//! Thread local state, [`thread_key::ThreadKey`], used to run transactions.
//!
//! A handle to the thread local state can be acquired by calling [`thread_key::get`].

use crate::{
    internal::{phoenix_tls::Phoenix, thread::Thread},
    read::ReadTx,
    rw::RwTx,
    tx::{Error, Status},
};
use core::fmt::{self, Debug, Formatter};

/// A handle to `swym`'s thread local state.
///
/// `ThreadKey` can be acquired by calling [`get`].
///
/// `ThreadKey`'s encapsulate the state required to perform transactions, and provides the necessary
/// methods for running transactions.
#[derive(Clone)]
pub struct ThreadKey {
    thread: Phoenix<Thread>,
}

impl Debug for ThreadKey {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.pad("ThreadKey { .. }")
    }
}

impl ThreadKey {
    /// Performs a transaction capabable of only reading.
    ///
    /// # Panics
    ///
    /// Panics if there is already a running transaction on the current thread.
    ///
    /// # Examples
    ///
    /// ```
    /// use swym::{tcell::TCell, thread_key};
    ///
    /// let x = TCell::new(String::from("not gonna be overwritten"));
    ///
    /// let thread_key = thread_key::get();
    ///
    /// let x_clone = thread_key.read(|tx| Ok(x.borrow(tx, Default::default())?.to_owned()));
    /// assert_eq!(x_clone, "not gonna be overwritten");
    /// ```
    #[inline]
    pub fn read<'tcell, F, O>(&'tcell self, f: F) -> O
    where
        F: FnMut(&ReadTx<'tcell>) -> Result<O, Error>,
    {
        self.try_read(f)
            .expect("nested transactions are not yet supported")
    }

    /// Performs a transaction capabable of reading and writing.
    ///
    /// # Panics
    ///
    /// Panics if there is already a running transaction on the current thread.
    ///
    /// # Examples
    ///
    /// ```
    /// use swym::{tcell::TCell, thread_key};
    ///
    /// let x = TCell::new(String::from("gonna be overwritten"));
    ///
    /// let thread_key = thread_key::get();
    ///
    /// let prev_x = thread_key.rw(|tx| {
    ///     let r = x.borrow(tx, Default::default())?.to_owned();
    ///     x.set(tx, "overwritten".to_owned())?;
    ///     Ok(r)
    /// });
    /// assert_eq!(prev_x, "gonna be overwritten");
    /// ```
    #[inline]
    pub fn rw<'tcell, F, O>(&'tcell self, f: F) -> O
    where
        F: FnMut(&mut RwTx<'tcell>) -> Result<O, Status>,
    {
        self.try_rw(f)
            .expect("nested transactions are not yet supported")
    }

    /// Performs a transaction capabable of only reading.
    ///
    /// # Errors
    ///
    /// Returns a [`TryReadErr`] if a transaction is already running on the current thread.
    ///
    /// # Examples
    ///
    /// ```
    /// use swym::{tcell::TCell, thread_key};
    ///
    /// let x = TCell::new(String::from("not gonna be overwritten"));
    ///
    /// let thread_key = thread_key::get();
    ///
    /// let x_clone = thread_key
    ///     .try_read(|tx| Ok(x.borrow(tx, Default::default())?.to_owned()))
    ///     .unwrap();
    /// assert_eq!(x_clone, "not gonna be overwritten");
    /// ```
    #[allow(clippy::redundant_closure)]
    #[inline]
    pub fn try_read<'tcell, F, O>(&'tcell self, f: F) -> Result<O, TryReadErr>
    where
        F: FnMut(&ReadTx<'tcell>) -> Result<O, Error>,
    {
        Ok(self
            .thread
            .try_pin()
            .ok_or_else(|| TryReadErr::new())?
            .run_read(f))
    }

    /// Performs a transaction capabable of reading and writing.
    ///
    /// # Errors
    ///
    /// Returns a [`TryRwErr`] if a transaction is already running on the current thread.
    ///
    /// # Examples
    ///
    /// ```
    /// use swym::{tcell::TCell, thread_key};
    ///
    /// let x = TCell::new(String::from("gonna be overwritten"));
    ///
    /// let thread_key = thread_key::get();
    ///
    /// let prev_x = thread_key
    ///     .try_rw(|tx| {
    ///         let prev_x = x.borrow(tx, Default::default())?.to_owned();
    ///         x.set(tx, "overwritten".to_owned())?;
    ///         Ok(prev_x)
    ///     })
    ///     .unwrap();
    /// assert_eq!(prev_x, "gonna be overwritten");
    /// ```
    #[allow(clippy::redundant_closure)]
    #[inline]
    pub fn try_rw<'tcell, F, O>(&'tcell self, f: F) -> Result<O, TryRwErr>
    where
        F: FnMut(&mut RwTx<'tcell>) -> Result<O, Status>,
    {
        Ok(self
            .thread
            .try_pin()
            .ok_or_else(|| TryRwErr::new())?
            .run_rw(f))
    }
}

mod tls {
    use crate::internal::thread::Thread;

    phoenix_tls! {
        pub static THREAD_KEY: Thread
    }
}

/// Returns a handle to `swym`'s thread local state.
///
/// # Note
///
/// Reusing the same [`ThreadKey`] between transactions is slightly more efficient due to the costs
/// of access thread local memory, and (non-atomic) reference counting.
///
/// # Examples
///
/// ```rust
/// use swym::{tcell::TCell, thread_key, tx::Ordering};
/// let thread_key = thread_key::get();
///
/// let x = TCell::new(0);
///
/// thread_key.rw(|tx| Ok(x.set(tx, 1)?));
///
/// let one = thread_key.read(|tx| Ok(x.get(tx, Ordering::default())?));
///
/// assert_eq!(one, 1);
/// ```
#[inline]
pub fn get() -> ThreadKey {
    ThreadKey {
        thread: tls::THREAD_KEY.get(),
    }
}

/// Error type indicating that the read transaction failed to even start due to nesting.
pub struct TryReadErr {
    _private: (),
}

impl Debug for TryReadErr {
    #[cold]
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.pad("TryReadError { .. }")
    }
}

impl TryReadErr {
    #[inline]
    fn new() -> Self {
        TryReadErr { _private: () }
    }
}

/// Error type indicating that the read-write transaction failed to even start due to nesting.
pub struct TryRwErr {
    _private: (),
}

impl Debug for TryRwErr {
    #[cold]
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.pad("TryRwErr { .. }")
    }
}

impl TryRwErr {
    #[inline]
    fn new() -> Self {
        TryRwErr { _private: () }
    }
}
