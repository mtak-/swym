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
        alloc::dyn_vec::{self, DynElemMut},
        bloom::Contained,
        tcell_erased::TCellErased,
        thread::{PinMutRef, PinRw},
        write_log::{Entry, WriteEntry, WriteEntryImpl},
    },
    stats,
    tcell::{Ref, TCell},
    tx::{self, Error, Ordering, SetError, Write, _TValue},
};
use core::{
    fmt::{self, Debug, Formatter},
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    ptr,
};

#[derive(Debug)]
struct RwTxImpl<'tx, 'tcell> {
    pin_ref: PinMutRef<'tx, 'tcell>,
}

impl<'tx, 'tcell> core::ops::Deref for RwTxImpl<'tx, 'tcell> {
    type Target = PinMutRef<'tx, 'tcell>;

    #[inline]
    fn deref(&self) -> &PinMutRef<'tx, 'tcell> {
        &self.pin_ref
    }
}

impl<'tx, 'tcell> core::ops::DerefMut for RwTxImpl<'tx, 'tcell> {
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
    fn rw_valid(&self, erased: &TCellErased) -> bool {
        self.pin_epoch()
            .read_write_valid_lockable(&erased.current_epoch)
    }

    #[inline(never)]
    #[cold]
    fn borrow_slow<T>(mut self, tcell: &'tcell TCell<T>) -> Result<Ref<'tx, T>, Error> {
        let logs = self.logs();
        let found = logs.write_log.find(&tcell.erased);
        unsafe {
            match found {
                None => {
                    let value = Ref::new(tcell.optimistic_read_acquire());
                    if likely!(self.rw_valid(&tcell.erased)) {
                        self.logs_mut().read_log.record(&tcell.erased);
                        return Ok(value);
                    }
                }
                Some(entry) => {
                    stats::read_after_write();
                    let value = Ref::new(entry.read::<T>());
                    if likely!(self.rw_valid(&tcell.erased)) {
                        return Ok(value);
                    }
                }
            }
        }
        Err(Error::CONFLICT)
    }

    #[inline]
    fn borrow_impl<T>(mut self, tcell: &'tcell TCell<T>) -> Result<Ref<'tx, T>, Error> {
        let logs = self.logs();
        if likely!(!logs.read_log.next_push_allocates())
            && likely!(logs.write_log.contained(&tcell.erased) == Contained::No)
        {
            unsafe {
                let value = Ref::new(tcell.optimistic_read_acquire());
                if likely!(self.rw_valid(&tcell.erased)) {
                    self.logs_mut().read_log.record_unchecked(&tcell.erased);
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
        let found = logs.write_log.find_skip_filter(&tcell.erased);
        unsafe {
            match found {
                None => {
                    let value = Ref::new(tcell.optimistic_read_acquire());
                    if likely!(self.rw_valid(&tcell.erased)) {
                        return Ok(value);
                    }
                }
                Some(entry) => {
                    stats::read_after_write();
                    let value = Ref::new(entry.read::<T>());
                    if likely!(self.rw_valid(&tcell.erased)) {
                        return Ok(value);
                    }
                }
            }
        }
        Err(Error::CONFLICT)
    }

    #[inline]
    fn borrow_unlogged_impl<T>(self, tcell: &'tcell TCell<T>) -> Result<Ref<'tx, T>, Error> {
        let logs = self.logs();
        if likely!(logs.write_log.contained(&tcell.erased) == Contained::No) {
            unsafe {
                let value = Ref::new(tcell.optimistic_read_acquire());
                if likely!(self.rw_valid(&tcell.erased)) {
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
                Entry::Vacant => {
                    if likely!(self.rw_valid(&tcell.erased)) {
                        let logs = self.logs_mut();
                        let replaced = logs.write_log.record_update(&tcell.erased, value);
                        debug_assert!(!replaced);
                        if mem::needs_drop::<T>() {
                            logs.garbage.dispose(tcell.optimistic_read_relaxed())
                        }
                        return Ok(());
                    }
                }
                Entry::Occupied { mut entry } => {
                    if V::REQUEST_TCELL_LIFETIME {
                        entry.deactivate();
                        let replaced = self
                            .logs_mut()
                            .write_log
                            .record_update(&tcell.erased, value);
                        debug_assert!(replaced);
                    } else {
                        let new_entry = WriteEntryImpl::new(&tcell.erased, value);
                        let new_vtable = dyn_vec::vtable::<dyn WriteEntry + 'tcell>(&new_entry);
                        DynElemMut::assign_unchecked(entry, new_vtable, new_entry)
                    }
                    return Ok(());
                }
            };

            let casted = mem::transmute_copy(&value);
            mem::forget(value);
            Err(SetError {
                value: casted,
                error: Error::CONFLICT,
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
        if likely!(!logs.write_log.next_push_allocates::<V>())
            && (!mem::needs_drop::<T>() || likely!(!logs.garbage.next_dispose_allocates::<T>()))
            && likely!(logs.write_log.contained_set(&tcell.erased) == Contained::No)
            && likely!(self.rw_valid(&tcell.erased))
        {
            let logs = self.logs_mut();
            unsafe {
                logs.write_log.record_unchecked(&tcell.erased, value);
                if mem::needs_drop::<T>() {
                    logs.garbage
                        .dispose_unchecked(tcell.optimistic_read_relaxed())
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

impl<'tcell> Debug for RwTx<'tcell> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadTx")
            .field("pin_mut_ref", &self.as_impl().pin_ref)
            .finish()
    }
}

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
                _ => self.as_impl().borrow_unlogged_impl(tcell),
            }
        } else {
            // If the type is zero sized, there's no need to any synchronization.
            Ok(Ref::new(unsafe { mem::zeroed::<ManuallyDrop<T>>() }))
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
            .dispose(ManuallyDrop::new(After::new(privatizer, |p| p())));
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
