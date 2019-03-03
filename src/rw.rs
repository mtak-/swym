//! Per-Object TL2 algorithm is used:
//! https://www.cs.tau.ac.il/~shanir/nir-pubs-web/Papers/Transactional_Locking.pdf
//!
//! The main difference is the addition of epoch based reclamation.
//! Another sublte difference is a change to when the global clock is bumped. By doing it after
//! TCells have had their value updated, but before releasing their locks, we can simplify reads.
//! Reads don't have to read the per object epoch _before_ and after loading the value from shared
//! memory. They only have to read the per object epoch after loading the value.

use crate::{
    internal::{
        alloc::dyn_vec::DynElemMut,
        epoch::QuiesceEpoch,
        tcell_erased::TCellErased,
        thread::{RWThreadKey, TxLogs},
        write_log::{bloom_hash, Contained, Entry, WriteEntryImpl},
    },
    tcell::TCell,
    tx::{self, Error, Ordering, SetError, Write, _TValue},
};
use std::{
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    ptr::{self, NonNull},
    sync::atomic::Ordering::{self as AtomicOrdering, Acquire, Relaxed},
};

#[derive(Clone, Copy, Debug)]
struct RWTxImpl {
    thread_key: RWThreadKey,
}

impl RWTxImpl {
    #[inline]
    fn new(thread_key: RWThreadKey) -> Self {
        RWTxImpl { thread_key }
    }

    /// Contains the read, write, and garbage logs.
    #[inline]
    fn logs(self) -> NonNull<TxLogs> {
        self.thread_key.tx_logs()
    }

    /// The epoch the transaction has pinned.
    #[inline]
    fn pin_epoch(self) -> QuiesceEpoch {
        self.thread_key.pin_epoch()
    }

    #[inline]
    fn rw_valid(self, erased: &TCellErased, o: AtomicOrdering) -> bool {
        self.pin_epoch()
            .read_write_valid_lockable(&erased.current_epoch, o)
    }

    #[inline(never)]
    #[cold]
    unsafe fn get_slow<T>(self, tcell: &TCell<T>) -> Result<ManuallyDrop<T>, Error> {
        let logs = &mut *self.logs().as_ptr();
        let found = logs.write_log.find(&tcell.erased);
        match found {
            None => {
                let value = tcell.erased.read_acquire::<T>();
                if likely!(self.rw_valid(&tcell.erased, Acquire)) {
                    logs.read_log.push(&tcell.erased);
                    return Ok(value);
                }
            }
            Some(entry) => {
                let value = entry.read::<T>();
                if likely!(self.rw_valid(&tcell.erased, Relaxed)) {
                    return Ok(value);
                }
            }
        }
        Err(Error::RETRY)
    }

    #[inline]
    unsafe fn get_impl<T>(self, tcell: &TCell<T>) -> Result<ManuallyDrop<T>, Error> {
        let logs = &mut *self.logs().as_ptr();
        if likely!(!logs.read_log.next_push_allocates())
            && likely!(logs.write_log.contained(bloom_hash(&tcell.erased)) == Contained::No)
        {
            let value = tcell.erased.read_acquire::<T>();
            if likely!(self.rw_valid(&tcell.erased, Acquire)) {
                logs.read_log.push_unchecked(&tcell.erased);
                return Ok(value);
            }
        }
        self.get_slow(tcell)
    }

    #[inline(never)]
    #[cold]
    unsafe fn get_unlogged_slow<T>(self, tcell: &TCell<T>) -> Result<ManuallyDrop<T>, Error> {
        let logs = &*self.logs().as_ptr();
        let found = logs.write_log.find(&tcell.erased);
        match found {
            None => {
                let value = tcell.erased.read_acquire::<T>();
                if likely!(self.rw_valid(&tcell.erased, Acquire)) {
                    return Ok(value);
                }
            }
            Some(entry) => {
                let value = entry.read::<T>();
                if likely!(self.rw_valid(&tcell.erased, Relaxed)) {
                    return Ok(value);
                }
            }
        }
        Err(Error::RETRY)
    }

    #[inline]
    unsafe fn get_unlogged_impl<T>(self, tcell: &TCell<T>) -> Result<ManuallyDrop<T>, Error> {
        let logs = &*self.logs().as_ptr();
        if likely!(logs.write_log.contained(bloom_hash(&tcell.erased)) == Contained::No) {
            let value = tcell.erased.read_acquire::<T>();
            if likely!(self.rw_valid(&tcell.erased, Acquire)) {
                return Ok(value);
            }
        }
        self.get_unlogged_slow(tcell)
    }

    #[inline(never)]
    #[cold]
    unsafe fn set_slow<T: 'static + Send, V: _TValue<T>>(
        self,
        tcell: &TCell<T>,
        value: V,
    ) -> Result<(), SetError<T>> {
        let logs = &mut *self.logs().as_ptr();
        match logs.write_log.entry(&tcell.erased) {
            Entry::Vacant { write_log, hash } => {
                if likely!(self.rw_valid(&tcell.erased, Relaxed)) {
                    write_log.push(&tcell.erased, value, hash);
                    if mem::needs_drop::<T>() {
                        logs.garbage.trash(tcell.erased.read_relaxed::<T>())
                    }
                    return Ok(());
                }
            }
            Entry::Occupied { mut entry, hash } => {
                if V::REQUEST_TCELL_LIFETIME {
                    entry.deactivate();
                    logs.write_log.push(&tcell.erased, value, hash);
                } else {
                    DynElemMut::assign_unchecked(entry, WriteEntryImpl::new(&tcell.erased, value))
                }
                return Ok(());
            }
        };

        let casted = mem::transmute_copy(&value);
        mem::forget(value);
        Err(SetError {
            value: casted,
            error: Error::RETRY,
        })
    }

    #[inline]
    unsafe fn set_impl<T: Send + 'static, V: _TValue<T>>(
        self,
        tcell: &TCell<T>,
        value: V,
    ) -> Result<(), SetError<T>> {
        let logs = &mut *self.logs().as_ptr();
        let hash = bloom_hash(&tcell.erased);

        if likely!(!logs.write_log.next_push_allocates::<V>())
            && (!mem::needs_drop::<T>() || likely!(!logs.garbage.next_trash_allocates::<T>()))
            && likely!(logs.write_log.contained(hash) == Contained::No)
            && likely!(self.rw_valid(&tcell.erased, Relaxed))
        {
            logs.write_log.push_unchecked(&tcell.erased, value, hash);
            if mem::needs_drop::<T>() {
                logs.garbage
                    .trash_unchecked(tcell.erased.read_relaxed::<T>())
            }
            Ok(())
        } else {
            self.set_slow(tcell, value)
        }
    }
}

/// A read write transaction.
//
// No instances of this type are ever created. References to values of this type are created by
// transmuting RWTxImpl's.
pub struct RWTx<'tcell>(PhantomData<fn(&'tcell ())>);
impl<'tcell> !Send for RWTx<'tcell> {}
impl<'tcell> !Sync for RWTx<'tcell> {}

impl<'tcell> RWTx<'tcell> {
    #[inline]
    pub(crate) fn new<'a>(thread_key: RWThreadKey) -> &'a mut Self {
        unsafe { mem::transmute(RWTxImpl::new(thread_key)) }
    }

    #[inline]
    fn as_impl(&self) -> RWTxImpl {
        unsafe { mem::transmute(self) }
    }
}

unsafe impl<'tcell> tx::Read<'tcell> for RWTx<'tcell> {
    #[inline]
    unsafe fn _get_unchecked<T>(
        &self,
        tcell: &TCell<T>,
        ordering: Ordering,
    ) -> Result<ManuallyDrop<T>, Error> {
        match ordering {
            Ordering::ReadWrite => self.as_impl().get_impl(tcell),
            Ordering::Read => self.as_impl().get_unlogged_impl(tcell),
        }
    }
}

unsafe impl<'tcell> Write<'tcell> for RWTx<'tcell> {
    #[inline]
    unsafe fn _set_unchecked<T: Send + 'static>(
        &self,
        tcell: &TCell<T>,
        value: impl _TValue<T>,
    ) -> Result<(), SetError<T>> {
        self.as_impl().set_impl(tcell, value)
    }

    #[inline]
    fn _privatize<F: FnOnce() + Copy + Send + 'static>(&self, privatizer: F) {
        unsafe {
            self.as_impl()
                .logs()
                .as_mut()
                .garbage
                .trash(ManuallyDrop::new(After::new(privatizer, |p| p())));
        }
    }
}

struct After<T, F: FnOnce(T)> {
    t: ManuallyDrop<T>,
    f: ManuallyDrop<F>,
}

impl<T, F: FnOnce(T)> Drop for After<T, F> {
    fn drop(&mut self) {
        unsafe {
            ManuallyDrop::into_inner(ptr::read(&self.f))(ManuallyDrop::into_inner(ptr::read(
                &self.t,
            )))
        }
    }
}

impl<T, F: FnOnce(T)> After<T, F> {
    #[inline]
    const fn new(t: T, f: F) -> Self {
        After {
            t: ManuallyDrop::new(t),
            f: ManuallyDrop::new(f),
        }
    }
}
