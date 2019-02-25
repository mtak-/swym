use crate::{
    internal::{
        alloc::dyn_vec::DynElemMut,
        epoch::{QuiesceEpoch, EPOCH_CLOCK},
        tcell_erased::TCellErased,
        thread::{ThreadKeyRaw, TxState},
        write_log::{dumb_reference_hash, Contained, Entry, WriteEntryImpl},
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
    thread_key: ThreadKeyRaw,
}

impl RWTxImpl {
    #[inline]
    fn new(thread_key: ThreadKeyRaw) -> Self {
        RWTxImpl { thread_key }
    }

    #[inline]
    fn tx_state(self) -> NonNull<TxState> {
        self.thread_key.tx_state()
    }

    #[inline]
    fn pin_epoch(self) -> QuiesceEpoch {
        unsafe { self.thread_key.synch().as_mut().current_epoch.get_unsync() }
    }

    #[inline(never)]
    #[cold]
    unsafe fn commit_read_fail(self) -> Option<QuiesceEpoch> {
        self.tx_state().as_ref().write_log.unlock_entries();
        None
    }

    #[inline]
    unsafe fn commit_success(self) -> Option<QuiesceEpoch> {
        let tx_state = &*self.tx_state().as_ptr();
        tx_state.write_log.perform_writes();

        let sync_epoch = EPOCH_CLOCK.fetch_and_tick(Release);
        debug_assert!(
            self.pin_epoch() <= sync_epoch,
            "`EpochClock::fetch_and_tick` returned an earlier time than expected"
        );

        tx_state.write_log.publish(sync_epoch.next());

        Some(sync_epoch)
    }

    #[inline]
    unsafe fn commit_slower(self) -> Option<QuiesceEpoch> {
        if likely!(self
            .tx_state()
            .as_ref()
            .read_log
            .validate_reads(self.pin_epoch()))
        {
            self.commit_success()
        } else {
            self.commit_read_fail()
        }
    }

    #[inline(never)]
    unsafe fn commit_slow(self) -> Option<QuiesceEpoch> {
        let tx_state = &mut *self.tx_state().as_ptr();
        tx_state.remove_writes_from_reads();
        // TODO: would commit algorithm be faster with a single global lock, or lock striping?
        // per object locking causes a cmpxchg per entry
        if likely!(tx_state.write_log.try_lock_entries(self.pin_epoch())) {
            self.commit_slower()
        } else {
            None
        }
    }

    #[inline]
    unsafe fn commit(self) -> Option<QuiesceEpoch> {
        debug_assert!(
            self.pin_epoch() <= EPOCH_CLOCK.now(Acquire),
            "`EpochClock` behind current transaction start time"
        );
        let tx_state = &*self.tx_state().as_ptr();
        if likely!(!tx_state.write_log.is_empty()) {
            self.commit_slow()
        } else {
            Some(QuiesceEpoch::first())
        }
    }

    #[inline]
    fn rw_valid(self, erased: &TCellErased, o: AtomicOrdering) -> bool {
        self.pin_epoch()
            .read_write_valid_lockable(&erased.current_epoch, o)
    }

    #[inline(never)]
    #[cold]
    unsafe fn get_slow<T>(self, tcell: &TCell<T>) -> Result<ManuallyDrop<T>, Error> {
        let tx_state = &mut *self.tx_state().as_ptr();
        let found = tx_state.write_log.find(&tcell.erased);
        match found {
            None => {
                let value = tcell.erased.read_acquire::<T>();
                if likely!(self.rw_valid(&tcell.erased, Acquire)) {
                    tx_state.read_log.push(&tcell.erased);
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
        let tx_state = &mut *self.tx_state().as_ptr();
        if likely!(!tx_state.read_log.next_push_allocates())
            && likely!(
                tx_state
                    .write_log
                    .contained(dumb_reference_hash(&tcell.erased))
                    == Contained::No
            )
        {
            let value = tcell.erased.read_acquire::<T>();
            if likely!(self.rw_valid(&tcell.erased, Acquire)) {
                tx_state.read_log.push_unchecked(&tcell.erased);
                return Ok(value);
            }
        }
        self.get_slow(tcell)
    }

    #[inline(never)]
    #[cold]
    unsafe fn get_unlogged_slow<T>(self, tcell: &TCell<T>) -> Result<ManuallyDrop<T>, Error> {
        let tx_state = &*self.tx_state().as_ptr();
        let found = tx_state.write_log.find(&tcell.erased);
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
        let tx_state = &*self.tx_state().as_ptr();
        if likely!(
            tx_state
                .write_log
                .contained(dumb_reference_hash(&tcell.erased))
                == Contained::No
        ) {
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
        let tx_state = &mut *self.tx_state().as_ptr();
        match tx_state.write_log.entry(&tcell.erased) {
            Entry::Vacant { write_log, hash } => {
                if likely!(self.rw_valid(&tcell.erased, Relaxed)) {
                    write_log.push(&tcell.erased, value, hash);
                    if mem::needs_drop::<T>() {
                        tx_state.garbage.trash(tcell.erased.read_relaxed::<T>())
                    }
                    return Ok(());
                }
            }
            Entry::Occupied { mut entry, hash } => {
                if V::REQUEST_TCELL_LIFETIME {
                    entry.deactivate();
                    tx_state.write_log.push(&tcell.erased, value, hash);
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
        let tx_state = &mut *self.tx_state().as_ptr();
        let hash = dumb_reference_hash(&tcell.erased);

        if likely!(!tx_state.write_log.next_push_allocates::<V>())
            && (!mem::needs_drop::<T>() || likely!(!tx_state.garbage.next_queue_allocates::<T>()))
            && likely!(tx_state.write_log.contained(hash) == Contained::No)
            && likely!(self.rw_valid(&tcell.erased, Relaxed))
        {
            tx_state
                .write_log
                .push_unchecked(&tcell.erased, value, hash);
            if mem::needs_drop::<T>() {
                tx_state
                    .garbage
                    .trash_unchecked(tcell.erased.read_relaxed::<T>())
            }
            Ok(())
        } else {
            self.set_slow(tcell, value)
        }
    }
}

/// A read write transaction.
pub struct RWTx<'tcell>(PhantomData<fn(&'tcell ())>);
impl<'tcell> !Send for RWTx<'tcell> {}
impl<'tcell> !Sync for RWTx<'tcell> {}

impl<'tcell> RWTx<'tcell> {
    #[inline]
    pub(crate) fn new<'a>(thread_key: ThreadKeyRaw) -> &'a mut Self {
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
                .tx_state()
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
