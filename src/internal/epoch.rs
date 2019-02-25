use std::{
    fmt::{self, Debug, Formatter},
    mem,
    num::NonZeroUsize,
    sync::atomic::{
        AtomicUsize,
        Ordering::{self, Relaxed},
    },
};

type Storage = usize;
type NonZeroStorage = NonZeroUsize;
type AtomicStorage = AtomicUsize;

const INACTIVE_EPOCH: QuiesceEpoch = QuiesceEpoch(unsafe { NonZeroStorage::new_unchecked(!0) });
const LOCK_BIT: Storage = 1 << (mem::size_of::<Storage>() as Storage * 8 - 1);
const FIRST: Storage = 1;

#[inline]
const fn lock_bit_set(e: Storage) -> bool {
    e & LOCK_BIT != 0
}

#[inline]
const fn as_unlocked(e: Storage) -> Storage {
    e & !LOCK_BIT
}

#[inline]
const fn toggle_lock_bit(e: Storage) -> Storage {
    e ^ LOCK_BIT
}

/// This type holds
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QuiesceEpoch(NonZeroStorage);

impl QuiesceEpoch {
    #[inline]
    unsafe fn new_unchecked(epoch: Storage) -> Self {
        debug_assert!(
            epoch >= FIRST,
            "creating a `QuieseEpoch` before the start of time"
        );
        debug_assert!(
            !lock_bit_set(epoch) || epoch == INACTIVE_EPOCH.0.get(),
            "creating a locked `QuieseEpoch` is a logic error"
        );
        QuiesceEpoch(NonZeroStorage::new_unchecked(epoch))
    }

    #[inline]
    pub const fn inactive() -> Self {
        INACTIVE_EPOCH
    }

    #[inline]
    pub const fn max_value() -> Self {
        INACTIVE_EPOCH
    }

    #[inline]
    pub unsafe fn end_of_time() -> Self {
        let r = QuiesceEpoch(NonZeroStorage::new_unchecked(!LOCK_BIT));
        debug_assert!(
            r.is_active(),
            "`QuiesceEpoch::end_of_time` returned an invalid epoch"
        );
        r
    }

    #[inline]
    pub fn first() -> Self {
        unsafe { QuiesceEpoch::new_unchecked(FIRST) }
    }

    #[inline]
    fn read_write_valid(self, target: NonZeroStorage) -> bool {
        self.0 >= target
    }

    #[inline]
    pub fn read_write_valid_lockable(self, target: &EpochLock, o: Ordering) -> bool {
        self.read_write_valid(target.load_raw(o))
    }

    #[inline]
    pub fn tick_size() -> NonZeroStorage {
        unsafe { NonZeroStorage::new_unchecked(1) }
    }

    #[inline]
    pub fn is_active(self) -> bool {
        self != INACTIVE_EPOCH
    }

    // unsafe due to overflow + NonZeroUsize
    #[inline]
    pub unsafe fn next(self) -> Self {
        QuiesceEpoch::new_unchecked(self.0.get() + QuiesceEpoch::tick_size().get())
    }
}

pub struct EpochLock(AtomicStorage);

impl Debug for EpochLock {
    #[inline(never)]
    #[cold]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let e = self.load_raw(Relaxed);
        f.debug_struct("EpochLock")
            .field("locked", &lock_bit_set(e.get()))
            .field("epoch", &as_unlocked(e.get()))
            .finish()
    }
}

impl EpochLock {
    #[inline]
    pub const fn first() -> Self {
        EpochLock(AtomicStorage::new(FIRST))
    }

    #[inline]
    fn load_raw(&self, o: Ordering) -> NonZeroStorage {
        let value = self.0.load(o);
        debug_assert!(value != 0, "`EpochLock` unexpectedly had 0 as the `Epoch`");
        unsafe { NonZeroStorage::new_unchecked(value) }
    }

    #[inline]
    #[must_use]
    pub fn try_lock(&self, max_expected: QuiesceEpoch, so: Ordering, fo: Ordering) -> bool {
        debug_assert!(
            max_expected.is_active(),
            "invalid max_expected epoch sent to `EpochLock::try_lock`"
        );
        let actual = self.load_raw(Relaxed); // could be a torn read, if MM permitted it
        likely!(max_expected.read_write_valid(actual))
            && likely!({
                unsafe {
                    assume!(
                        !lock_bit_set(actual.get()),
                        "lock bit unexpectedly set on `EpochLock`"
                    )
                };
                self.0
                    .compare_exchange(actual.get(), toggle_lock_bit(actual.get()), so, fo)
                    .is_ok()
            })
    }

    // certain values of QuiesceEpoch are overloaded to mean "locked", making this function unsafe
    // additionally requires that this is locked by calling thread
    #[inline]
    pub unsafe fn unlock_as_epoch(&self, epoch: QuiesceEpoch, o: Ordering) {
        debug_assert!(
            lock_bit_set(self.0.load(Relaxed)),
            "attempt to unlock an unlocked EpochLock"
        );
        debug_assert!(
            epoch.is_active(),
            "attempt to unlock an EpochLock to the inactive state"
        );
        debug_assert!(
            !lock_bit_set(epoch.0.get()),
            "attempt to unlock an EpochLock with a locked epoch"
        );
        self.0.store(epoch.0.get(), o);
    }

    // UB if not locked by calling thread
    #[inline]
    pub unsafe fn unlock(&self, o: Ordering) {
        // TODO: bench this?
        // we have the lock exclusively, other threads _cannot_ mutate it, so this is safe
        // x86_64 should wind up with a `bzhiq` after these shenanigans (tighter instructions)
        let prev = *(*(&self.0 as *const AtomicStorage as *mut AtomicStorage)).get_mut();
        assume!(
            lock_bit_set(prev),
            "lock bit unexpectedly not set on `EpochLock`"
        );
        self.unlock_as_epoch(
            QuiesceEpoch(NonZeroStorage::new_unchecked(toggle_lock_bit(prev))),
            o,
        );
    }
}

#[derive(Debug)]
pub struct AtomicQuiesceEpoch(AtomicStorage);

impl AtomicQuiesceEpoch {
    #[inline]
    pub fn inactive() -> Self {
        AtomicQuiesceEpoch(AtomicStorage::new(INACTIVE_EPOCH.0.get()))
    }

    #[inline]
    pub fn get(&self, o: Ordering) -> QuiesceEpoch {
        unsafe { QuiesceEpoch::new_unchecked(self.0.load(o)) }
    }

    #[inline]
    pub fn get_unsync(&mut self) -> QuiesceEpoch {
        unsafe { QuiesceEpoch::new_unchecked(*self.0.get_mut()) }
    }

    #[inline]
    pub fn is_active_unsync(&mut self) -> bool {
        self.get_unsync().is_active()
    }

    #[inline]
    pub fn set(&self, value: QuiesceEpoch, o: Ordering) {
        self.0.store(value.0.get(), o)
    }

    // certain values of QuiesceEpoch are overloaded to mean "locked", making this function unsafe
    // additionally requires that self is not active
    #[inline]
    pub unsafe fn activate(&self, epoch: QuiesceEpoch, o: Ordering) {
        debug_assert!(
            !self.get(Relaxed).is_active(),
            "already active AtomicQuiesceEpoch"
        );
        debug_assert!(
            epoch.is_active(),
            "cannot activate an AtomicQuiesceEpoch to the inactive state"
        );
        debug_assert!(
            !lock_bit_set(epoch.0.get()),
            "cannot activate an AtomicQuiesceEpoch to a locked state"
        );
        self.set(epoch, o);
    }

    #[inline]
    pub fn deactivate(&self, o: Ordering) {
        debug_assert!(
            self.get(Relaxed).is_active(),
            "attempt to deactive an already inactive AtomicQuiesceEpoch"
        );
        self.set(INACTIVE_EPOCH, o)
    }
}

#[derive(Debug)]
pub struct EpochClock(AtomicStorage);

// TODO: stick this in quiescelist???
pub static EPOCH_CLOCK: EpochClock = EpochClock::new();

impl EpochClock {
    #[inline]
    const fn new() -> EpochClock {
        EpochClock(AtomicStorage::new(FIRST))
    }

    #[inline]
    pub fn now(&self, o: Ordering) -> QuiesceEpoch {
        unsafe { QuiesceEpoch::new_unchecked(self.0.load(o)) }
    }

    // unsafe due to overflow + NonZeroUsize
    #[inline]
    pub unsafe fn fetch_and_tick(&self, o: Ordering) -> QuiesceEpoch {
        let result = self.0.fetch_add(QuiesceEpoch::tick_size().get(), o);
        debug_assert!(
            result < EpochClock::max_version().0.get() - QuiesceEpoch::tick_size().get(),
            "potential `EpochClock` overflow detected"
        );
        QuiesceEpoch::new_unchecked(result)
    }

    #[inline]
    fn max_version() -> QuiesceEpoch {
        unsafe {
            QuiesceEpoch::new_unchecked(
                !((1 as Storage)
                    << (mem::size_of::<usize>() as Storage * 8 - QuiesceEpoch::tick_size().get())),
            )
        }
    }
}
