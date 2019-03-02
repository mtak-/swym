use crate::{
    internal::{
        epoch::{AtomicQuiesceEpoch, QuiesceEpoch, EPOCH_CLOCK},
        gc::{Synch, ThreadGarbage},
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
/// Synch is aliased in the GlobalSynchList of the garbage collector.
#[repr(C, align(64))]
pub struct Thread {
    tx_state:         TxLogs,
    pub(crate) synch: Synch,
    ref_count:        Cell<usize>,
}

impl Thread {
    #[inline(never)]
    #[cold]
    pub fn new() -> Self {
        Thread {
            tx_state:  TxLogs::new(),
            synch:     Synch::new(),
            ref_count: Cell::new(1),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ThreadKeyRaw {
    thread: NonNull<Thread>,
}

// TODO: is this all necessary to avoid violating aliasing rules?
impl ThreadKeyRaw {
    pub fn new(thread: NonNull<Thread>) -> Self {
        ThreadKeyRaw { thread }
    }

    #[inline]
    pub fn thread(self) -> NonNull<Thread> {
        self.thread
    }

    #[inline]
    pub fn tx_logs(self) -> NonNull<TxLogs> {
        self.thread.cast()
    }

    #[inline]
    pub fn synch(self) -> NonNull<Synch> {
        unsafe {
            self.tx_logs()
                .assume_aligned()
                .add(1)
                .assume_aligned()
                .cast::<Synch>()
                .align_next()
        }
    }

    #[inline]
    pub fn ref_count(self) -> NonNull<Cell<usize>> {
        unsafe {
            self.synch()
                .assume_aligned()
                .add(1)
                .assume_aligned()
                .cast::<Cell<usize>>()
                .align_next()
        }
    }

    #[inline]
    pub unsafe fn read_slow<'tcell, F, O>(self, mut f: F) -> O
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

    #[inline]
    pub unsafe fn rw_slow<'tcell, F, O>(self, mut f: F) -> O
    where
        F: FnMut(&mut RWTx<'tcell>) -> Result<O, Error>,
    {
        loop {
            stats::write_transaction();

            self.tx_logs().as_mut().validate_start_state();
            let pin = self.pin_rw();
            let tx = RWTx::new(self);
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
        PinRw::new(ThreadKeyRaw {
            thread: self.thread,
        })
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

pub struct PinRw<'a> {
    thread:  ThreadKeyRaw,
    phantom: PhantomData<&'a mut ()>,
}

impl<'a> PinRw<'a> {
    #[inline]
    fn new(thread: ThreadKeyRaw) -> Self {
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
            let thread = ThreadKeyRaw {
                thread: self.thread.thread(),
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

pub struct UnpinRw<'a> {
    thread:  ThreadKeyRaw,
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
