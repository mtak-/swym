use crate::internal::{
    alloc::FVec,
    gc::quiesce::synch::{OwnedSynch, Synch},
};
use std::ptr::NonNull;

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
    #[inline]
    pub fn unregister(&mut self, to_remove: &OwnedSynch) {
        let to_remove = &to_remove.inner;
        let position = self
            .synchs
            .iter()
            .rev()
            .position(|&synch| synch == to_remove.into());

        debug_assert!(
            position.is_some(),
            "failed to find thread in the global thread list"
        );

        position.map(|position| unsafe {
            // safe since self.synchs has not been modified
            self.synchs.rswap_erase_unchecked(position);
        });
    }

    #[inline]
    pub(super) fn iter<'a>(&'a self) -> impl Iterator<Item = &Synch> + 'a {
        self.synchs.iter().map(|p| unsafe {
            // register requires that Synchs aren't moved or dropped until after unregister is
            // called
            p.as_ref()
        })
    }
}
