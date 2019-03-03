use crate::{
    internal::{
        epoch::{AtomicQuiesceEpoch, QuiesceEpoch, EPOCH_CLOCK},
        gc::{GlobalSynchList, Synch, ThreadGarbage},
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
/// functions. But,s doing so, makes 'tcell lifetimes hard/impossible to create.
#[repr(C, align(64))]
struct Thread {
    /// Contains the Read/Write logs plus the ThreadGarbage. This field needs to be referenced
    /// mutably, and the uniqueness requirement of pinning guarantees that we dont violate any
    /// aliasing rules.
    tx_state: TxLogs,

    /// The part of a Thread that is visible to other threads in swym (an atomic epoch, and sharded
    /// lock).
    synch: Synch,

    /// The reference count.
    ref_count: Cell<usize>,
}

impl Thread {
    #[inline(never)]
    #[cold]
    fn new() -> Self {
        Thread {
            tx_state:  TxLogs::new(),
            synch:     Synch::new(),
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
fn tx_logs(thread: NonNull<Thread>) -> NonNull<TxLogs> {
    // relies on repr(C) on Thread
    thread.cast()
}

/// Returns a raw pointer to the shared state of the thread (sharded lock and atomic epoch).
#[inline]
fn synch(thread: NonNull<Thread>) -> NonNull<Synch> {
    // relies on repr(C) on Thread
    unsafe {
        tx_logs(thread)
            .add(1) // synch is the field immediately after tx logs
            .assume_aligned() // assume_aligned here, makes align_next optimize away on most (all?) platforms
            .cast::<Synch>()
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

#[derive(Debug)]
pub struct ThreadKeyInner {
    thread: NonNull<Thread>,
}

impl Clone for ThreadKeyInner {
    #[inline]
    fn clone(&self) -> Self {
        let ref_count = self.ref_count();
        let ref_count = unsafe { ref_count.as_ref() };
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
        let ref_count = unsafe { ref_count.as_ref() };
        let count = ref_count.get();
        debug_assert!(count > 0, "double free on `ThreadKey` attempted");
        if likely!(count != 1) {
            ref_count.set(count - 1)
        } else {
            unsafe {
                dealloc(ThreadKeyInner {
                    thread: self.thread,
                });
            }

            #[inline(never)]
            #[cold]
            unsafe fn dealloc(this: ThreadKeyInner) {
                let synch = this.synch();
                let synch = synch.as_ref();

                synch
                    .current_epoch
                    .set(QuiesceEpoch::end_of_time(), Release);
                this.tx_logs()
                    .as_mut()
                    .garbage
                    .synch_and_collect_all(this.synch().as_ref());
                synch.current_epoch.set(QuiesceEpoch::inactive(), Release);

                GlobalSynchList::instance_unchecked()
                    .write()
                    .unregister(synch);
                drop(Box::from_raw(this.thread.as_ptr()));
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

    /// Returns a raw pointer to the transaction logs (read/write/thread garbage).
    #[inline]
    fn tx_logs(&self) -> NonNull<TxLogs> {
        tx_logs(self.thread)
    }

    /// Returns a raw pointer to the shared state of the thread (sharded lock and atomic epoch).
    #[inline]
    fn synch(&self) -> NonNull<Synch> {
        synch(self.thread)
    }

    /// Returns a raw pointer to the reference count.
    #[inline]
    fn ref_count(&self) -> NonNull<Cell<usize>> {
        ref_count(self.thread)
    }

    /// Returns whether the thread is currently active (pinned or collecting garbage). Requires the
    /// thread pointer to still be valid.
    #[inline]
    pub unsafe fn is_active(&self) -> bool {
        // current_epoch is never changed by any thread except the owning thread, so we're able to
        // read it without synchronization.
        //
        // TODO: this should not borrow synch as_mut, and should instead do pointer casting to
        // epoch::Storage
        self.synch().as_mut().current_epoch.is_active_unsync()
    }

    /// Runs a read only transaction. Requires the thread to not be in a transaction, and the thread
    /// pointer to still be valid.
    #[inline]
    pub unsafe fn read_slow<'tcell, F, O>(&self, mut f: F) -> O
    where
        F: FnMut(&ReadTx<'tcell>) -> Result<O, Error>,
    {
        loop {
            stats::read_transaction();

            let (pin, now) = self.pin_read();
            let tx = ReadTx::new(now);
            match f(tx) {
                Ok(o) => {
                    drop(pin);
                    break o;
                }
                Err(Error::RETRY) => {}
            }

            stats::read_transaction_failure();
        }
    }

    /// Runs a read-write transaction. Requires the thread to not be in a transaction, and the
    /// thread pointer to still be valid.
    #[inline]
    pub unsafe fn rw_slow<'tcell, F, O>(&self, mut f: F) -> O
    where
        F: FnMut(&mut RWTx<'tcell>) -> Result<O, Error>,
    {
        loop {
            stats::write_transaction();

            self.tx_logs().as_mut().validate_start_state();
            let pin = self.pin_rw();
            let tx = RWTx::new(RWThreadKey {
                thread: self.thread,
            });
            let r = f(tx);
            if likely!(r.is_ok()) {
                if let Ok(o) = r {
                    let quiesce_epoch = tx.commit();
                    let unpinned = pin.unpin();
                    if likely!(quiesce_epoch.is_some()) {
                        if let Some(quiesce_epoch) = quiesce_epoch {
                            unpinned.success(quiesce_epoch);
                            self.tx_logs().as_mut().validate_start_state();
                            return o;
                        }
                    }
                }
            }

            stats::write_transaction_failure();
        }
    }

    #[inline]
    fn pin_read(&self) -> (PinRead<'_>, QuiesceEpoch) {
        PinRead::new(unsafe { &(*self.thread.as_ptr()).synch.current_epoch })
    }

    #[inline]
    fn pin_rw(&self) -> PinRw<'_> {
        PinRw::new(ThreadKeyInner {
            thread: self.thread,
        })
    }
}

#[derive(Copy, Clone, Debug)]
pub struct RWThreadKey {
    thread: NonNull<Thread>,
}

impl RWThreadKey {
    /// Returns a raw pointer to the transaction logs (read/write/thread garbage).
    #[inline]
    pub fn tx_logs(self) -> NonNull<TxLogs> {
        tx_logs(self.thread)
    }

    /// Gets the currently pinned epoch. Requires the thread pointer to still be valid.
    #[inline]
    pub unsafe fn pinned_epoch(self) -> QuiesceEpoch {
        // current_epoch is never changed by any thread except the owning thread, so we're able to
        // read it without synchronization.
        let pinned_epoch = synch(self.thread).as_mut().current_epoch.get_unsync();
        debug_assert!(
            pinned_epoch.is_active(),
            "attempt to get pinned_epoch of thread that is not pinned"
        );
        debug_assert!(
            pinned_epoch != QuiesceEpoch::end_of_time(),
            "attempt to get pinned_epoch of thread that is not pinned"
        );
        pinned_epoch
    }
}

// TODO: optimize memory layout
#[repr(C)]
pub struct TxLogs {
    pub read_log:  ReadLog,
    pub write_log: WriteLog,
    pub garbage:   ThreadGarbage,
}

impl TxLogs {
    #[inline]
    fn new() -> Self {
        TxLogs {
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
impl Drop for TxLogs {
    fn drop(&mut self) {
        self.validate_start_state();
    }
}

struct PinRead<'a> {
    current_epoch: &'a AtomicQuiesceEpoch,
}

impl<'a> PinRead<'a> {
    #[inline]
    fn new(current_epoch: &'a AtomicQuiesceEpoch) -> (Self, QuiesceEpoch) {
        let now = EPOCH_CLOCK.now(Acquire);
        unsafe { current_epoch.activate(now, Release) };
        (PinRead { current_epoch }, now)
    }
}

impl<'a> Drop for PinRead<'a> {
    #[inline]
    fn drop(&mut self) {
        self.current_epoch.deactivate(Release)
    }
}

struct PinRw<'a> {
    thread:  ThreadKeyInner,
    phantom: PhantomData<&'a mut ()>,
}

impl<'a> PinRw<'a> {
    #[inline]
    fn new(thread: ThreadKeyInner) -> Self {
        let now = EPOCH_CLOCK.now(Acquire);
        unsafe {
            thread.synch().as_ref().current_epoch.activate(now, Release);

            PinRw {
                thread,
                phantom: PhantomData,
            }
        }
    }

    #[inline]
    fn unpin(self) -> UnpinRw<'a> {
        unsafe {
            let thread = ThreadKeyInner {
                thread: self.thread.thread,
            };
            mem::forget(self);
            thread
                .synch()
                .as_ref()
                .current_epoch
                .set(QuiesceEpoch::end_of_time(), Release);
            UnpinRw {
                thread,
                phantom: PhantomData,
            }
        }
    }
}

impl Drop for PinRw<'_> {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            self.thread
                .synch()
                .as_ref()
                .current_epoch
                .deactivate(Release);
            let mut tx_state = self.thread.tx_logs();
            let tx_state = tx_state.as_mut();
            tx_state.read_log.clear();
            tx_state.garbage.abort_speculative_garbage();
            tx_state.write_log.clear();
        }
    }
}

struct UnpinRw<'a> {
    thread:  ThreadKeyInner,
    phantom: PhantomData<&'a mut ()>,
}

impl Drop for UnpinRw<'_> {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            let mut tx_state = self.thread.tx_logs();
            let tx_state = tx_state.as_mut();
            tx_state.read_log.clear();
            tx_state.garbage.abort_speculative_garbage();
            tx_state.write_log.clear();
            self.thread
                .synch()
                .as_ref()
                .current_epoch
                .deactivate(Relaxed);
        }
    }
}

impl UnpinRw<'_> {
    #[inline]
    fn success(self, quiesce_epoch: QuiesceEpoch) {
        unsafe {
            let mut tx_state = self.thread.tx_logs();
            let tx_state = tx_state.as_mut();
            let synch = self.thread.synch();
            let synch = synch.as_ref();
            mem::forget(self);
            tx_state.read_log.clear();
            tx_state.write_log.clear_no_drop();
            tx_state.garbage.seal_with_epoch(synch, quiesce_epoch);
            synch.current_epoch.deactivate(Relaxed);
        }
    }
}
