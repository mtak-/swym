//! Thread local state, [`thread_key::ThreadKey`], used to run transactions.
//!
//! A handle to the thread local state can be acquired by calling [`thread_key::get`].
use crate::{
    internal::{
        epoch::QuiesceEpoch,
        gc::GlobalSynchList,
        thread::{DecRefCountResult, Thread, ThreadKeyRaw},
    },
    read::ReadTx,
    rw::RWTx,
    tx::Error,
};
use std::{
    fmt::{self, Debug, Formatter},
    ptr::NonNull,
    sync::atomic::Ordering::Release,
    thread::AccessError,
};

/// A handle to `swym`'s thread local state.
///
/// `ThreadKey` can be acquired by calling [`get`].
///
/// `ThreadKey`'s encapsulate the state required to perform transactions, and provides the necessary
/// methods for running transactions.
pub struct ThreadKey {
    thread: ThreadKeyRaw,
}

impl ThreadKey {
    #[inline(never)]
    #[cold]
    fn new() -> Self {
        let thread = Box::new(Thread::new());
        unsafe {
            GlobalSynchList::instance().write().register(&thread.synch);
            let thread = ThreadKeyRaw::new(NonNull::new_unchecked(Box::into_raw(thread)));
            ThreadKey { thread }
        }
    }

    #[inline(never)]
    #[cold]
    unsafe fn unregister(&self) {
        let synch = self.thread.synch();
        synch
            .as_ref()
            .current_epoch
            .set(QuiesceEpoch::end_of_time(), Release);
        self.thread
            .tx_logs()
            .as_mut()
            .garbage
            .synch_and_collect_all(self.thread.synch().as_ref());
        synch
            .as_ref()
            .current_epoch
            .set(QuiesceEpoch::inactive(), Release);

        tls::clear_tls();
        GlobalSynchList::instance_unchecked()
            .write()
            .unregister(synch);
        drop(Box::from_raw(self.thread.thread().as_ptr()))
    }

    #[inline]
    pub(crate) fn as_raw(&self) -> ThreadKeyRaw {
        ThreadKeyRaw::new(self.thread.thread())
    }

    /// Performs a transaction capabable of only reading.
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
    /// Panics if there is already a running transaction.
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
            if likely!(!raw.synch().as_mut().current_epoch.is_active_unsync()) {
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
            if likely!(!raw.synch().as_mut().current_epoch.is_active_unsync()) {
                Ok(raw.rw_slow(f))
            } else {
                Err(TryRWErr::new())
            }
        }
    }

    /// Performs a transaction capabable of only reading.
    ///
    /// # Safety
    ///
    /// If the thread is currently in a transaction, this results in undefined behavior.
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
    /// unsafe {
    ///     let x_clone =
    ///         thread_key.read_unchecked(|tx| Ok(x.borrow(tx, Default::default())?.to_owned()));
    ///     assert_eq!(x_clone, "not gonna be overwritten");
    /// }
    /// ```
    #[inline]
    pub unsafe fn read_unchecked<'tcell, F, O>(&'tcell self, f: F) -> O
    where
        F: FnMut(&ReadTx<'tcell>) -> Result<O, Error>,
    {
        let raw = self.as_raw();
        debug_assert!(
            !raw.synch().as_mut().current_epoch.is_active_unsync(),
            "`rw_unchecked` called during a transaction",
        );
        raw.read_slow(f)
    }

    /// Performs a transaction capabable of reading and writing.
    ///
    /// # Safety
    ///
    /// If the thread is currently in a transaction, this results in undefined behavior.
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
    /// unsafe {
    ///     let prev_x = thread_key.rw_unchecked(|tx| {
    ///         let prev_x = x.borrow(tx, Default::default())?.to_owned();
    ///         x.set(tx, "overwritten".to_owned())?;
    ///         Ok(prev_x)
    ///     });
    ///     assert_eq!(prev_x, "gonna be overwritten");
    /// }
    /// ```
    #[inline]
    pub unsafe fn rw_unchecked<'tcell, F, O>(&'tcell self, f: F) -> O
    where
        F: FnMut(&mut RWTx<'tcell>) -> Result<O, Error>,
    {
        let raw = self.as_raw();
        debug_assert!(
            !raw.synch().as_mut().current_epoch.is_active_unsync(),
            "`rw_unchecked` called during a transaction",
        );
        raw.rw_slow(f)
    }
}

impl Clone for ThreadKey {
    #[inline]
    fn clone(&self) -> Self {
        unsafe {
            self.thread.inc_ref_count();
            ThreadKey {
                thread: self.as_raw(),
            }
        }
    }
}

impl Drop for ThreadKey {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            if self.thread.dec_ref_count() == DecRefCountResult::DestroyRequested {
                self.unregister()
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
mod tls {
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
mod tls {
    use super::{err_into_thread_key, NonNull, ThreadKey, ThreadKeyRaw, THREAD_KEY};
    use crate::internal::thread::Thread;
    use std::{mem, ptr};

    #[thread_local]
    static mut TLS: *mut Thread = ptr::null_mut();

    #[inline]
    pub fn clear_tls() {
        unsafe {
            TLS = ptr::null_mut();
        }
    }

    #[inline(never)]
    #[cold]
    fn thread_key_impl() -> ThreadKey {
        THREAD_KEY
            .try_with(|thread_key| unsafe {
                TLS = thread_key.as_raw().thread().as_ptr();
                thread_key.clone()
            })
            .unwrap_or_else(err_into_thread_key)
    }

    #[inline]
    pub fn thread_key() -> ThreadKey {
        unsafe {
            let tls = TLS;
            if likely!(!tls.is_null()) {
                let thread_key = ThreadKey {
                    thread: ThreadKeyRaw::new(NonNull::new_unchecked(tls)),
                };
                mem::forget(thread_key.clone()); // bump ref_count since we created ThreadKey through other means
                thread_key
            } else {
                thread_key_impl()
            }
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
    pub(crate) fn new() -> Self {
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
    pub(crate) fn new() -> Self {
        TryRWErr { _private: () }
    }
}
