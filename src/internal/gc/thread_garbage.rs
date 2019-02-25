use crate::internal::{
    alloc::{DynVec, FVec},
    epoch::QuiesceEpoch,
    gc::{
        queued::{FnOnceish, Queued},
        quiesce::Synch,
    },
};
use std::{
    mem::{self, ManuallyDrop},
    num::NonZeroUsize,
};

const UNUSED_BAG_COUNT: NonZeroUsize = unsafe { NonZeroUsize::new_unchecked(64) };

struct Bag {
    queued: DynVec<dyn FnOnceish + 'static>,
}

impl Bag {
    #[inline]
    fn new() -> Self {
        Bag {
            queued: DynVec::new().unwrap(),
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
    unsafe fn collect(&mut self) {
        assume!(
            !self.queued.is_empty(),
            "unexpectedly collecting an empty bag"
        );
        for mut closure in self.queued.drain() {
            closure.call()
        }
    }
}

#[repr(C)]
struct SealedBag {
    quiesce_epoch: QuiesceEpoch,
    bag:           Bag,
}

struct UnusedBags {
    bags: FVec<Bag>,
}

impl UnusedBags {
    #[inline]
    fn new() -> Self {
        let mut result = UnusedBags {
            bags: unsafe {
                FVec::with_capacity(NonZeroUsize::new_unchecked(UNUSED_BAG_COUNT.get() + 1))
                    .unwrap()
            },
        };

        unsafe {
            for _ in 0..UNUSED_BAG_COUNT.get() {
                result.bags.push_unchecked(Bag::new());
            }
        }
        result
    }

    #[inline]
    unsafe fn open_bag_unchecked(&mut self) -> Bag {
        let bag = self.bags.pop_unchecked();
        debug_assert!(bag.queued.is_empty(), "checked out a non-empty `Bag`");
        bag
    }

    #[inline]
    unsafe fn recycle_bag_unchecked(&mut self, bag: Bag) {
        self.bags.push_unchecked(bag);
        let bag = self.bags.back_unchecked();

        // collect after pushing, so that we don't lose a bag on panic
        // panicking here does not result in leaking, but it does cause the collection of this
        // garbage to be delayed a lot
        bag.collect();
        debug_assert!(bag.queued.is_empty(), "bag not empty after collection");
    }
}

// TODO: lotsa room to optimize this
#[repr(C)]
pub struct ThreadGarbage {
    current_bag: Bag,
    sealed_bags: FVec<SealedBag>,
    unused_bags: UnusedBags,
}

impl ThreadGarbage {
    #[inline]
    pub fn new() -> Self {
        let mut unused_bags = UnusedBags::new();
        let current_bag = unsafe { unused_bags.open_bag_unchecked() };

        ThreadGarbage {
            current_bag,
            sealed_bags: FVec::with_capacity(UNUSED_BAG_COUNT).unwrap(),
            unused_bags,
        }
    }

    #[inline]
    pub fn is_current_epoch_empty(&self) -> bool {
        self.current_bag.queued.is_empty()
    }

    #[inline]
    pub fn leak_current_epoch(&mut self) {
        self.current_bag.queued.clear()
    }

    #[inline]
    pub fn next_queue_allocates<T: 'static + Send>(&self) -> bool {
        unsafe { self.current_bag.queued.next_push_allocates::<Queued<T>>() }
    }

    #[inline]
    pub fn trash<T: 'static + Send>(&mut self, value: ManuallyDrop<T>) {
        unsafe { self.current_bag.queued.push(Queued::new(value)) }
    }

    #[inline]
    pub unsafe fn trash_unchecked<T: 'static + Send>(&mut self, value: ManuallyDrop<T>) {
        self.current_bag.queued.push_unchecked(Queued::new(value))
    }

    #[inline(never)]
    unsafe fn seal_with_epoch_slow(&mut self, quiesce_epoch: QuiesceEpoch, synch: &Synch) {
        let new_bag = self.unused_bags.open_bag_unchecked();
        let prev_bag = mem::replace(&mut self.current_bag, new_bag);
        let sealed_bag = prev_bag.seal(quiesce_epoch);
        self.sealed_bags.push_unchecked(sealed_bag);

        if unlikely!(self.sealed_bags.next_push_allocates()) {
            self.synch_and_collect(synch)
        }
    }

    #[inline]
    pub unsafe fn seal_with_epoch(&mut self, quiesce_epoch: QuiesceEpoch, synch: &Synch) {
        debug_assert!(
            quiesce_epoch.is_active(),
            "attempt to seal with an \"inactive\" epoch"
        );
        if unlikely!(!self.is_current_epoch_empty()) {
            self.seal_with_epoch_slow(quiesce_epoch, synch)
        }
    }

    // collects garbage from epochs: [0..max_epoch)
    #[inline(never)]
    #[cold]
    pub unsafe fn collect(&mut self, max_epoch: QuiesceEpoch) {
        debug_assert!(
            self.is_current_epoch_empty(),
            "`collect` called while current bag is not empty"
        );
        assume!(
            !self.sealed_bags.is_empty(),
            "`collect` called with no sealed bags"
        );

        let drain_iter =
            self.sealed_bags.drain_while(move |sealed_bag| sealed_bag.quiesce_epoch < max_epoch);
        for sealed_bag in drain_iter {
            // TODO: NESTING: tx's can start here
            self.unused_bags.recycle_bag_unchecked(sealed_bag.bag);
        }
    }

    #[inline]
    unsafe fn synch_and_collect_impl(&mut self, synch: &Synch, quiesce_epoch: QuiesceEpoch) {
        debug_assert!(
            synch.is_quiesced(quiesce_epoch),
            "attempt to collect garbage while active"
        );
        let collect_epoch = synch.freeze_list().quiesce(quiesce_epoch);
        self.collect(collect_epoch);
    }

    #[inline(never)]
    #[cold]
    unsafe fn synch_and_collect(&mut self, synch: &Synch) {
        self.synch_and_collect_impl(synch, self.earliest_epoch_unchecked())
    }

    #[inline(never)]
    #[cold]
    pub unsafe fn synch_and_collect_all(&mut self, synch: &Synch) {
        if !self.sealed_bags.is_empty() {
            self.synch_and_collect_impl(synch, self.latest_epoch_unchecked())
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
            self.current_bag.queued.is_empty(),
            "dropping the thread garbage while the current bag is not empty"
        );
        debug_assert!(
            self.sealed_bags.is_empty(),
            "dropping the thread garbage while there is still uncollected garbage"
        );
    }
}
