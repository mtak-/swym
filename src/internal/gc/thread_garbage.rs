use crate::internal::{
    alloc::{DynVec, FVec},
    epoch::QuiesceEpoch,
    gc::{
        queued::{FnOnceish, Queued},
        quiesce::OwnedSynch,
    },
};
use std::{
    mem::{self, ManuallyDrop},
    num::NonZeroUsize,
};

// TODO: measure to see what works best in practice.
const UNUSED_BAG_COUNT: usize = 64;

/// A contiguous container of trash.
struct Bag {
    queued: DynVec<dyn FnOnceish + 'static>,
}

impl Bag {
    #[inline]
    fn new() -> Self {
        Bag {
            queued: DynVec::new(),
        }
    }

    #[inline]
    fn seal(self, quiesce_epoch: QuiesceEpoch) -> SealedBag {
        debug_assert!(!self.queued.is_empty(), "attempt to seal an empty `Bag`");
        SealedBag {
            bag: self,
            quiesce_epoch,
        }
    }

    #[inline]
    fn collect(&mut self) {
        // the closures are only safe to call once, drain ensures, that is the case
        unsafe {
            for mut closure in self.queued.drain() {
                closure.call()
            }
        }
    }
}

#[repr(C)]
struct SealedBag {
    quiesce_epoch: QuiesceEpoch,
    bag:           Bag,
}

/// Collection of reusable bags.
struct UnusedBags {
    bags: FVec<Bag>,
}

impl UnusedBags {
    #[inline]
    fn new() -> Self {
        // We add 1 here because FVec allocates on the push when it becomes full, instead of the
        // push after
        let capacity = NonZeroUsize::new(UNUSED_BAG_COUNT + 1).unwrap();
        let mut result = UnusedBags {
            bags: FVec::with_capacity(capacity),
        };

        for _ in 0..UNUSED_BAG_COUNT {
            result.bags.push(Bag::new());
        }
        result
    }

    /// Gets an empty bag out of the collection.
    #[inline]
    fn open_bag(&mut self) -> Option<Bag> {
        self.bags.pop().map(|bag| {
            debug_assert!(bag.queued.is_empty(), "opened up a non-empty `Bag`");
            bag
        })
    }

    /// Gets an empty bag out of the collection.
    #[inline]
    unsafe fn open_bag_unchecked(&mut self) -> Bag {
        let bag = self.bags.pop_unchecked();
        debug_assert!(bag.queued.is_empty(), "opened up a non-empty `Bag`");
        bag
    }

    /// Runs collect on a bag, and puts it back into the collection.
    ///
    /// Bag must have been acquired using `self.open_bag_unchecked()`
    #[inline]
    unsafe fn recycle_bag_unchecked(&mut self, bag: Bag) {
        // to ensure that a bag is never "lost", the `collect()`, which runs arbitrary user code,
        // must be called after the bag has been pushed
        self.bags.push_unchecked(bag);
        let bag = self.bags.back_unchecked();
        debug_assert!(
            !bag.queued.is_empty(),
            "unexpectedly collecting an empty bag"
        );
        // the Drain iter, leaves the bag empty, even if there is a panic
        bag.collect();
        debug_assert!(bag.queued.is_empty(), "bag not empty after collection");
    }
}

/// Thread local garbage
#[repr(C)]
pub struct ThreadGarbage {
    /// The bag that new trash will be pushed to.
    speculative_bag: Bag,

    /// Bags that have been queued up for collection.
    sealed_bags: FVec<SealedBag>,

    /// Store of unused bags from which new bags can be obtained.
    unused_bags: UnusedBags,
}

impl ThreadGarbage {
    #[inline]
    pub fn new() -> Self {
        let mut unused_bags = UnusedBags::new();
        debug_assert!(!unused_bags.bags.is_empty());
        let speculative_bag = unused_bags
            .open_bag()
            .expect("ThreadGarbage ran out of unused bagss");
        let sealed_capacity = NonZeroUsize::new(UNUSED_BAG_COUNT).unwrap();
        ThreadGarbage {
            speculative_bag,
            sealed_bags: FVec::with_capacity(sealed_capacity),
            unused_bags,
        }
    }

    /// Checks if there is any speculative trash.
    #[inline]
    pub fn is_speculative_bag_empty(&self) -> bool {
        self.speculative_bag.queued.is_empty()
    }

    /// Leaks all the trash in the speculative bag.
    ///
    /// When running a transaction, trash is queued up speculatively. If the transaction fails, none
    /// of the garbage that was queued up should be collected.
    #[inline]
    pub fn abort_speculative_garbage(&mut self) {
        self.speculative_bag.queued.clear_no_drop()
    }

    /// Used to help move allocations out of the fast path.
    #[inline]
    pub fn next_dispose_allocates<T: 'static + Send>(&self) -> bool {
        self.speculative_bag
            .queued
            .next_push_allocates::<Queued<T>>()
    }

    /// Queues value up to be dropped should the current transaction succeed.
    #[inline]
    pub fn dispose<T: 'static + Send>(&mut self, value: ManuallyDrop<T>) {
        self.speculative_bag.queued.push(Queued::new(value))
    }

    /// Queues value up to be dropped should the current transaction succeed.
    ///
    /// Assumes that the current bag has a large enough capacity to store the new garbage.
    #[inline]
    pub unsafe fn dispose_unchecked<T: 'static + Send>(&mut self, value: ManuallyDrop<T>) {
        self.speculative_bag
            .queued
            .push_unchecked(Queued::new(value))
    }

    /// Ends the speculative garbage queuing, and commits the current_bag's garbage to be collected
    /// at some point. May modify synch's current_epoch.
    ///
    /// This potentially will cause garbage collection to happen.
    #[inline]
    pub unsafe fn seal_with_epoch(&mut self, synch: &OwnedSynch, quiesce_epoch: QuiesceEpoch) {
        debug_assert!(
            quiesce_epoch.is_active(),
            "attempt to seal with an \"inactive\" epoch"
        );
        if unlikely!(!self.is_speculative_bag_empty()) {
            self.seal_with_epoch_slow(synch, quiesce_epoch)
        }
    }

    /// If the current bag was not empty, we have real work to do.
    #[inline(never)]
    unsafe fn seal_with_epoch_slow(&mut self, synch: &OwnedSynch, quiesce_epoch: QuiesceEpoch) {
        let new_bag = self.unused_bags.open_bag_unchecked();
        let prev_bag = mem::replace(&mut self.speculative_bag, new_bag);
        let sealed_bag = prev_bag.seal(quiesce_epoch);
        self.sealed_bags.push_unchecked(sealed_bag);

        // If we've run out of unused bags/room in the sealed_bags container, which should be the
        // same, then we need to collect garbage to make room for the next transaction.
        if unlikely!(self.sealed_bags.next_push_allocates()) {
            self.synch_and_collect(synch)
        }
    }

    /// This is guaranteed to atleast collect the first bag from the garbage, but may wind up
    /// collecting more.
    #[inline(never)]
    #[cold]
    unsafe fn synch_and_collect(&mut self, synch: &OwnedSynch) {
        self.synch_and_collect_impl(synch, self.earliest_epoch_unchecked())
    }

    /// This is guaranteed to collect all of the garbage that has been queued. May modify synch's
    /// current_epoch.
    ///
    /// It is used in the destructor for ThreadKey.
    #[inline(never)]
    #[cold]
    pub unsafe fn synch_and_collect_all(&mut self, synch: &OwnedSynch) {
        if !self.sealed_bags.is_empty() {
            self.synch_and_collect_impl(synch, self.latest_epoch_unchecked())
        }
    }

    /// Synchronizes with all the other threads participating in the STM, then collects the garbage.
    /// Modifies synch's current_epoch.
    #[inline]
    unsafe fn synch_and_collect_impl(&mut self, synch: &OwnedSynch, quiesce_epoch: QuiesceEpoch) {
        // we want to collect atleast through quiesce_epoch, but it's possible that `quiesce` can
        // detect that even more garbage is able to be collected.
        let collect_epoch = synch.freeze_list().quiesce(quiesce_epoch);
        self.collect(collect_epoch);
    }

    // collects garbage from epochs: [0..max_epoch)
    #[inline(never)]
    #[cold]
    pub unsafe fn collect(&mut self, max_epoch: QuiesceEpoch) {
        debug_assert!(
            self.is_speculative_bag_empty(),
            "`collect` called while current bag is not empty"
        );
        debug_assert!(
            !self.sealed_bags.is_empty(),
            "`collect` called with no sealed bags"
        );

        let drain_iter = self
            .sealed_bags
            .drain_while(move |sealed_bag| sealed_bag.quiesce_epoch < max_epoch);
        for sealed_bag in drain_iter {
            // TODO: NESTING: tx's can start here
            self.unused_bags.recycle_bag_unchecked(sealed_bag.bag);
        }
    }

    #[inline]
    pub unsafe fn earliest_epoch_unchecked(&self) -> QuiesceEpoch {
        debug_assert!(
            !self.sealed_bags.is_empty(),
            "`earliest_epoch_unchecked` called with no sealed bags"
        );
        self.sealed_bags.get_unchecked(0).quiesce_epoch
    }

    #[inline]
    pub unsafe fn latest_epoch_unchecked(&self) -> QuiesceEpoch {
        debug_assert!(
            !self.sealed_bags.is_empty(),
            "`earliest_epoch_unchecked` called with no sealed bags"
        );
        self.sealed_bags
            .get_unchecked(self.sealed_bags.len() - 1)
            .quiesce_epoch
    }
}

#[cfg(debug_assertions)]
impl Drop for ThreadGarbage {
    fn drop(&mut self) {
        debug_assert!(
            self.speculative_bag.queued.is_empty(),
            "dropping the thread garbage while the current bag is not empty"
        );
        debug_assert!(
            self.sealed_bags.is_empty(),
            "dropping the thread garbage while there is still uncollected garbage"
        );
    }
}
