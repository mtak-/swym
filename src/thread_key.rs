//! Thread local state, [`thread_key::ThreadKey`], used to run transactions.
//!
//! A handle to the thread local state can be acquired by calling [`thread_key::get`].
use crate::{internal::thread::ThreadKeyInner, read::ReadTx, rw::RWTx, tx::Error};
use std::{
    fmt::{self, Debug, Formatter},
    thread::AccessError,
};

/// A handle to `swym`'s thread local state.
///
/// `ThreadKey` can be acquired by calling [`get`].
///
/// `ThreadKey`'s encapsulate the state required to perform transactions, and provides the necessary
/// methods for running transactions.
#[derive(Clone, Debug)]
pub struct ThreadKey {
    thread: ThreadKeyInner,
}

impl ThreadKey {
    #[inline(never)]
    #[cold]
    fn new() -> Self {
        ThreadKey {
            thread: ThreadKeyInner::new(),
        }
    }

    #[inline]
    fn as_raw(&self) -> &ThreadKeyInner {
        &self.thread
    }

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
        F: FnMut(&mut RWTx<'tcell>) -> Result<O, Error>,
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
    #[inline]
    pub fn try_read<'tcell, F, O>(&'tcell self, f: F) -> Result<O, TryReadErr>
    where
        F: FnMut(&ReadTx<'tcell>) -> Result<O, Error>,
    {
        let raw = self.as_raw();
        unsafe {
            if likely!(!raw.is_active()) {
                Ok(raw.read_slow(f))
            } else {
                Err(TryReadErr::new())
            }
        }
    }

    /// Performs a transaction capabable of reading and writing.
    ///
    /// # Errors
    ///
    /// Returns a [`TryRWErr`] if a transaction is already running on the current thread.
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
    #[inline]
    pub fn try_rw<'tcell, F, O>(&'tcell self, f: F) -> Result<O, TryRWErr>
    where
        F: FnMut(&mut RWTx<'tcell>) -> Result<O, Error>,
    {
        let raw = self.as_raw();
        unsafe {
            if likely!(!raw.is_active()) {
                Ok(raw.rw_slow(f))
            } else {
                Err(TryRWErr::new())
            }
        }
    }
}

#[inline(never)]
#[cold]
fn new_thread_key() -> ThreadKey {
    ThreadKey::new()
}

#[inline(never)]
#[cold]
fn err_into_thread_key(_: AccessError) -> ThreadKey {
    new_thread_key()
}

thread_local! {
    static THREAD_KEY: ThreadKey = new_thread_key();
}

#[cfg(not(target_thread_local))]
pub(crate) mod tls {
    use super::*;

    #[inline(never)]
    pub fn thread_key() -> ThreadKey {
        THREAD_KEY
            .try_with(ThreadKey::clone)
            .unwrap_or_else(err_into_thread_key)
    }

    #[inline]
    pub fn clear_tls() {}
}

#[cfg(target_thread_local)]
pub(crate) mod tls {
    use super::{err_into_thread_key, ThreadKey, ThreadKeyInner, THREAD_KEY};
    use std::{cell::Cell, mem, ptr::NonNull};

    #[thread_local]
    static TLS: Cell<Option<NonNull<()>>> = Cell::new(None);

    #[inline]
    pub fn clear_tls() {
        TLS.set(None)
    }

    #[inline(never)]
    #[cold]
    fn thread_key_impl() -> ThreadKey {
        THREAD_KEY
            .try_with(|thread_key| {
                TLS.set(Some(unsafe {
                    mem::transmute_copy::<ThreadKey, _>(thread_key)
                }));
                thread_key.clone()
            })
            .unwrap_or_else(err_into_thread_key)
    }

    #[inline]
    pub fn thread_key() -> ThreadKey {
        match TLS.get() {
            Some(thread) => {
                let thread_key: ThreadKey = unsafe { mem::transmute(thread) };
                mem::forget(thread_key.clone()); // bump ref_count since we created ThreadKey through other means
                thread_key
            }
            None => thread_key_impl(),
        }
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
    tls::thread_key()
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
pub struct TryRWErr {
    _private: (),
}

impl Debug for TryRWErr {
    #[cold]
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.pad("TryRWErr { .. }")
    }
}

impl TryRWErr {
    #[inline]
    fn new() -> Self {
        TryRWErr { _private: () }
    }
}
