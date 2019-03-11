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
        tcell_erased::TCellErased,
        thread::{PinRef, PinRw},
        write_log::{bloom_hash, Contained, Entry, WriteEntryImpl},
    },
    tcell::TCell,
    tx::{self, Error, Ordering, SetError, Write, _TValue},
};
use std::{
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    ptr,
    sync::atomic::Ordering::{self as AtomicOrdering, Acquire, Relaxed},
};

#[derive(Debug)]
struct RWTxImpl<'tx, 'tcell> {
    pin_ref: PinRef<'tx, 'tcell>,
}

impl<'tx, 'tcell> std::ops::Deref for RWTxImpl<'tx, 'tcell> {
    type Target = PinRef<'tx, 'tcell>;

    #[inline]
    fn deref(&self) -> &PinRef<'tx, 'tcell> {
        &self.pin_ref
    }
}

impl<'tx, 'tcell> std::ops::DerefMut for RWTxImpl<'tx, 'tcell> {
    #[inline]
    fn deref_mut(&mut self) -> &mut PinRef<'tx, 'tcell> {
        &mut self.pin_ref
    }
}

impl<'tx, 'tcell> RWTxImpl<'tx, 'tcell> {
    #[inline]
    fn new(pin_rw: &'tx mut PinRw<'_, 'tcell>) -> Self {
        RWTxImpl {
            pin_ref: pin_rw.reborrow(),
        }
    }

    #[inline]
    fn rw_valid(&self, erased: &TCellErased, o: AtomicOrdering) -> bool {
        self.pin_epoch()
            .read_write_valid_lockable(&erased.current_epoch, o)
    }

    #[inline(never)]
    #[cold]
    unsafe fn get_slow<T>(mut self, tcell: &TCell<T>) -> Result<ManuallyDrop<T>, Error> {
        let logs = self.logs();
        let found = logs.write_log.find(&tcell.erased);
        match found {
            None => {
                let value = tcell.erased.read_acquire::<T>();
                if likely!(self.rw_valid(&tcell.erased, Acquire)) {
                    self.logs_mut().read_log.push(&tcell.erased);
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
    unsafe fn get_impl<T>(mut self, tcell: &TCell<T>) -> Result<ManuallyDrop<T>, Error> {
        let logs = self.logs();
        if likely!(!logs.read_log.next_push_allocates())
            && likely!(logs.write_log.contained(bloom_hash(&tcell.erased)) == Contained::No)
        {
            let value = tcell.erased.read_acquire::<T>();
            if likely!(self.rw_valid(&tcell.erased, Acquire)) {
                self.logs_mut().read_log.push_unchecked(&tcell.erased);
                return Ok(value);
            }
        }
        self.get_slow(tcell)
    }

    #[inline(never)]
    #[cold]
    unsafe fn get_unlogged_slow<T>(self, tcell: &TCell<T>) -> Result<ManuallyDrop<T>, Error> {
        let logs = self.logs();
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
        let logs = self.logs();
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
        mut self,
        tcell: &TCell<T>,
        value: V,
    ) -> Result<(), SetError<T>> {
        match self.logs_mut().write_log.entry(&tcell.erased) {
            Entry::Vacant { write_log: _, hash } => {
                if likely!(self.rw_valid(&tcell.erased, Relaxed)) {
                    let logs = self.logs_mut();
                    logs.write_log.push(&tcell.erased, value, hash);
                    if mem::needs_drop::<T>() {
                        logs.garbage.trash(tcell.erased.read_relaxed::<T>())
                    }
                    return Ok(());
                }
            }
            Entry::Occupied { mut entry, hash } => {
                if V::REQUEST_TCELL_LIFETIME {
                    entry.deactivate();
                    self.logs_mut().write_log.push(&tcell.erased, value, hash);
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
        mut self,
        tcell: &TCell<T>,
        value: V,
    ) -> Result<(), SetError<T>> {
        let logs = self.logs();
        let hash = bloom_hash(&tcell.erased);

        if likely!(!logs.write_log.next_push_allocates::<V>())
            && (!mem::needs_drop::<T>() || likely!(!logs.garbage.next_trash_allocates::<T>()))
            && likely!(logs.write_log.contained(hash) == Contained::No)
            && likely!(self.rw_valid(&tcell.erased, Relaxed))
        {
            let logs = self.logs_mut();
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
    pub(crate) fn new<'tx>(pin_rw: &'tx mut PinRw<'_, 'tcell>) -> &'tx mut Self {
        unsafe { mem::transmute(RWTxImpl::new(pin_rw)) }
    }

    #[inline]
    fn as_impl(&self) -> RWTxImpl<'_, 'tcell> {
        unsafe { mem::transmute(self) }
    }
}

unsafe impl<'tcell> tx::Read<'tcell> for RWTx<'tcell> {
    #[inline]
    unsafe fn _get_unchecked<T>(
        &self,
        tcell: &'tcell TCell<T>,
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
        &mut self,
        tcell: &'tcell TCell<T>,
        value: impl _TValue<T>,
    ) -> Result<(), SetError<T>> {
        self.as_impl().set_impl(tcell, value)
    }

    #[inline]
    fn _privatize<F: FnOnce() + Copy + Send + 'static>(&self, privatizer: F) {
        self.as_impl()
            .logs_mut()
            .garbage
            .trash(ManuallyDrop::new(After::new(privatizer, |p| p())));
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
