use crate::internal::{
    epoch::QuiesceEpoch,
    frw_lock::FrwLock,
    gc::quiesce::{synch::Synch, thread_list::ThreadList},
};
use lock_api::RawRwLock;
use std::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    ptr,
    sync::{
        atomic::{
            AtomicPtr,
            Ordering::{Acquire, Relaxed, Release},
        },
        Once,
    },
};

#[repr(C)]
pub struct GlobalThreadList {
    thread_list: UnsafeCell<ThreadList>,
    mutex:       FrwLock,
}

unsafe impl Sync for GlobalThreadList {}

static SINGLETON: AtomicPtr<GlobalThreadList> = AtomicPtr::new(0 as _);

impl GlobalThreadList {
    // slow path
    #[inline(never)]
    #[cold]
    fn init() -> &'static Self {
        static INIT_QUIESCE_LIST: Once = Once::new();

        #[inline(never)]
        #[cold]
        fn do_init() {
            SINGLETON.store(
                Box::into_raw(Box::new(GlobalThreadList {
                    thread_list: UnsafeCell::new(ThreadList::new().unwrap()),
                    mutex:       RawRwLock::INIT,
                })),
                Release,
            );
        }

        INIT_QUIESCE_LIST.call_once(do_init);

        // SINGLETON is leaked, so this is always valid..
        unsafe { Self::instance_unchecked() }
    }

    #[inline]
    pub fn instance() -> &'static Self {
        let raw = SINGLETON.load(Acquire);
        if likely!(!raw.is_null()) {
            unsafe { &*raw }
        } else {
            GlobalThreadList::init()
        }
    }

    // fast path
    #[inline]
    pub unsafe fn instance_unchecked() -> &'static Self {
        let raw = SINGLETON.load(Relaxed);
        debug_assert!(
            !raw.is_null(),
            "`GlobalThreadList::instance_unchecked` called before instance was created"
        );
        &*raw
    }

    #[inline]
    unsafe fn raw(&self) -> &ThreadList {
        &*self.thread_list.get()
    }

    // This uses a per thread mutex to get write access
    // read access is achieved by simply locking the Handle
    #[inline]
    pub fn write<'a>(&'a self) -> Write<'a> {
        Write::new(self)
    }
}

pub struct Write<'a> {
    list: &'a GlobalThreadList,
}

impl<'a> Write<'a> {
    #[inline]
    fn new(list: &'a GlobalThreadList) -> Self {
        list.mutex.lock_exclusive();
        unsafe {
            for synch in list.raw().iter() {
                synch.lock.lock_exclusive();
            }
            Write { list }
        }
    }
}

impl<'a> Drop for Write<'a> {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            for synch in self.list.raw().iter() {
                synch.lock.unlock_exclusive();
            }
        }
        self.list.mutex.unlock_exclusive();
    }
}

impl<'a> Deref for Write<'a> {
    type Target = ThreadList;

    #[inline]
    fn deref(&self) -> &ThreadList {
        unsafe { &*self.list.thread_list.get() }
    }
}

impl<'a> DerefMut for Write<'a> {
    #[inline]
    fn deref_mut(&mut self) -> &mut ThreadList {
        unsafe { &mut *self.list.thread_list.get() }
    }
}

pub struct FreezeList<'a> {
    lock: &'a FrwLock,
}

impl<'a> FreezeList<'a> {
    #[inline]
    pub(crate) fn new(lock: &'a FrwLock) -> Self {
        lock.lock_shared();
        FreezeList { lock }
    }

    #[inline]
    pub fn requested_by(&self, synch: &Synch) -> bool {
        ptr::eq(self.lock, &synch.lock)
    }

    // result is always greater than quiesce_epoch, (must wait for threads who have a lesser epoch)
    #[inline]
    pub unsafe fn quiesce(&self, epoch: QuiesceEpoch) -> QuiesceEpoch {
        let mut result = QuiesceEpoch::max_value();

        assume_no_panic! {
            for synch in GlobalThreadList::instance_unchecked().raw().iter() {
                let td_epoch = synch.current_epoch.get(Acquire);

                debug_assert!(
                    td_epoch > epoch || !self.requested_by(synch),
                    "deadlock detected. `wait_until_epoch_end` called by an active thread"
                );

                if likely!(td_epoch > epoch) {
                    result = result.min(td_epoch);
                } else {
                    synch.local_quiesce(epoch);
                }
            }
        }

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
