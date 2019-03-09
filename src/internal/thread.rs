use crate::{
    internal::{
        epoch::{QuiesceEpoch, EPOCH_CLOCK},
        gc::{GlobalSynchList, OwnedSynch, ThreadGarbage},
        pointer::PtrExt,
        read_log::ReadLog,
        stats,
        write_log::WriteLog,
    },
    read::ReadTx,
    rw::RWTx,
    tx::Error,
};
use std::{
    cell::Cell,
    marker::PhantomData,
    mem,
    ptr::NonNull,
    sync::atomic::Ordering::{Acquire, Relaxed, Release},
};

/// Intrusive reference counted thread local data.
///
/// Synch is aliased in the GlobalSynchList of the garbage collector by a NonNull<Synch> pointer.
/// This strongly hints that Synch and TxLogs should not be stored in the same struct; however, it
/// is an optimization win for RWTx to only have one pointer to all of the threads state.
///
/// TODO: It's possible we don't need reference counting, if read/try_read/rw/try_rw are made free
/// functions. But, doing so, makes 'tcell lifetimes hard/impossible to create.
#[repr(C, align(64))]
struct Thread {
    /// Contains the Read/Write logs plus the ThreadGarbage. This field needs to be referenced
    /// mutably, and the uniqueness requirement of pinning guarantees that we dont violate any
    /// aliasing rules.
    logs: Logs,

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
            logs:      Logs::new(),
            synch:     OwnedSynch::new(),
            ref_count: Cell::new(1),
        }
    }
}

/// Given a pointer to a thread, we want to be able to create pointers to its members without going
/// through a &mut. Going through an &mut would violate rusts aliasing rules, because Synch might be
/// borrowed immutably by other threads performing garbage collection.
///
/// These free functions handle the `offset_of` logic for creating member pointers.

/// Returns a raw pointer to the transaction logs (read/write/thread garbage).
#[inline]
fn logs(thread: NonNull<Thread>) -> NonNull<Logs> {
    // relies on repr(C) on Thread
    thread.cast()
}

/// Returns a raw pointer to the shared state of the thread (sharded lock and atomic epoch).
#[inline]
fn synch(thread: NonNull<Thread>) -> NonNull<OwnedSynch> {
    // relies on repr(C) on Thread
    unsafe {
        logs(thread)
            .add(1) // synch is the field immediately after tx logs
            .assume_aligned() // assume_aligned here, makes align_next optimize away on most (all?) platforms
            .cast::<OwnedSynch>()
            .align_next() // adjusts the pointer in the case that Synchs alignment is > TxLogs
    }
}

/// Returns a raw pointer to the reference count.
#[inline]
fn ref_count(thread: NonNull<Thread>) -> NonNull<Cell<usize>> {
    // relies on repr(C) on Thread
    unsafe {
        synch(thread)
            .add(1) // ref_count is the field immediately after tx logs
            .assume_aligned() // assume_aligned here, makes align_next optimize away on most (all?) platforms
            .cast::<Cell<usize>>()
            .align_next() // adjusts the pointer in the case that Cell<usize> alignment is > Synch
    }
}

/// Reference counted pointer to Thread.
#[derive(Debug)]
pub struct ThreadKeyInner {
    thread: NonNull<Thread>,
}

impl Clone for ThreadKeyInner {
    #[inline]
    fn clone(&self) -> Self {
        let ref_count = self.ref_count();
        let count = ref_count.get();
        debug_assert!(count > 0, "attempt to clone a deallocated `ThreadKey`");
        ref_count.set(count + 1);
        ThreadKeyInner {
            thread: self.thread,
        }
    }
}

impl Drop for ThreadKeyInner {
    #[inline]
    fn drop(&mut self) {
        let ref_count = self.ref_count();
        let count = ref_count.get();
        debug_assert!(count > 0, "double free on `ThreadKey` attempted");
        if likely!(count != 1) {
            ref_count.set(count - 1)
        } else {
            // this is safe as long as the reference counting logic is safe
            unsafe {
                dealloc(self.thread);
            }

            #[inline(never)]
            #[cold]
            unsafe fn dealloc(this: NonNull<Thread>) {
                let synch = synch(this);
                let synch = synch.as_ref();

                // All thread garbage must be collected before the Thread is dropped.
                logs(this).as_mut().garbage.synch_and_collect_all(synch);

                // fullfilling the promise we made in `Self::new`. we must unregister before
                // deallocation, or there will be UB
                GlobalSynchList::instance_unchecked()
                    .write()
                    .unregister(synch);
                // clear the cached #[thread_local] pointer to `this`
                crate::thread_key::tls::clear_tls();
                drop(Box::from_raw(this.as_ptr()))
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

    #[inline]
    fn synch(&self) -> &OwnedSynch {
        // this is safe as long as the reference counting logic is safe
        unsafe { &*synch(self.thread).as_ptr() }
    }

    #[inline]
    fn ref_count(&self) -> &Cell<usize> {
        // this is safe as long as the reference counting logic is safe
        unsafe { &*ref_count(self.thread).as_ptr() }
    }

    /// Returns whether the thread is currently active (pinned or collecting garbage).
    #[inline]
    fn is_active(&self) -> bool {
        self.synch().pin_epoch().is_active()
    }

    /// Tries to run a read only transaction. Returns Some on success.
    #[inline]
    pub fn try_read<'tcell, F, O>(&'tcell self, f: F) -> Option<O>
    where
        F: FnMut(&ReadTx<'tcell>) -> Result<O, Error>,
    {
        if likely!(!self.is_active()) {
            // we have checked that there is no transaction in the current thread, so it's safe to
            // start one.
            Some(unsafe { self.read_slow(f) })
        } else {
            None
        }
    }

    /// Runs a read only transaction. Requires the thread to not be in a transaction.
    #[inline]
    unsafe fn read_slow<'tcell, F, O>(&'tcell self, mut f: F) -> O
    where
        F: FnMut(&ReadTx<'tcell>) -> Result<O, Error>,
    {
        loop {
            stats::read_transaction();

            let (_pin, now) = self.pin_read();
            let r = f(ReadTx::new(now));
            match r {
                Ok(o) => break o,
                Err(Error::RETRY) => {}
            }

            stats::read_transaction_failure();
        }
    }

    /// Tries to run a read write transaction. Returns Some on success.
    #[inline]
    pub fn try_rw<'tcell, F, O>(&'tcell self, f: F) -> Option<O>
    where
        F: FnMut(&mut RWTx<'tcell>) -> Result<O, Error>,
    {
        if likely!(!self.is_active()) {
            // we have checked that there is no transaction in the current thread, so it's safe to
            // start one.
            Some(unsafe { self.rw_slow(f) })
        } else {
            None
        }
    }

    /// Runs a read-write transaction. Requires the thread to not be in a transaction.
    #[inline]
    unsafe fn rw_slow<'tcell, F, O>(&'tcell self, mut f: F) -> O
    where
        F: FnMut(&mut RWTx<'tcell>) -> Result<O, Error>,
    {
        loop {
            stats::write_transaction();

            logs(self.thread).as_mut().validate_start_state();
            let pin = self.pin_rw();
            let r = f(RWTx::new(RWThreadKey::new(self)));
            match r {
                Ok(o) => {
                    if likely!(pin.commit()) {
                        logs(self.thread).as_mut().validate_start_state();
                        return o;
                    }
                }
                Err(Error::RETRY) => {}
            }

            stats::write_transaction_failure();
        }
    }

    #[inline]
    fn pin_read(&self) -> (PinRead<'_>, QuiesceEpoch) {
        PinRead::new(self.synch())
    }

    #[inline]
    fn pin_rw(&self) -> PinRw<'_> {
        PinRw::new(self.thread)
    }
}

#[derive(Debug)]
pub struct RWThreadKey<'tcell> {
    thread:  NonNull<Thread>,
    phantom: PhantomData<&'tcell ThreadKeyInner>,
}

impl<'tcell> RWThreadKey<'tcell> {
    #[inline]
    fn new(thread_key: &'tcell ThreadKeyInner) -> Self {
        RWThreadKey {
            thread:  thread_key.thread,
            phantom: PhantomData,
        }
    }

    /// Returns a reference to the transaction logs (read/write/thread garbage).
    #[inline]
    pub fn logs(&self) -> &'tcell Logs {
        unsafe { &*logs(self.thread).as_ptr() }
    }

    /// Returns a &mut to the transaction logs (read/write/thread garbage).
    #[inline]
    pub fn logs_mut(&mut self) -> &'tcell mut Logs {
        unsafe { &mut *logs(self.thread).as_ptr() }
    }

    /// Gets the currently pinned epoch.
    #[inline]
    pub fn pin_epoch(&self) -> QuiesceEpoch {
        let synch = synch(self.thread);
        let synch = unsafe { synch.as_ref() };
        let pin_epoch = synch.pin_epoch();
        debug_assert!(
            pin_epoch.is_active(),
            "attempt to get pinned_epoch of thread that is not pinned"
        );
        pin_epoch
    }
}

// TODO: optimize memory layout
#[repr(C)]
pub struct Logs {
    pub read_log:  ReadLog,
    pub write_log: WriteLog,
    pub garbage:   ThreadGarbage,
}

impl Logs {
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
        for i in (0..self.read_log.len()).rev() {
            debug_assert!(i < self.read_log.len(), "bug in `remove_writes_from_reads`");
            if self
                .write_log
                .find(unsafe { self.read_log.get_unchecked(i).src.as_ref() })
                .is_some()
            {
                let l = self.read_log.len();
                unsafe {
                    self.read_log.swap_erase_unchecked(i);
                }
                count += 1;
                debug_assert!(
                    l == self.read_log.len() + 1,
                    "bug in `remove_writes_from_reads`"
                );
            }
        }
        stats::unnecessary_read_size(count)
    }

    #[inline]
    fn validate_start_state(&mut self) {
        debug_assert!(self.read_log.is_empty());
        debug_assert!(self.write_log.is_empty());
        debug_assert!(self.garbage.is_speculative_bag_empty());
    }
}

#[cfg(debug_assertions)]
impl Drop for Logs {
    fn drop(&mut self) {
        self.validate_start_state();
    }
}

struct PinRead<'a> {
    synch: &'a OwnedSynch,
}

impl<'a> PinRead<'a> {
    #[inline]
    fn new(synch: &'a OwnedSynch) -> (Self, QuiesceEpoch) {
        let now = EPOCH_CLOCK.now(Acquire);
        synch.pin(now, Release);
        (PinRead { synch }, now)
    }
}

impl<'a> Drop for PinRead<'a> {
    #[inline]
    fn drop(&mut self) {
        self.synch.unpin(Release)
    }
}

struct PinRw<'tcell> {
    thread:  NonNull<Thread>,
    phantom: PhantomData<&'tcell mut ()>,
}

impl<'a> PinRw<'a> {
    #[inline]
    fn new(thread: NonNull<Thread>) -> Self {
        let now = EPOCH_CLOCK.now(Acquire);
        unsafe {
            synch(thread).as_ref().pin(now, Release);

            PinRw {
                thread,
                phantom: PhantomData,
            }
        }
    }

    /// Gets the currently pinned epoch. Requires the thread pointer to still be valid.
    #[inline]
    pub fn pin_epoch(&self) -> QuiesceEpoch {
        // current_epoch is never changed by any thread except the owning thread, so we're able to
        // read it without synchronization.
        let synch = synch(self.thread);
        let synch = unsafe { synch.as_ref() };
        let pin_epoch = synch.pin_epoch();
        debug_assert!(
            pin_epoch.is_active(),
            "attempt to get pinned_epoch of thread that is not pinned"
        );
        pin_epoch
    }

    /// The commit algorithm, called after user code has finished running without returning an
    /// error. Returns true if the transaction committed successfully.
    #[inline]
    unsafe fn commit(self) -> bool {
        let logs = &*logs(self.thread).as_ptr();

        if likely!(!logs.write_log.is_empty()) {
            self.commit_slow()
        } else {
            self.commit_empty_write_log()
        }
    }

    #[inline]
    unsafe fn commit_empty_write_log(self) -> bool {
        let logs = &mut *logs(self.thread).as_ptr();
        let synch = synch(self.thread);
        let synch = synch.as_ref();
        mem::forget(self);
        // RWTx validates reads as they occur. As a result, if there are no writes, then we have
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
        synch.unpin(Relaxed);
        true
    }

    /// This performs a lot of lock cmpxchgs, so inlining doesn't really doesn't give us much.
    #[inline(never)]
    unsafe fn commit_slow(self) -> bool {
        let logs = &mut *logs(self.thread).as_ptr();

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
            self.write_log_lock_success()
        } else {
            self.write_log_lock_failure()
        }
    }

    #[inline(never)]
    #[cold]
    unsafe fn write_log_lock_failure(self) -> bool {
        drop(self);
        false
    }

    #[inline]
    unsafe fn write_log_lock_success(self) -> bool {
        // after locking the write set, ensure nothing in the read set has been modified.
        if likely!(logs(self.thread)
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
    unsafe fn validation_success(self) -> bool {
        let logs = &mut *logs(self.thread).as_ptr();

        // The writes must be performed before the EPOCH_CLOCK is tick'ed.
        // Reads can get away with performing less work with this ordering.
        logs.write_log.perform_writes();

        let sync_epoch = EPOCH_CLOCK.fetch_and_tick();
        debug_assert!(
            self.pin_epoch() <= sync_epoch,
            "`EpochClock::fetch_and_tick` returned an earlier time than expected"
        );

        // unlocks everything in the write lock and sets the TCell epochs to sync_epoch.next()
        logs.write_log.publish(sync_epoch.next());

        let synch = synch(self.thread);
        mem::forget(self);
        let synch = synch.as_ref();
        logs.read_log.clear();
        logs.write_log.clear_no_drop();
        logs.garbage.seal_with_epoch(synch, sync_epoch);
        synch.unpin(Relaxed);

        true
    }

    #[inline(never)]
    #[cold]
    unsafe fn validation_failure(self) -> bool {
        // on fail unlock the write set
        logs(self.thread).as_ref().write_log.unlock_entries();
        drop(self);
        false
    }
}

impl Drop for PinRw<'_> {
    #[inline(never)]
    #[cold]
    fn drop(&mut self) {
        unsafe {
            let mut logs = logs(self.thread);
            let logs = logs.as_mut();
            logs.read_log.clear();
            logs.garbage.abort_speculative_garbage();
            logs.write_log.clear();
            synch(self.thread).as_ref().unpin(Relaxed);
        }
    }
}
