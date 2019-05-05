use crate::internal::{
    alloc::FVec,
    gc::quiesce::synch::{OwnedSynch, Synch},
};
use core::ptr::NonNull;

/// A list of pointers to each threads Synch (sharded lock and current epoch)
pub struct SynchList {
    synchs: FVec<NonNull<Synch>>,
}

impl SynchList {
    pub fn new() -> Self {
        SynchList {
            synchs: FVec::new(),
        }
    }

    /// Registers a new synch for participation in the STM.
    ///
    /// Synch must never be moved or dropped, until it is unregistered.
    #[inline]
    pub unsafe fn register(&mut self, to_add: &OwnedSynch) {
        let to_add = &to_add.inner;
        self.synchs.push(to_add.into())
    }

    /// Unregisters a destructing synch.
    ///
    /// Returns true if the OwnedSynch was successfully unregistered, false otherwise.
    #[inline]
    pub fn unregister(&mut self, to_remove: &OwnedSynch) -> bool {
        let to_remove = &to_remove.inner;
        self.synchs
            .iter()
            .position(|&synch| synch == to_remove.into())
            .map(|position| self.synchs.swap_remove(position))
            .is_some()
    }

    #[inline]
    pub(super) fn iter<'a>(
        &'a self,
    ) -> impl ExactSizeIterator<Item = &'a Synch> + DoubleEndedIterator<Item = &'a Synch> {
        self.synchs.iter().map(|p| unsafe {
            // register requires that Synchs aren't moved or dropped until after unregister is
            // called
            p.as_ref()
        })
    }
}
