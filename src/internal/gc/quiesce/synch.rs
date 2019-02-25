use crate::internal::{
    epoch::{AtomicQuiesceEpoch, QuiesceEpoch},
    frw_lock::{self, FrwLock},
    gc::quiesce::global::FreezeList,
};
use std::sync::atomic::Ordering::Acquire;

#[repr(C)]
pub struct Synch {
    pub current_epoch: AtomicQuiesceEpoch,
    pub(crate) lock:   FrwLock,
}

impl Synch {
    #[inline]
    pub fn new() -> Synch {
        Synch {
            current_epoch: AtomicQuiesceEpoch::inactive(),
            lock:          FrwLock::INIT_LOCKED,
        }
    }

    #[inline]
    pub fn is_quiesced(&self, rhs: QuiesceEpoch) -> bool {
        // TODO: acquire seems unneeded, but syncs with release in activate_epoch
        self.current_epoch.get(Acquire) > rhs
    }

    // the thread calling must be different from the thread owning self, or self must be inactive
    #[inline(never)]
    #[cold]
    pub(crate) unsafe fn local_quiesce(&self, quiesce_epoch: QuiesceEpoch) {
        loop {
            frw_lock::backoff();
            if self.is_quiesced(quiesce_epoch) {
                break;
            }
        }
    }

    #[inline]
    pub fn freeze_list<'a>(&'a self) -> FreezeList<'a> {
        FreezeList::new(&self.lock)
    }
}
