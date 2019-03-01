use crate::internal::{alloc::FVec, gc::quiesce::synch::Synch};
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
    pub unsafe fn register(&mut self, synch: &Synch) {
        self.synchs.push(synch.into())
    }

    /// Unregisters a destructing synch.
    #[inline]
    pub fn unregister(&mut self, to_remove: NonNull<Synch>) {
        let position = self
            .synchs
            .iter()
            .rev()
            .position(|&synch| synch == to_remove);

        match position {
            Some(position) => unsafe {
                // safe since self.synchs has not been modified
                self.synchs.rswap_erase_unchecked(position);
            },
            None => {
                if cfg!(debug_assertions) {
                    panic!("failed to find thread in the global thread list")
                }
            }
        }
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
