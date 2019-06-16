use crate::{
    internal::{
        epoch::{QuiesceEpoch, EPOCH_CLOCK},
        gc::{GlobalSynchList, OwnedSynch, ThreadGarbage},
        phoenix_tls::PhoenixTarget,
        read_log::ReadLog,
        starvation::{self, Progress},
        write_log::WriteLog,
    },
    read::ReadTx,
    rw::RwTx,
    stats,
    tx::{Error, InternalStatus, Status},
};
use core::{
    cell::UnsafeCell,
    fmt::{self, Debug, Formatter},
    marker::PhantomData,
    mem,
    ops::{Deref, DerefMut},
    ptr,
    sync::atomic::Ordering::{Relaxed, Release},
};

/// Thread local data.
///
/// Synch is aliased in the GlobalSynchList of the garbage collector by a NonNull<Synch> pointer.
/// This strongly hints that Synch and TxLogs should not be stored in the same struct; however, it
/// is an optimization win for RwTx to only have one pointer to all of the threads state.
#[repr(C, align(64))]
pub struct Thread {
    /// Contains the Read/Write logs plus the ThreadGarbage. This field needs to be referenced
    /// mutably, and the uniqueness requirement of pinning guarantees that we dont violate any
    /// aliasing rules.
    logs: UnsafeCell<Logs<'static>>,

    /// The part of a Thread that is visible to other threads in swym (an atomic epoch, and sharded
    /// lock).
    synch: OwnedSynch,

    /// Backoff handling for thread starvation.
    progress: Progress,
}

impl Default for Thread {
    #[inline]
    fn default() -> Self {
        Thread::new()
    }
}

impl Thread {
    #[inline]
    pub fn new() -> Self {
        Thread {
            logs:     UnsafeCell::new(Logs::new()),
            synch:    OwnedSynch::new(),
            progress: Progress::new(),
        }
    }

    /// Returns whether the thread is pinned.
    #[inline]
    fn is_pinned(&self) -> bool {
        self.synch.current_epoch().is_active()
    }

    /// Tries to pin the current thread, returns None if already pinned.
    ///
    /// This makes mutable access to `Logs` safe, and is the only way to perform transactions.
    #[inline]
    pub fn try_pin<'tcell>(&'tcell self) -> Option<Pin<'tcell>> {
        Pin::try_new(self)
    }
}

impl PhoenixTarget for Thread {
    fn subscribe(&mut self) {
        unsafe {
            starvation::inc_thread_estimate();
            GlobalSynchList::instance().write().register(&self.synch);
        }
    }

    fn unsubscribe(&mut self) {
        unsafe {
            // All thread garbage must be collected before the Thread is dropped.
            (&mut *self.logs.get())
                .garbage
                .synch_and_collect_all(&self.synch);
        }

        // fullfilling the promise we made in `Self::new`. we must unregister before
        // deallocation, or there will be UB
        let did_remove = GlobalSynchList::instance().write().unregister(&self.synch);
        debug_assert!(
            did_remove,
            "failed to find thread in the global thread list"
        );
        starvation::dec_thread_estimate();
    }
}

// TODO: optimize memory layout
#[repr(C)]
pub struct Logs<'tcell> {
    pub read_log:  ReadLog<'tcell>,
    pub write_log: WriteLog<'tcell>,
    pub garbage:   ThreadGarbage,
}

impl<'tcell> Logs<'tcell> {
    #[inline]
    fn new() -> Self {
        Logs {
            read_log:  ReadLog::new(),
            write_log: WriteLog::new(),
            garbage:   ThreadGarbage::new(),
        }
    }

    #[inline]
    fn validate_start_state(&self) {
        debug_assert!(self.read_log.is_empty());
        debug_assert!(self.write_log.is_empty());
        debug_assert!(self.garbage.is_speculative_bag_empty());
    }
}

#[cfg(debug_assertions)]
impl<'tcell> Drop for Logs<'tcell> {
    fn drop(&mut self) {
        self.validate_start_state();
    }
}

pub struct PinRef<'tx, 'tcell> {
    thread:  &'tx Thread,
    phantom: PhantomData<fn(&'tcell ())>,
}

impl<'tx, 'tcell> Debug for PinRef<'tx, 'tcell> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.pad("PinRef { .. }")
    }
}

impl<'tx, 'tcell> PinRef<'tx, 'tcell> {
    /// Returns a reference to the current threads Synch.
    #[inline]
    pub fn reborrow(&mut self) -> PinRef<'_, 'tcell> {
        PinRef {
            thread:  self.thread,
            phantom: PhantomData,
        }
    }

    /// Returns a reference to the current threads Synch.
    #[inline]
    fn synch(&self) -> &OwnedSynch {
        &self.thread.synch
    }

    /// Returns a reference to the transaction logs (read/write/thread garbage).
    #[inline]
    pub fn logs(&self) -> &Logs<'tcell> {
        unsafe { &*self.thread.logs.get() }
    }

    /// Returns a reference to the Progress backoff object.
    #[inline]
    pub fn progress(&self) -> &Progress {
        &self.thread.progress
    }

    /// Gets the currently pinned epoch.
    #[inline]
    pub fn pin_epoch(&self) -> QuiesceEpoch {
        let pin_epoch = self.synch().current_epoch();
        debug_assert!(
            pin_epoch.is_active(),
            "attempt to get pinned_epoch of thread that is not pinned"
        );
        pin_epoch
    }
}

pub struct PinMutRef<'tx, 'tcell> {
    pin_ref: PinRef<'tx, 'tcell>,
}

impl<'tx, 'tcell> Deref for PinMutRef<'tx, 'tcell> {
    type Target = PinRef<'tx, 'tcell>;

    #[inline]
    fn deref(&self) -> &PinRef<'tx, 'tcell> {
        &self.pin_ref
    }
}

impl<'tx, 'tcell> Debug for PinMutRef<'tx, 'tcell> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.pad("PinMutRef { .. }")
    }
}

impl<'tx, 'tcell> PinMutRef<'tx, 'tcell> {
    /// Returns a reference to the current threads Synch.
    #[inline]
    pub fn reborrow(&mut self) -> PinMutRef<'_, 'tcell> {
        PinMutRef {
            pin_ref: self.pin_ref.reborrow(),
        }
    }

    /// Returns a &mut to the transaction logs (read/write/thread garbage).
    #[inline]
    pub fn logs_mut(&mut self) -> &mut Logs<'tcell> {
        unsafe { &mut *(self.pin_ref.thread.logs.get() as *const _ as *mut _) }
    }

    #[inline]
    unsafe fn into_inner(self) -> (&'tx OwnedSynch, &'tx mut Logs<'tcell>, &'tx Progress) {
        let synch = &self.pin_ref.thread.synch;
        let logs = &mut *(self.pin_ref.thread.logs.get() as *const _ as *mut _);
        let progress = &self.pin_ref.thread.progress;
        (synch, logs, progress)
    }
}

pub struct Pin<'tcell> {
    pin_ref: PinRef<'tcell, 'tcell>,
}

impl<'tcell> Drop for Pin<'tcell> {
    #[inline]
    fn drop(&mut self) {
        self.synch().unpin(Release);
        // Panics are more or less considered a successful transaction with no write log.
        self.progress().progressed();
    }
}

impl<'tcell> Deref for Pin<'tcell> {
    type Target = PinRef<'tcell, 'tcell>;

    #[inline]
    fn deref(&self) -> &PinRef<'tcell, 'tcell> {
        &self.pin_ref
    }
}

impl<'tcell> Pin<'tcell> {
    #[inline]
    fn try_new(thread: &'tcell Thread) -> Option<Pin<'tcell>> {
        if likely!(!thread.is_pinned()) {
            let now = EPOCH_CLOCK.now();
            if let Some(now) = now {
                thread.synch.pin(now, Release);
                Some(Pin {
                    pin_ref: PinRef {
                        thread,
                        phantom: PhantomData,
                    },
                })
            } else {
                abort!()
            }
        } else {
            None
        }
    }

    #[inline]
    fn repin(&mut self) {
        let now = EPOCH_CLOCK.now();
        if let Some(now) = now {
            self.synch().repin(now, Release);
        } else {
            abort!()
        }
    }

    #[inline]
    fn snooze_repin(&mut self) {
        self.progress().failed_to_progress(self.pin_epoch());
        self.repin()
    }

    #[inline]
    fn unpin_without_progress(self) {
        self.synch().unpin(Release);
        mem::forget(self);
    }

    /// Runs a read only transaction.
    #[inline]
    pub fn run_read<F, O>(mut self, mut f: F) -> O
    where
        F: FnMut(&ReadTx<'tcell>) -> Result<O, Error>,
    {
        let mut conflicts = 0;
        let result = loop {
            let r = f(ReadTx::new(&mut self));
            match r {
                Ok(o) => break o,
                Err(Error::CONFLICT) => {}
            }
            conflicts += 1;
            self.snooze_repin();
        };
        stats::read_transaction_conflicts(conflicts);
        result
    }

    /// Runs a read-write transaction.
    #[inline]
    pub fn run_rw<F, O>(mut self, mut f: F) -> O
    where
        F: FnMut(&mut RwTx<'tcell>) -> Result<O, Status>,
    {
        let mut eager_conflicts = 0;
        let mut commit_conflicts = 0;
        let result = loop {
            self.logs().validate_start_state();
            {
                let mut pin_rw = unsafe { PinRw::new(&mut self) };
                let r = f(RwTx::new(&mut pin_rw));
                match r {
                    Ok(o) => {
                        if likely!(pin_rw.commit()) {
                            self.logs().validate_start_state();
                            break o;
                        }
                        commit_conflicts += 1;
                    }
                    Err(Status {
                        kind: InternalStatus::Error(Error::CONFLICT),
                    }) => {
                        eager_conflicts += 1;
                    }
                    Err(Status::AWAIT_RETRY) => {
                        crate::internal::parking::park(pin_rw);
                        self.repin();
                        continue;
                    }
                }
            }
            self.snooze_repin();
        };
        // A successful commit has already recorded progress
        self.unpin_without_progress();
        stats::write_transaction_eager_conflicts(eager_conflicts);
        stats::write_transaction_commit_conflicts(commit_conflicts);
        result
    }
}

pub struct PinRw<'tx, 'tcell> {
    pin_ref: PinMutRef<'tx, 'tcell>,
}

impl<'tx, 'tcell> Drop for PinRw<'tx, 'tcell> {
    #[inline(never)]
    #[cold]
    fn drop(&mut self) {
        let logs = self.logs_mut();
        logs.read_log.clear();
        logs.garbage.abort_speculative_garbage();
        logs.write_log.clear();
    }
}

impl<'tx, 'tcell> Deref for PinRw<'tx, 'tcell> {
    type Target = PinMutRef<'tx, 'tcell>;

    #[inline]
    fn deref(&self) -> &PinMutRef<'tx, 'tcell> {
        &self.pin_ref
    }
}

impl<'tx, 'tcell> DerefMut for PinRw<'tx, 'tcell> {
    #[inline]
    fn deref_mut(&mut self) -> &mut PinMutRef<'tx, 'tcell> {
        &mut self.pin_ref
    }
}

impl<'tx, 'tcell> PinRw<'tx, 'tcell> {
    /// It is not safe to mem::forget PinRw
    #[inline]
    unsafe fn new(pin: &'tx mut Pin<'tcell>) -> Self {
        PinRw {
            pin_ref: PinMutRef {
                pin_ref: pin.pin_ref.reborrow(),
            },
        }
    }

    #[inline]
    pub unsafe fn into_inner(self) -> (&'tx OwnedSynch, &'tx mut Logs<'tcell>, &'tx Progress) {
        let pin_ref = ptr::read(&self.pin_ref);
        mem::forget(self);
        pin_ref.into_inner()
    }

    pub fn parked(self) -> ParkPinMutRef<'tx, 'tcell> {
        ParkPinMutRef::new(self)
    }
}

pub struct ParkPinMutRef<'tx, 'tcell> {
    logs:          &'tx mut Logs<'tcell>,
    pub pin_epoch: QuiesceEpoch,
    synch:         &'tx OwnedSynch,
}

impl<'tx, 'tcell> Drop for ParkPinMutRef<'tx, 'tcell> {
    fn drop(&mut self) {
        self.logs.read_log.clear();
        self.logs.write_log.clear_no_drop();
        let now = EPOCH_CLOCK.now();
        if let Some(now) = now {
            self.synch.pin(now, Release);
        } else {
            abort!()
        }
    }
}

impl<'tx, 'tcell> Deref for ParkPinMutRef<'tx, 'tcell> {
    type Target = Logs<'tcell>;

    #[inline]
    fn deref(&self) -> &Logs<'tcell> {
        self.logs
    }
}

impl<'tx, 'tcell> ParkPinMutRef<'tx, 'tcell> {
    fn new(pin_rw: PinRw<'tx, 'tcell>) -> Self {
        pin_rw.progress().progressed();
        let (synch, logs, _) = unsafe { pin_rw.into_inner() };
        let pin_epoch = synch.current_epoch();

        // Before doing anything that can panic, create our `Drop` type that will cleanup our logs.
        let result = ParkPinMutRef {
            logs,
            pin_epoch,
            synch,
        };

        result.logs.garbage.abort_speculative_garbage();

        // this can panic
        unsafe { result.logs.write_log.drop_writes() };
        synch.unpin(Relaxed);
        result
    }

    pub fn park_token(&self) -> usize {
        self as *const _ as usize
    }

    pub unsafe fn from_park_token(token: usize) -> &'static Self {
        &*(token as *const _)
    }
}
