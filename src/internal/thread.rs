use crate::{
    internal::{
        epoch::{QuiesceEpoch, EPOCH_CLOCK},
        gc::{GlobalSynchList, OwnedSynch, ThreadGarbage},
        read_log::ReadLog,
        stats,
        write_log::WriteLog,
    },
    read::ReadTx,
    rw::RwTx,
    tx::Error,
};
use std::{
    cell::{Cell, UnsafeCell},
    fmt::{self, Debug, Formatter},
    marker::PhantomData,
    mem,
    ops::{Deref, DerefMut},
    process,
    ptr::{self, NonNull},
    sync::atomic::Ordering::{Acquire, Release},
};

/// Intrusive reference counted thread local data.
///
/// Synch is aliased in the GlobalSynchList of the garbage collector by a NonNull<Synch> pointer.
/// This strongly hints that Synch and TxLogs should not be stored in the same struct; however, it
/// is an optimization win for RwTx to only have one pointer to all of the threads state.
///
/// TODO: It's possible we don't need reference counting, if read/try_read/rw/try_rw are made free
/// functions. But, doing so, makes 'tcell lifetimes hard/impossible to create.
#[repr(C, align(64))]
pub struct Thread {
    /// Contains the Read/Write logs plus the ThreadGarbage. This field needs to be referenced
    /// mutably, and the uniqueness requirement of pinning guarantees that we dont violate any
    /// aliasing rules.
    logs: UnsafeCell<Logs<'static>>,

    /// The part of a Thread that is visible to other threads in swym (an atomic epoch, and sharded
    /// lock).
    synch: OwnedSynch,

    /// The reference count.
    ref_count: Cell<usize>,
}

impl Thread {
    #[inline(never)]
    #[cold]
    fn new() -> Self {
        Thread {
            logs:      UnsafeCell::new(Logs::new()),
            synch:     OwnedSynch::new(),
            ref_count: Cell::new(1),
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
    pub fn remove_writes_from_reads(&mut self) {
        let mut count = 0;
        unsafe {
            for i in (0..self.read_log.len()).rev() {
                debug_assert!(i < self.read_log.len(), "bug in `remove_writes_from_reads`");
                if self
                    .write_log
                    .find(self.read_log.get_unchecked(i))
                    .is_some()
                {
                    let l = self.read_log.len();
                    self.read_log.swap_erase_unchecked(i);
                    count += 1;
                    debug_assert!(
                        l == self.read_log.len() + 1,
                        "bug in `remove_writes_from_reads`"
                    );
                }
            }
        }
        stats::unnecessary_read_size(count)
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

/// Reference counted pointer to Thread.
pub struct ThreadKeyInner {
    thread: NonNull<Thread>,
}

impl Debug for ThreadKeyInner {
    #[cold]
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.pad("ThreadKeyInner { .. }")
    }
}

impl Deref for ThreadKeyInner {
    type Target = Thread;

    #[inline]
    fn deref(&self) -> &Thread {
        unsafe { self.thread.as_ref() }
    }
}

impl Clone for ThreadKeyInner {
    #[inline]
    fn clone(&self) -> Self {
        let count = self.ref_count.get();;
        debug_assert!(count > 0, "attempt to clone a deallocated `ThreadKey`");
        self.ref_count.set(count + 1);
        ThreadKeyInner {
            thread: self.thread,
        }
    }
}

impl Drop for ThreadKeyInner {
    #[inline]
    fn drop(&mut self) {
        let count = self.ref_count.get();
        debug_assert!(count > 0, "double free on `ThreadKey` attempted");
        if likely!(count != 1) {
            self.ref_count.set(count - 1)
        } else {
            // this is safe as long as the reference counting logic is safe
            unsafe {
                dealloc(self.thread);
            }

            #[inline(never)]
            #[cold]
            unsafe fn dealloc(mut this_ptr: NonNull<Thread>) {
                let this = this_ptr.as_mut();
                // All thread garbage must be collected before the Thread is dropped.
                (&mut *this.logs.get())
                    .garbage
                    .synch_and_collect_all(&this.synch);

                // fullfilling the promise we made in `Self::new`. we must unregister before
                // deallocation, or there will be UB
                GlobalSynchList::instance_unchecked()
                    .write()
                    .unregister(&this.synch);
                // clear the cached #[thread_local] pointer to `this`
                crate::thread_key::tls::clear_tls();
                drop(Box::from_raw(this_ptr.as_ptr()))
            }
        }
    }
}

impl ThreadKeyInner {
    #[inline]
    pub fn new() -> Self {
        let thread = Box::new(Thread::new());
        unsafe {
            // here we promise to never move or drop our thread until we unregister it.
            GlobalSynchList::instance().write().register(&thread.synch);
            ThreadKeyInner {
                thread: NonNull::new_unchecked(Box::into_raw(thread)),
            }
        }
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
    fn into_inner(self) -> (&'tx OwnedSynch, &'tx mut Logs<'tcell>) {
        let synch = &self.pin_ref.thread.synch;
        let logs = unsafe { &mut *(self.pin_ref.thread.logs.get() as *const _ as *mut _) };
        (synch, logs)
    }
}

pub struct Pin<'tcell> {
    pin_ref: PinRef<'tcell, 'tcell>,
}

impl<'tcell> Drop for Pin<'tcell> {
    #[inline]
    fn drop(&mut self) {
        self.synch().unpin(Release)
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
            let now = EPOCH_CLOCK.now(Acquire);
            if let Some(now) = now {
                thread.synch.pin(now, Release);
                Some(Pin {
                    pin_ref: PinRef {
                        thread,
                        phantom: PhantomData,
                    },
                })
            } else {
                process::abort()
            }
        } else {
            None
        }
    }

    #[inline]
    fn repin(&mut self) {
        let now = EPOCH_CLOCK.now(Acquire);
        if let Some(now) = now {
            self.synch().repin(now, Release);
        } else {
            process::abort()
        }
    }

    /// Runs a read only transaction.
    #[inline]
    pub fn run_read<F, O>(mut self, mut f: F) -> O
    where
        F: FnMut(&ReadTx<'tcell>) -> Result<O, Error>,
    {
        loop {
            stats::read_transaction();

            let r = f(ReadTx::new(&mut self));
            match r {
                Ok(o) => break o,
                Err(Error::RETRY) => {}
            }
            stats::read_transaction_failure();
            self.repin()
        }
    }

    /// Runs a read-write transaction.
    #[inline]
    pub fn run_rw<F, O>(mut self, mut f: F) -> O
    where
        F: FnMut(&mut RwTx<'tcell>) -> Result<O, Error>,
    {
        loop {
            stats::write_transaction();
            self.logs().validate_start_state();
            {
                let mut pin = unsafe { PinRw::new(&mut self) };
                let r = f(RwTx::new(&mut pin));
                match r {
                    Ok(o) => {
                        if likely!(pin.commit()) {
                            self.logs().validate_start_state();
                            return o;
                        }
                    }
                    Err(Error::RETRY) => {}
                }
            }
            stats::write_transaction_failure();
            self.repin();
        }
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
    unsafe fn into_inner(self) -> (&'tx OwnedSynch, &'tx mut Logs<'tcell>) {
        let pin_ref = ptr::read(&self.pin_ref);
        mem::forget(self);
        pin_ref.into_inner()
    }

    /// The commit algorithm, called after user code has finished running without returning an
    /// error. Returns true if the transaction committed successfully.
    #[inline]
    fn commit(self) -> bool {
        if likely!(!self.logs().write_log.is_empty()) {
            self.commit_slow()
        } else {
            unsafe { self.commit_empty_write_log() }
        }
    }

    #[inline]
    unsafe fn commit_empty_write_log(self) -> bool {
        let (_, logs) = self.into_inner();
        // RwTx validates reads as they occur. As a result, if there are no writes, then we have
        // no work to do in our commit algorithm.
        //
        // On the off chance we do have garbage, with an empty write log. Then there's no way
        // that garbage could have been previously been shared, as the data cannot
        // have been made inaccessible via an STM write. It is a logic error in user
        // code, and requires `unsafe` to make that error. This assert helps to
        // catch that.
        debug_assert!(
            logs.garbage.is_speculative_bag_empty(),
            "Garbage queued, without any writes!"
        );
        logs.read_log.clear();
        true
    }

    /// This performs a lot of lock cmpxchgs, so inlining doesn't really doesn't give us much.
    #[inline(never)]
    fn commit_slow(mut self) -> bool {
        // Locking the write log, would cause validation of any reads to the same TCell to fail.
        // So we remove all TCells in the read log that are also in the read log, and assume all
        // TCells in the write log, have been read.
        self.logs_mut().remove_writes_from_reads();

        // Locking the write set can fail if another thread has the lock, or if any TCell in the
        // write set has been updated since the transaction began.
        //
        // TODO: would commit algorithm be faster with a single global lock, or lock striping?
        // per object locking causes a cmpxchg per entry
        unsafe {
            if likely!(self.logs().write_log.try_lock_entries(self.pin_epoch())) {
                self.write_log_lock_success()
            } else {
                self.write_log_lock_failure()
            }
        }
    }

    #[inline(never)]
    #[cold]
    unsafe fn write_log_lock_failure(self) -> bool {
        false
    }

    #[inline]
    unsafe fn write_log_lock_success(self) -> bool {
        // after locking the write set, ensure nothing in the read set has been modified.
        if likely!(self.logs().read_log.validate_reads(self.pin_epoch())) {
            // The transaction can no longer fail, so proceed to modify and publish the TCells in
            // the write set.
            self.validation_success()
        } else {
            self.validation_failure()
        }
    }

    #[inline]
    unsafe fn validation_success(self) -> bool {
        let (synch, logs) = self.into_inner();

        // The writes must be performed before the EPOCH_CLOCK is tick'ed.
        // Reads can get away with performing less work with this ordering.
        logs.write_log.perform_writes();

        let sync_epoch = EPOCH_CLOCK.fetch_and_tick();
        debug_assert!(
            synch.current_epoch() <= sync_epoch,
            "`EpochClock::fetch_and_tick` returned an earlier time than expected"
        );

        // unlocks everything in the write lock and sets the TCell epochs to sync_epoch.next()
        logs.write_log.publish(sync_epoch.next());
        logs.read_log.clear();
        logs.write_log.clear_no_drop();
        logs.garbage.seal_with_epoch(synch, sync_epoch);

        true
    }

    #[inline(never)]
    #[cold]
    unsafe fn validation_failure(self) -> bool {
        // on fail unlock the write set
        self.logs().write_log.unlock_entries();
        false
    }
}
