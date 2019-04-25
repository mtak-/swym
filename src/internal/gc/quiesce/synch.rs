use crate::internal::{
    epoch::{QuiesceEpoch, ThreadEpoch},
    gc::quiesce::GlobalSynchList,
};
use lock_api::RawRwLock as _;
use parking_lot::RawRwLock;
use std::{
    ptr,
    sync::atomic::Ordering::{self, Acquire},
};

/// A sharded lock protecting the GlobalThreadList, and the current epoch (or inactive) for the
/// owning thread.
///
/// To modify the GlobalThreadList all Synchs must be locked. To read from the GlobalThreadList,
/// only the current threads Synch needs to be locked.
///
/// Synch provides read only access to current_epoch, whereas, OwnedSynch has read-write access.
///
/// TODO: optimize layout
#[repr(C)]
pub struct Synch {
    /// The currently pinned epoch, or INACTIVE_EPOCH
    current_epoch: ThreadEpoch,

    /// The sharded lock protecting the GlobalThreadList
    lock: RawRwLock,
}

impl Synch {
    #[inline]
    fn new() -> Self {
        let lock = RawRwLock::INIT;
        lock.lock_shared();
        Synch {
            current_epoch: ThreadEpoch::inactive(),
            // Synchs are created in a locked state, since the GlobalThreadList is assumed to be
            // locked, whenever a Synch is created.
            //
            // Immediately after creation the Synch is added to the GlobalThreadList where it will
            // be unlocked, when the list is unlocked.
            lock,
        }
    }

    /// Checks that the thread owning this Synch is no longer accessing data that existed before the
    /// quiesce epoch.
    #[inline]
    pub fn is_quiesced(&self, quiesce_epoch: QuiesceEpoch) -> bool {
        // TODO: acquire seems unneeded, but syncs with release in activate_epoch
        self.current_epoch.is_quiesced(quiesce_epoch, Acquire)
    }

    /// Waits until the thread owning this Synch is no longer accessing data that existed before
    /// quiesce epoch.
    ///
    /// The calling thread must be different from the thread owning self, or self
    /// must be inactive, else deadlock.
    #[inline(never)]
    #[cold]
    pub(super) fn local_quiesce(&self, quiesce_epoch: QuiesceEpoch) {
        loop {
            // backoff.snooze();
            if self.is_quiesced(quiesce_epoch) {
                break;
            }
        }
    }

    #[inline]
    pub(super) fn lock(&self) {
        self.lock.lock_exclusive()
    }

    #[inline]
    pub(super) fn unlock(&self) {
        self.lock.unlock_exclusive()
    }
}

pub struct OwnedSynch {
    pub(super) inner: Synch,
}

impl !Sync for OwnedSynch {}

impl OwnedSynch {
    #[inline]
    pub fn new() -> Self {
        OwnedSynch {
            inner: Synch::new(),
        }
    }

    /// Acquires the Synch's lock allowing read only access to the GlobalThreadList.
    ///
    /// Requires that self is registered to the GlobalThreadList
    #[inline]
    pub unsafe fn freeze_list<'a>(&'a self) -> FreezeList<'a> {
        FreezeList::new(self)
    }

    /// Gets the value of the currently pinned epoch (or returns an inactive epoch).
    #[inline]
    pub fn current_epoch(&self) -> QuiesceEpoch {
        let epoch_ptr = &self.inner.current_epoch as *const ThreadEpoch as *const QuiesceEpoch;
        // safe since we only allow mutation through OwnedSynch (which does not implement sync)
        unsafe { *epoch_ptr }
    }

    #[inline]
    pub fn pin(&self, epoch: QuiesceEpoch, o: Ordering) {
        self.inner.current_epoch.pin(epoch, o)
    }

    #[inline]
    pub fn repin(&self, epoch: QuiesceEpoch, o: Ordering) {
        self.inner.current_epoch.repin(epoch, o)
    }

    #[inline]
    pub fn unpin(&self, o: Ordering) {
        self.inner.current_epoch.unpin(o)
    }
}

/// A read only guard for the GlobalSynchList.
pub struct FreezeList<'a> {
    lock: &'a RawRwLock,
}

impl<'a> FreezeList<'a> {
    /// Creating a new freezelist requires the Synch to have been registered to the GlobalSynchList
    #[inline]
    unsafe fn new(synch: &'a OwnedSynch) -> Self {
        // setting a dummy epoch that is far in the future, will protect against transactions
        // running during garbage collection.
        synch.inner.current_epoch.set_collect_epoch();

        let lock = &synch.inner.lock;
        lock.lock_shared();
        debug_assert!(
            GlobalSynchList::instance()
                .raw()
                .iter()
                .find(|&lhs| ptr::eq(lhs, &synch.inner))
                .is_some(),
            "bug: synch not registered to the GlobalSynchList"
        );

        FreezeList { lock }
    }

    /// Returns true if the synchs lock is currently held by self.
    #[inline]
    fn requested_by(&self, synch: &Synch) -> bool {
        ptr::eq(self.lock, &synch.lock)
    }

    /// Waits for all threads to pass `epoch` (or go inactive) and then returns the minimum active
    /// epoch.
    ///
    /// The result is always greater than `epoch`, (must wait for threads who have a lesser
    /// epoch).
    #[inline]
    pub fn quiesce(&self, epoch: QuiesceEpoch) -> QuiesceEpoch {
        // we hold one of the sharded locks, so read access is safe.
        let synchs = unsafe { GlobalSynchList::instance_unchecked().raw().iter() };
        let result = synchs
            .flat_map(|synch| {
                let td_epoch = synch.current_epoch.get(Acquire);

                debug_assert!(
                    !self.requested_by(synch) || td_epoch > epoch,
                    "deadlock detected. `wait_until_epoch_end` called by an active thread"
                );

                if likely!(td_epoch > epoch) {
                    Some(td_epoch)
                } else {
                    // after quiescing, the thread owning `synch` will have entered the
                    // INACTIVE_EPOCH atleast once, so there's no need to update result
                    synch.local_quiesce(epoch);
                    None
                }
            })
            .min()
            .unwrap_or(QuiesceEpoch::max_value());

        debug_assert!(
            result >= epoch,
            "bug: quiesced to an epoch less than the requested epoch"
        );
        result
    }
}

impl<'a> Drop for FreezeList<'a> {
    #[inline]
    fn drop(&mut self) {
        self.lock.unlock_shared()
    }
}
