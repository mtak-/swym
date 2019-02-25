use crate::internal::{
    alloc::{fvec::Error, FVec},
    gc::quiesce::synch::Synch,
};
use std::ptr::NonNull;

pub struct ThreadList {
    threads: FVec<NonNull<Synch>>,
}

impl ThreadList {
    pub(crate) fn new() -> Result<Self, Error> {
        Ok(ThreadList {
            threads: FVec::new()?,
        })
    }

    #[inline]
    pub fn register(&mut self, thread: NonNull<Synch>) {
        self.threads.push(thread)
    }

    #[inline]
    pub unsafe fn unregister(&mut self, thread: NonNull<Synch>) {
        let global = &mut self.threads;
        for i in 0..global.len() {
            let elem = *global.rget_mut_unchecked(i);
            if unlikely!(elem == thread) {
                global.rswap_erase_unchecked(i);
                return;
            }
        }

        unreach!("failed to find thread in the global thread list")
    }

    #[inline]
    pub(crate) unsafe fn iter<'a>(&'a self) -> impl Iterator<Item = &Synch> + 'a {
        self.threads.iter().map(#[inline(always)]
        |p| p.as_ref())
    }
}
