use std::{
    fmt::{self, Debug, Formatter},
    mem,
    num::NonZeroUsize,
    sync::atomic::{
        AtomicUsize,
        Ordering::{self, Relaxed, Release},
    },
};

type Storage = usize;
type NonZeroStorage = NonZeroUsize;
type AtomicStorage = AtomicUsize;

const INACTIVE_EPOCH: Storage = !0;
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
            !lock_bit_set(epoch) || epoch == INACTIVE_EPOCH,
            "creating a locked `QuieseEpoch` is a logic error"
        );
        QuiesceEpoch(NonZeroStorage::new_unchecked(epoch))
    }

    #[inline]
    fn new(epoch: Storage) -> Option<Self> {
        debug_assert!(
            epoch >= FIRST,
            "creating a `QuieseEpoch` before the start of time"
        );
        debug_assert!(
            !lock_bit_set(epoch) || epoch == INACTIVE_EPOCH,
            "creating a locked `QuieseEpoch` is a logic error"
        );
        NonZeroStorage::new(epoch).map(QuiesceEpoch)
    }

    #[inline]
    pub fn max_value() -> Self {
        QuiesceEpoch::new(!0).unwrap()
    }

    #[inline]
    fn end_of_time() -> Self {
        let r = QuiesceEpoch::new(!LOCK_BIT).unwrap();
        debug_assert!(
            r.is_active(),
            "`QuiesceEpoch::end_of_time` returned an invalid epoch"
        );
        r
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
    pub fn is_active(self) -> bool {
        self.0.get() != INACTIVE_EPOCH
    }

    /// Gets the epoch immediately after self.
    ///
    /// This is unsafe due to overflow, and the only way of creating invalid epochs from outside
    /// this module. EpochClock::fetch_and_tick is guaranteed to return epochs with a valid `next`.
    /// So this should only be called on epochs returned from `fetch_and_tick`.
    #[inline]
    pub unsafe fn next(self) -> Self {
        QuiesceEpoch::new_unchecked(self.0.get() + 1)
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
                debug_assert!(
                    !lock_bit_set(actual.get()),
                    "lock bit unexpectedly set on `EpochLock`"
                );
                self.0
                    .compare_exchange(actual.get(), toggle_lock_bit(actual.get()), so, fo)
                    .is_ok()
            })
    }

    // UB if not locked by calling thread
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
        AtomicQuiesceEpoch(AtomicStorage::new(INACTIVE_EPOCH))
    }

    #[inline]
    pub fn get(&self, o: Ordering) -> QuiesceEpoch {
        unsafe { QuiesceEpoch::new_unchecked(self.0.load(o)) }
    }

    #[inline]
    fn set(&self, value: QuiesceEpoch, o: Ordering) {
        self.0.store(value.0.get(), o)
    }

    #[inline]
    pub fn set_collect_epoch(&self) {
        self.set(QuiesceEpoch::end_of_time(), Release)
    }

    #[inline]
    pub fn activate(&self, epoch: QuiesceEpoch, o: Ordering) {
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
        self.set(QuiesceEpoch::new(INACTIVE_EPOCH).unwrap(), o)
    }
}

#[derive(Debug)]
pub struct EpochClock(AtomicStorage);

// TODO: stick this in GlobalSynchList?
pub static EPOCH_CLOCK: EpochClock = EpochClock::new();

impl EpochClock {
    #[inline]
    const fn new() -> EpochClock {
        EpochClock(AtomicStorage::new(FIRST))
    }

    #[inline]
    pub fn now(&self, o: Ordering) -> Option<QuiesceEpoch> {
        let epoch = self.0.load(o);
        if likely!(!lock_bit_set(epoch)) {
            Some(unsafe { QuiesceEpoch::new_unchecked(epoch) })
        } else {
            // Program would probly have to be running for several centuries, see below.
            None
        }
    }

    // increments the clock, and returns the previous epoch
    #[inline]
    pub fn fetch_and_tick(&self) -> QuiesceEpoch {
        // To calculate how long overflow will take:
        // - MSB is reserved for the lock bit
        // - so on 64 bit platforms we have 2^63 * fetch_add_time(ns) / 1000000000(ns/s) /
        //   60(sec/min) / 60(min/hr) / 24(hr/day) / 365(day/yr)
        //  for a fetch_add time of 1ns (actually benched at around 5ns), we get 292.46yrs
        //  on 32 bit platforms we get just over 1sec until overflow (fetch_add=1ns)
        //
        // When this overflow happens, the lock bit becomes set for additional epochs causing
        // reads from EpochLocks to succeed even when locked by another thread. This is solved by
        // "now" checking for a set lock bit and returning None. Relies on us knowing that Rw
        // transactions' commit is the only thing that calls fetch_and_tick, and both Read,
        // and Rw transactions call now, before starting.
        let result = self.0.fetch_add(1, Release);

        // Technically, this can wrap to 0 making this UB.
        // - EpochClock is at `end_of_time` - 0x7FFF_FFFF_FFFF_FFFF
        // - Now 2^63 + 1 (0x1000_0000_0000_0001) or more threads start read write transactions.
        // - ...
        // - They all succeed and commit, causing epoch clock to overflow.
        //
        // Memory would run out well before that many threads could be created.
        unsafe { QuiesceEpoch::new_unchecked(result) }
    }
}
