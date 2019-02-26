use crate::internal::{
    epoch::{AtomicQuiesceEpoch, QuiesceEpoch},
    frw_lock::{self, FrwLock},
    gc::quiesce::global::FreezeList,
};
use std::sync::atomic::Ordering::Acquire;

/// A sharded lock protecting the GlobalThreadList, and the current epoch (or inactive) for the
/// owning thread.
///
/// To modify the GlobalThreadList all Synchs must be locked. To read from the GlobalThreadList,
/// only the current threads Synch needs to be locked.
///
/// repr(C) is used to help reduce false sharing in `ThreadKey`
#[repr(C)]
pub struct Synch {
    /// The currently pinned epoch, or INACTIVE_EPOCH
    pub current_epoch: AtomicQuiesceEpoch,

    /// The sharded lock protecting the GlobalThreadList
    pub lock: FrwLock,
}

impl Synch {
    #[inline]
    pub fn new() -> Synch {
        Synch {
            current_epoch: AtomicQuiesceEpoch::inactive(),
            // Synchs are created in a locked state, since the GlobalThreadList is assumed to be
            // locked, whenever a Synch is created.
            //
            // Immediately after creation the Synch is added to the GlobalThreadList where it will
            // be unlocked, when the list is unlocked.
            lock: FrwLock::INIT_LOCKED,
        }
    }

    /// Checks that the thread owning this Synch is no longer accessing data that existed before the
    /// quiesce epoch.
    #[inline]
    pub fn is_quiesced(&self, quiesce_epoch: QuiesceEpoch) -> bool {
        // TODO: acquire seems unneeded, but syncs with release in activate_epoch
        self.current_epoch.get(Acquire) > quiesce_epoch
    }

    /// Waits until the thread owning this Synch is no longer accessing data that existed before
    /// quiesce epoch.
    ///
    /// The calling thread must be different from the thread owning self, or self
    /// must be inactive.
    #[inline(never)]
    #[cold]
    pub unsafe fn local_quiesce(&self, quiesce_epoch: QuiesceEpoch) {
        loop {
            frw_lock::backoff();
            if self.is_quiesced(quiesce_epoch) {
                break;
            }
        }
    }

    /// Acquires the Synch's lock allowing read only access to the GlobalThreadList.
    ///
    /// This is only called by the thread that owns self.
    #[inline]
    pub fn freeze_list<'a>(&'a self) -> FreezeList<'a> {
        FreezeList::new(self)
    }
}
