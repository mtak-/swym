//! Per-Object TL2 algorithm is used:
//! https://www.cs.tau.ac.il/~shanir/nir-pubs-web/Papers/Transactional_Locking.pdf
//!
//! The main difference is the addition of epoch based reclamation.
//! Another subtle difference is a change to when the global clock is bumped. By doing it after
//! TCells have had their value updated, but before releasing their locks, we can simplify reads.
//! Reads don't have to read the per object epoch _before_ and after loading the value from shared
//! memory. They only have to read the per object epoch after loading the value.

use crate::{
    internal::{
        alloc::dyn_vec::DynElemMut,
        tcell_erased::TCellErased,
        thread::{PinMutRef, PinRw},
        write_log::{bloom_hash, Contained, Entry, WriteEntryImpl},
    },
    tcell::{Ref, TCell},
    tx::{self, Error, Ordering, SetError, Write, _TValue},
};
use std::{
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    ptr,
};

#[derive(Debug)]
struct RwTxImpl<'tx, 'tcell> {
    pin_ref: PinMutRef<'tx, 'tcell>,
}

impl<'tx, 'tcell> std::ops::Deref for RwTxImpl<'tx, 'tcell> {
    type Target = PinMutRef<'tx, 'tcell>;

    #[inline]
    fn deref(&self) -> &PinMutRef<'tx, 'tcell> {
        &self.pin_ref
    }
}

impl<'tx, 'tcell> std::ops::DerefMut for RwTxImpl<'tx, 'tcell> {
    #[inline]
    fn deref_mut(&mut self) -> &mut PinMutRef<'tx, 'tcell> {
        &mut self.pin_ref
    }
}

impl<'tx, 'tcell> RwTxImpl<'tx, 'tcell> {
    #[inline]
    fn new(pin_rw: &'tx mut PinRw<'_, 'tcell>) -> Self {
        RwTxImpl {
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
    fn borrow_slow<T>(mut self, tcell: &'tcell TCell<T>) -> Result<Ref<'tx, T>, Error> {
        let logs = self.logs();
        let found = logs.write_log.find(&tcell.erased);
        unsafe {
            match found {
                None => {
                    let value = Ref::new(tcell.erased.optimistic_read_acquire::<T>());
                    if likely!(self.rw_valid(&tcell.erased, Acquire)) {
                        self.logs_mut().read_log.push(&tcell.erased);
                        return Ok(value);
                    }
                }
                Some(entry) => {
                    let value = Ref::new(entry.read::<T>());
                    if likely!(self.rw_valid(&tcell.erased, Relaxed)) {
                        return Ok(value);
                    }
                }
            }
        }
        Err(Error::RETRY)
    }

    #[inline]
    fn borrow_impl<T>(mut self, tcell: &'tcell TCell<T>) -> Result<Ref<'tx, T>, Error> {
        let logs = self.logs();
        if likely!(!logs.read_log.next_push_allocates())
            && likely!(logs.write_log.contained(bloom_hash(&tcell.erased)) == Contained::No)
        {
            unsafe {
                let value = Ref::new(tcell.erased.optimistic_read_acquire::<T>());
                if likely!(self.rw_valid(&tcell.erased, Acquire)) {
                    self.logs_mut().read_log.push_unchecked(&tcell.erased);
                    return Ok(value);
                }
            }
        }

        self.borrow_slow(tcell)
    }

    #[inline(never)]
    #[cold]
    fn borrow_unlogged_slow<T>(self, tcell: &TCell<T>) -> Result<Ref<'tx, T>, Error> {
        let logs = self.logs();
        let found = logs.write_log.find(&tcell.erased);
        unsafe {
            match found {
                None => {
                    let value = Ref::new(tcell.erased.optimistic_read_acquire::<T>());
                    if likely!(self.rw_valid(&tcell.erased, Acquire)) {
                        return Ok(value);
                    }
                }
                Some(entry) => {
                    let value = Ref::new(entry.read::<T>());
                    if likely!(self.rw_valid(&tcell.erased, Relaxed)) {
                        return Ok(value);
                    }
                }
            }
        }
        Err(Error::RETRY)
    }

    #[inline]
    fn borrow_unlogged_impl<T>(self, tcell: &'tcell TCell<T>) -> Result<Ref<'tx, T>, Error> {
        let logs = self.logs();
        if likely!(logs.write_log.contained(bloom_hash(&tcell.erased)) == Contained::No) {
            unsafe {
                let value = Ref::new(tcell.erased.optimistic_read_acquire::<T>());
                if likely!(self.rw_valid(&tcell.erased, Acquire)) {
                    return Ok(value);
                }
            }
        }
        self.borrow_unlogged_slow(tcell)
    }

    #[inline(never)]
    #[cold]
    fn set_slow<T: 'static + Send, V: _TValue<T>>(
        mut self,
        tcell: &'tcell TCell<T>,
        value: V,
    ) -> Result<(), SetError<T>> {
        unsafe {
            match self.logs_mut().write_log.entry(&tcell.erased) {
                Entry::Vacant { write_log: _, hash } => {
                    if likely!(self.rw_valid(&tcell.erased, Relaxed)) {
                        let logs = self.logs_mut();
                        logs.write_log.push(&tcell.erased, value, hash);
                        if mem::needs_drop::<T>() {
                            logs.garbage
                                .trash(tcell.erased.optimistic_read_relaxed::<T>())
                        }
                        return Ok(());
                    }
                }
                Entry::Occupied { mut entry, hash } => {
                    if V::REQUEST_TCELL_LIFETIME {
                        entry.deactivate();
                        self.logs_mut().write_log.push(&tcell.erased, value, hash);
                    } else {
                        DynElemMut::assign_unchecked(
                            entry,
                            WriteEntryImpl::new(&tcell.erased, value),
                        )
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
    }

    #[inline]
    fn set_impl<T: Send + 'static, V: _TValue<T>>(
        mut self,
        tcell: &'tcell TCell<T>,
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
            unsafe {
                logs.write_log.push_unchecked(&tcell.erased, value, hash);
                if mem::needs_drop::<T>() {
                    logs.garbage
                        .trash_unchecked(tcell.erased.optimistic_read_relaxed::<T>())
                }
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
// transmuting RwTxImpl's.
pub struct RwTx<'tcell>(PhantomData<fn(&'tcell ())>);
impl<'tcell> !Send for RwTx<'tcell> {}
impl<'tcell> !Sync for RwTx<'tcell> {}

impl<'tcell> RwTx<'tcell> {
    #[inline]
    pub(crate) fn new<'tx>(pin_rw: &'tx mut PinRw<'_, 'tcell>) -> &'tx mut Self {
        unsafe { mem::transmute(RwTxImpl::new(pin_rw)) }
    }

    #[inline]
    fn as_impl(&self) -> RwTxImpl<'_, 'tcell> {
        unsafe { mem::transmute(self) }
    }
}

impl<'tcell> tx::Read<'tcell> for RwTx<'tcell> {
    #[inline]
    fn borrow<'tx, T>(
        &'tx self,
        tcell: &'tcell TCell<T>,
        ordering: Ordering,
    ) -> Result<Ref<'tx, T>, Error> {
        if mem::size_of::<T>() != 0 {
            match ordering {
                Ordering::ReadWrite => self.as_impl().borrow_impl(tcell),
                Ordering::Read => self.as_impl().borrow_unlogged_impl(tcell),
            }
        } else {
            // If the type is zero sized, there's no need to any synchronization.
            Ok(Ref::new(unsafe { mem::zeroed() }))
        }
    }
}

impl<'tcell> Write<'tcell> for RwTx<'tcell> {
    #[inline]
    fn set<T: Send + 'static>(
        &mut self,
        tcell: &'tcell TCell<T>,
        value: impl _TValue<T>,
    ) -> Result<(), SetError<T>> {
        assert_eq!(
            mem::size_of_val(&value),
            mem::size_of::<T>(),
            "swym currently requires undo callbacks to be zero sized"
        );
        if mem::size_of::<T>() != 0 {
            self.as_impl().set_impl(tcell, value)
        } else {
            // publication/privatization is not public (yet?). so this todo should never fire
            #[inline]
            fn assert_not_tcell_lifetime<T: _TValue<U>, U: 'static>(value: T) {
                assert!(
                    !T::REQUEST_TCELL_LIFETIME,
                    "TODO: publication/privatization of zero sized types"
                );
                drop(value)
            }
            assert_not_tcell_lifetime(value);

            // If the type is zero sized, there's no need to any synchronization.
            Ok(())
        }
    }

    #[inline]
    fn _privatize<F: FnOnce() + Copy + Send + 'static>(&mut self, privatizer: F) {
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
    fn new(t: T, f: F) -> Self {
        After {
            t: ManuallyDrop::new(t),
            f: ManuallyDrop::new(f),
        }
    }
}
