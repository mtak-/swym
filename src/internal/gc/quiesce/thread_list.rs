use crate::internal::{
    alloc::{fvec::Error, FVec},
    gc::quiesce::synch::Synch,
};
use std::ptr::NonNull;

/// A list of pointers to each threads Synch (sharded lock and current epoch)
pub struct ThreadList {
    threads: FVec<NonNull<Synch>>,
}

impl ThreadList {
    pub fn new() -> Result<Self, Error> {
        Ok(ThreadList {
            threads: FVec::new()?,
        })
    }

    /// Registers a new thread for participation in the STM.
    #[inline]
    pub fn register(&mut self, thread: NonNull<Synch>) {
        self.threads.push(thread)
    }

    /// Unregisters a destructing thread.
    #[inline]
    pub unsafe fn unregister(&mut self, thread: NonNull<Synch>) {
        let global = &mut self.threads;
        for i in 0..global.len() {
            // accessing elements is negligibly faster using rget_*
            let elem = *global.rget_mut_unchecked(i);
            if unlikely!(elem == thread) {
                global.rswap_erase_unchecked(i);
                return;
            }
        }

        unreach!("failed to find thread in the global thread list")
    }

    #[inline]
    pub unsafe fn iter<'a>(&'a self) -> impl Iterator<Item = &Synch> + 'a {
        self.threads.iter().map(|p| p.as_ref())
    }
}
