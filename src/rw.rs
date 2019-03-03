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
        epoch::{QuiesceEpoch, EPOCH_CLOCK},
        tcell_erased::TCellErased,
        thread::{ThreadKeyInner, TxLogs},
        write_log::{bloom_hash, Contained, Entry, WriteEntryImpl},
    },
    tcell::TCell,
    tx::{self, Error, Ordering, SetError, Write, _TValue},
};
use std::{
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    ptr::{self, NonNull},
    sync::atomic::Ordering::{self as AtomicOrdering, Acquire, Relaxed, Release},
};

#[derive(Clone, Copy, Debug)]
struct RWTxImpl {
    thread_key: ThreadKeyInner,
}

impl RWTxImpl {
    #[inline]
    fn new(thread_key: ThreadKeyInner) -> Self {
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
        unsafe { self.thread_key.pinned_epoch() }
    }

    /// The commit algorithm, called after user code has finished running without returning an
    /// error. This doesn't handle things like sealing up the current garbage bag, etc...
    ///
    /// Returns the QuiesceEpoch that has just began on success, or None on failure.
    #[inline]
    unsafe fn commit(self) -> Option<QuiesceEpoch> {
        debug_assert!(
            self.pin_epoch() <= EPOCH_CLOCK.now(Acquire),
            "`EpochClock` behind current transaction start time"
        );
        let logs = &*self.logs().as_ptr();

        if likely!(!logs.write_log.is_empty()) {
            self.commit_slow()
        } else {
            // RWTx validates reads as they occur. As a result, if there are no writes, then we have
            // no work to do in our commit algorithm.
            //
            // The first epoch is a safe choice as having "began", it is only used for queuing up
            // garbage, which we should not have with an empty write log. On the off chance we do
            // have garbage, with an empty write log. Then there's no way that garbage could have
            // been previously been shared, as the data cannot have been made inaccessible via an
            // STM write. It is a logic error in user code, and requires `unsafe` to make that
            // error. This assert helps to catch that.
            debug_assert!(
                logs.garbage.is_speculative_bag_empty(),
                "Garbage queued, without any writes!"
            );
            Some(QuiesceEpoch::first())
        }
    }

    /// This performs a lot of lock cmpxchgs, so inlining doesn't really doesn't give us much.
    #[inline(never)]
    unsafe fn commit_slow(self) -> Option<QuiesceEpoch> {
        let logs = &mut *self.logs().as_ptr();

        // Locking the write log, would cause validation of any reads to the same TCell to fail.
        // So we remove all TCells in the read log that are also in the read log, and assume all
        // TCells in the write log, have been read.
        logs.remove_writes_from_reads();

        // Locking the write set can fail if another thread has the lock, or if any TCell in the
        // write set has been updated since the transaction began.
        //
        // TODO: would commit algorithm be faster with a single global lock, or lock striping?
        // per object locking causes a cmpxchg per entry
        if likely!(logs.write_log.try_lock_entries(self.pin_epoch())) {
            self.commit_slower()
        } else {
            None
        }
    }

    #[inline]
    unsafe fn commit_slower(self) -> Option<QuiesceEpoch> {
        // after locking the write set, ensure nothing in the read set has been modified.
        if likely!(self
            .logs()
            .as_ref()
            .read_log
            .validate_reads(self.pin_epoch()))
        {
            // The transaction can no longer fail, so proceed to modify and publish the TCells in
            // the write set.
            self.validation_success()
        } else {
            self.validation_failure()
        }
    }

    #[inline]
    unsafe fn validation_success(self) -> Option<QuiesceEpoch> {
        let logs = &*self.logs().as_ptr();

        // The writes must be performed before the EPOCH_CLOCK is tick'ed.
        // Reads can get away with performing less work with this ordering.
        logs.write_log.perform_writes();

        let sync_epoch = EPOCH_CLOCK.fetch_and_tick(Release);
        debug_assert!(
            self.pin_epoch() <= sync_epoch,
            "`EpochClock::fetch_and_tick` returned an earlier time than expected"
        );

        // unlocks everything in the write lock and sets the TCell epochs to sync_epoch.next()
        logs.write_log.publish(sync_epoch.next());

        // return the synch epoch
        Some(sync_epoch)
    }

    #[inline(never)]
    #[cold]
    unsafe fn validation_failure(self) -> Option<QuiesceEpoch> {
        // on fail unlock the write set
        self.logs().as_ref().write_log.unlock_entries();
        None
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
    pub(crate) fn new<'a>(thread_key: ThreadKeyInner) -> &'a mut Self {
        unsafe { mem::transmute(RWTxImpl::new(thread_key)) }
    }

    #[inline]
    fn as_impl(&self) -> RWTxImpl {
        unsafe { mem::transmute(self) }
    }

    #[inline]
    pub(crate) unsafe fn commit(&self) -> Option<QuiesceEpoch> {
        self.as_impl().commit()
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
