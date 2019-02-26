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

/// A synchronized ThreadList. Synchronization is provided by the sharded locks in Synch, and the
/// outer mutex
// TODO: Does repr(C) give better asm? Also repr(align(64)) might be helpful.
#[repr(C)]
pub struct GlobalThreadList {
    /// The list of threads participating in the STM.
    thread_list: UnsafeCell<ThreadList>,

    /// This mutex is only grabbed before modifying to the GlobalThreadList, and still requires
    /// every threads lock to be acquired before any mutations.
    mutex: FrwLock,
}

unsafe impl Sync for GlobalThreadList {}

// Once allocated the SINGLETON is never deallocated.
static SINGLETON: AtomicPtr<GlobalThreadList> = AtomicPtr::new(0 as _);

impl GlobalThreadList {
    // slow path
    #[inline(never)]
    #[cold]
    fn init() -> &'static Self {
        // Once handles two threads racing to initialize the GlobalThreadList
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

    /// Unsafe without holding atleast one of the locks in the GlobalThreadList.
    #[inline]
    unsafe fn raw(&self) -> &ThreadList {
        &*self.thread_list.get()
    }

    /// Gets write access to the GlobalThreadList.
    #[inline]
    pub fn write<'a>(&'a self) -> Write<'a> {
        Write::new(self)
    }
}

/// A write guard for the GlobalThreadList.
pub struct Write<'a> {
    list: &'a GlobalThreadList,
}

impl<'a> Write<'a> {
    #[inline]
    fn new(list: &'a GlobalThreadList) -> Self {
        // Atleast one mutex has to be held in order to call `raw` safely.
        // The outer mutex is used for this purpose, so that, under contention, two writers will
        // never deadlock.
        list.mutex.lock_exclusive();
        unsafe {
            // lock all the Synchs to prevent them from creating a FreezeList
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

/// A read only guard for the GlobalThreadList.
pub struct FreezeList<'a> {
    lock: &'a FrwLock,
}

impl<'a> FreezeList<'a> {
    #[inline]
    pub fn new(synch: &'a Synch) -> Self {
        let lock = &synch.lock;
        lock.lock_shared();
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
    pub unsafe fn quiesce(&self, epoch: QuiesceEpoch) -> QuiesceEpoch {
        let mut result = QuiesceEpoch::max_value();

        assume_no_panic! {
            for synch in GlobalThreadList::instance_unchecked().raw().iter() {
                let td_epoch = synch.current_epoch.get(Acquire);

                debug_assert!(
                    !self.requested_by(synch) || td_epoch > epoch,
                    "deadlock detected. `wait_until_epoch_end` called by an active thread"
                );

                if likely!(td_epoch > epoch) {
                    result = result.min(td_epoch);
                } else {
                    // after quiescing, the thread owning `synch` will have entered the
                    // INACTIVE_EPOCH atleast once, so there's no need to update result
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
