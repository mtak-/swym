//! swym uses 3 different types of epochs to perform synchronization: EpochClock,
//! ThreadEpoch, and EpochLock. The common language between these three types, is the unsychronized
//! QuiesceEpoch. Below is a rough explanation of how these three synchronization types interact.
//!
//! - The EpochClock is a singleton, which holds the current "time".
//! - Each transactional memory location contains an EpochLock which holds the "time" at which the
//!   memory location was last written if unlocked. If locked, the location is currently being
//!   modified.
//! - Each thread holds a ThreadEpoch which contains the "time" that the thread is currently reading
//!   from or a sentinel "INACTIVE_EPOCH", if the thread isn't doing anything transactional.
//!
//! These three types interact as follows.
//! - An inactive thread reads the current "time" from the EpochClock and stores it in its
//!   ThreadEpoch - pinning the thread.
//! - It then proceeds to read from transactional memory locations and check the EpochLock's "time"
//!   to ensure that the value is not locked, and was written before the thread was pinned.
//! - In order to write to one or more locations
//!     - the thread acquires the locations EpochLock(s)
//!     - writes the new values
//!     - bumps current time on the EpochClock
//!     - the thread then atomically unlocks the locations EpochLocks, and sets their new modified
//!       time to the bumped EpochClock time.
//!
//! ThreadEpoch also plays a role in garbage collection. That is more thoroughly explained in
//! `internal/gc.rs`

use core::{
    fmt::{self, Debug, Formatter},
    mem,
    num::NonZeroUsize,
    sync::atomic::Ordering::{self, Acquire, Relaxed, Release},
};
use swym_htm::{HardwareTx, HtmUsize};

type Storage = usize;
type NonZeroStorage = NonZeroUsize;
type HtmStorage = HtmUsize;

/// These values are carefully chosen to allow for simpler comparisons.

/// ThreadEpoch will hold this value when not pinned. It is conveniently greater than all
/// other epochs.
const INACTIVE_EPOCH: Storage = !0;

/// The most significant bit is used to represent whether an EpochLock is locked or not. A value of
/// 1 at that bit indicates the lock is held.
const LOCK_BIT: Storage = 1 << (mem::size_of::<Storage>() as Storage * 8 - 1);

/// The beginning of time.
const FIRST: Storage = TICK_SIZE + UNPARK_BIT;

/// The smallest difference between points on the EpochClock.
///
/// Two is used because the first bit of EpochLock is reserved as the UNPARK_BIT
pub const TICK_SIZE: Storage = 1 << 1;

/// The least significant bit is set when _no_ threads are parked waiting for modifications to an
/// EpochLock.
const UNPARK_BIT: Storage = 1 << 0;

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

#[inline]
const fn unpark_bit_set(e: Storage) -> bool {
    e & UNPARK_BIT != 0
}

#[inline]
const fn clear_unpark_bit(e: Storage) -> Storage {
    e & !UNPARK_BIT
}

/// NonZero representation of epochs. This is assumed to never have the lock bit set unless it
/// contains the INACTIVE_EPOCH.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QuiesceEpoch(NonZeroStorage);

impl QuiesceEpoch {
    /// Creates a new QuiesceEpoch.
    ///
    /// The epoch passed in should not have it's lock bit set, unless it is the inactive epoch. It
    /// is UB to pass in a value of 0.
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
        assume!(epoch != 0, "QuiesceEpoch with value of 0");
        QuiesceEpoch(NonZeroStorage::new_unchecked(epoch))
    }

    /// Creates a new QuiesceEpoch. Returns None if epoch is 0, else it returns Some.
    ///
    /// The epoch passed in should not have it's lock bit set, unless it is the inactive epoch.
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
    pub fn get(self) -> NonZeroStorage {
        self.0
    }

    /// Returns the maximum value that a QuiesceEpoch can hold. This is useful for finding the
    /// minimum of a set of epochs.
    #[inline]
    pub fn max_value() -> Self {
        QuiesceEpoch::new(INACTIVE_EPOCH).unwrap()
    }

    /// The last epoch that is still a valid "time" (e.g. not locked).
    #[inline]
    fn end_of_time() -> Self {
        let r = QuiesceEpoch::new(!LOCK_BIT).unwrap();
        debug_assert!(
            r.is_active(),
            "`QuiesceEpoch::end_of_time` returned an invalid epoch"
        );
        r
    }

    /// Returns true if "self" epoch can read from epochs of the "target" epoch.
    ///
    /// Essentially, does the target epoch come before self.
    #[inline]
    fn read_write_valid_(self, target: Storage) -> bool {
        self.0.get() >= target
    }

    /// Returns true if "self" epoch can read from epochs of the "target" epoch.
    ///
    /// Essentially, does the target epoch come before self.
    #[inline]
    fn read_write_valid(self, target: NonZeroStorage) -> bool {
        self.read_write_valid_(target.get())
    }

    /// Returns true if "self" epoch can read from epochs of the "target" EpochLock - see above.
    ///
    /// If the target EpochLock is locked, this returns false.
    #[inline]
    pub fn read_write_valid_lockable(self, target: &EpochLock) -> bool {
        self.read_write_valid(target.load_raw(Relaxed))
    }

    /// Returns true if self is not the INACTIVE_EPOCH.
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
        QuiesceEpoch::new_unchecked(self.0.get() + TICK_SIZE)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParkStatus {
    NoParked,
    HasParked,
}

impl ParkStatus {
    #[inline]
    pub fn merge(self, rhs: ParkStatus) -> ParkStatus {
        match (self, rhs) {
            (ParkStatus::HasParked, _) => ParkStatus::HasParked,
            (_, ParkStatus::HasParked) => ParkStatus::HasParked,
            _ => ParkStatus::NoParked,
        }
    }
}

/// An atomic QuiesceEpoch, where the MSB is used as a lock. It should never contain the inactive
/// epoch.
///
/// This is used to protect transactional memory locations. All transactional memory locations start
/// out in the FIRST epoch.
pub struct EpochLock(HtmStorage);

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
    /// Creates a new EpochLock initialized to the FIRST epoch.
    #[inline]
    pub const fn first() -> Self {
        EpochLock(HtmStorage::new(FIRST))
    }

    #[inline]
    fn load_raw(&self, o: Ordering) -> NonZeroStorage {
        let value = self.0.load(o);
        debug_assert!(value != 0, "`EpochLock` unexpectedly had 0 as the `Epoch`");
        // The only way to set the value in the EpochLock is with a QuiesceEpoch, which is always
        // NonZero, therefore, loads will always be NonZero.
        unsafe { NonZeroStorage::new_unchecked(value) }
    }

    /// Returns true if the lock is currently held.
    #[inline]
    pub fn is_locked(&self, o: Ordering) -> bool {
        lock_bit_set(self.0.load(o))
    }

    /// Attempts to lock the EpochLock, returning true on success, or false if the lock is already
    /// held, or if the lock contains an epoch greater than the max_expected epoch.
    ///
    /// This is allowed to fail spuriously.
    #[inline]
    #[must_use]
    pub fn try_lock(&self, max_expected: QuiesceEpoch) -> Option<ParkStatus> {
        debug_assert!(
            max_expected.is_active(),
            "invalid max_expected epoch sent to `EpochLock::try_lock`"
        );
        let actual = self.load_raw(Relaxed); // could be a torn read, if MM permitted it
        let success = likely!(max_expected.read_write_valid(actual))
            && likely!({
                debug_assert!(
                    !lock_bit_set(actual.get()),
                    "lock bit unexpectedly set on `EpochLock`"
                );
                self.0
                    .compare_exchange(
                        actual.get(),
                        toggle_lock_bit(actual.get()),
                        Relaxed,
                        Relaxed,
                    )
                    .is_ok()
            });
        if success {
            Some(if unpark_bit_set(actual.get()) {
                ParkStatus::NoParked
            } else {
                ParkStatus::HasParked
            })
        } else {
            None
        }
    }

    /// Attempts to acquire the lock, aborting the transaction on failure.
    #[inline]
    pub fn try_lock_htm(&self, htx: &HardwareTx, max_expected: QuiesceEpoch) -> ParkStatus {
        let actual = self.0.get(htx);
        if likely!(max_expected.read_write_valid_(actual)) {
            self.0.set(htx, toggle_lock_bit(actual));
            if unpark_bit_set(actual) {
                ParkStatus::NoParked
            } else {
                ParkStatus::HasParked
            }
        } else {
            htx.abort()
        }
    }

    /// Unlocks the EpochLock as the specified epoch (basically self.set(epoch)). It is required
    /// that the calling thread hold the lock, and that the epoch passed in does not have it's lock
    /// bit set, and is not inactive.
    #[inline]
    pub unsafe fn unlock_publish(&self, epoch: QuiesceEpoch) {
        debug_assert!(
            self.is_locked(Relaxed),
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
        self.0.store(epoch.0.get(), Release);
    }

    /// Unlocks the EpochLock. It is required that the calling thread hold the lock.
    ///
    /// This does not modify any bit of the EpochLock except the lock bit.
    #[inline]
    pub unsafe fn unlock_undo(&self) {
        let prev = self.load_raw(Relaxed);
        assume!(
            lock_bit_set(prev.get()),
            "lock bit unexpectedly not set on `EpochLock`"
        );
        self.0.store(toggle_lock_bit(prev.get()), Relaxed);
    }

    /// Clears the unpark bit. May abort the transaction
    #[inline]
    pub fn clear_unpark_bit_htm(&self, max_expected: QuiesceEpoch, htx: &HardwareTx) {
        let actual = self.load_raw(Relaxed);
        if likely!(max_expected.read_write_valid(actual)) {
            if unpark_bit_set(actual.get()) {
                self.0.set(htx, clear_unpark_bit(actual.get()))
            }
        } else {
            htx.abort()
        }
    }

    /// Attempts to clear the unpark bit.
    ///
    /// # Warning
    ///
    /// Never call this without holding the parking_lot queue lock.
    #[inline]
    pub fn try_clear_unpark_bit(&self, max_expected: QuiesceEpoch) -> Option<ParkStatus> {
        debug_assert!(
            max_expected.is_active(),
            "invalid max_expected epoch sent to `EpochLock::try_lock`"
        );
        let actual = self.load_raw(Relaxed);
        if likely!(max_expected.read_write_valid(actual)) {
            debug_assert!(
                !lock_bit_set(actual.get()),
                "lock bit unexpectedly set on `EpochLock`"
            );
            if !unpark_bit_set(actual.get()) {
                // if already set, say so
                Some(ParkStatus::HasParked)
            } else if likely!(self
                .0
                .compare_exchange(
                    actual.get(),
                    clear_unpark_bit(actual.get()),
                    Relaxed,
                    Relaxed,
                )
                .is_ok())
            {
                Some(ParkStatus::NoParked)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Set the unpark bit.
    ///
    /// # Warning
    ///
    /// Never call this without holding the parking_lot queue lock. Deadlocks can result.
    #[inline]
    pub fn set_unpark_bit(&self) {
        drop(self.0.fetch_or(UNPARK_BIT, Relaxed));
    }
}

/// This holds the most recent epoch that a thread may currently be accessing, or INACTIVE_EPOCH if
/// the thread is not currently accessing any transactional memory locations.
///
/// Thread's are able to read from any EpochLock whose value is <= it's ThreadEpoch.
#[derive(Debug)]
pub struct ThreadEpoch(HtmStorage);

impl ThreadEpoch {
    #[inline]
    pub fn inactive() -> Self {
        ThreadEpoch(HtmStorage::new(INACTIVE_EPOCH))
    }

    /// Returns true if this thread might be accessing values in quiesce_epoch.
    #[inline]
    pub fn is_quiesced(&self, quiesce_epoch: QuiesceEpoch, o: Ordering) -> bool {
        self.get(o) > quiesce_epoch
    }

    /// Gets the pinned epoch or returns the inactive epoch.
    #[inline]
    pub fn get(&self, o: Ordering) -> QuiesceEpoch {
        // The only way to set the contained value is with a QuiesceEpoch, which is NonZero,
        // therefore, loads will always be NonZero.
        unsafe { QuiesceEpoch::new_unchecked(self.0.load(o)) }
    }

    /// Sets the contained epoch.
    #[inline]
    fn set(&self, value: QuiesceEpoch, o: Ordering) {
        self.0.store(value.0.get(), o)
    }

    /// Yet another sentinel value used to mark a thread that is currently collecting garbage. This
    /// epoch is considered "active" to prevent garbage collection from starting transactions, and
    /// punt on all things reentrancy.
    #[inline]
    pub fn set_collect_epoch(&self) {
        self.set(QuiesceEpoch::end_of_time(), Release)
    }

    /// Pins the ThreadEpoch to epoch.
    ///
    /// Requires that self is not currently pinned, epoch does not have it's lock bit set, and is
    /// not the INACTIVE_EPOCH.
    #[inline]
    pub fn pin(&self, epoch: QuiesceEpoch, o: Ordering) {
        debug_assert!(!self.get(Relaxed).is_active(), "already active ThreadEpoch");
        debug_assert!(
            epoch.is_active(),
            "cannot activate an ThreadEpoch to the inactive state"
        );
        debug_assert!(
            !lock_bit_set(epoch.0.get()),
            "cannot activate an ThreadEpoch to a locked state"
        );
        self.set(epoch, o);
    }

    /// Repins the ThreadEpoch to a later epoch.
    ///
    /// Requires that self is currently pinned, epoch does not have it's lock bit set, is
    /// not the INACTIVE_EPOCH, and is not an epoch earlier than the currently pinned epoch.
    #[inline]
    pub fn repin(&self, epoch: QuiesceEpoch, o: Ordering) {
        debug_assert!(
            self.get(Relaxed).is_active(),
            "attempt to repin an inactive ThreadEpoch"
        );
        debug_assert!(
            epoch >= self.get(Relaxed),
            "attempt to repin ThreadEpoch to an earlier epoch than the currently pinned epoch."
        );
        debug_assert!(
            epoch.is_active(),
            "cannot activate an ThreadEpoch to the inactive state"
        );
        debug_assert!(
            !lock_bit_set(epoch.0.get()),
            "cannot activate an v to a locked state"
        );
        self.set(epoch, o);
    }

    /// Unpins the ThreadEpoch, putting it into the INACTIVE_EPOCH.
    #[inline]
    pub fn unpin(&self, o: Ordering) {
        debug_assert!(
            self.get(Relaxed).is_active(),
            "attempt to deactive an already inactive ThreadEpochs"
        );
        self.set(QuiesceEpoch::new(INACTIVE_EPOCH).unwrap(), o)
    }
}

/// A monotonically increasing clock.
#[derive(Debug)]
#[repr(align(64))]
pub struct EpochClock(HtmStorage);

/// The world clock. The source of truth, and synchronization for swym. Every write transaction
/// bumps this during a successful commit.
pub static EPOCH_CLOCK: EpochClock = EpochClock::new(); // TODO: stick this in GlobalSynchList?

impl EpochClock {
    #[inline]
    const fn new() -> EpochClock {
        EpochClock(HtmStorage::new(FIRST))
    }

    /// Returns the current epoch.
    #[inline]
    pub fn now(&self) -> Option<QuiesceEpoch> {
        let epoch = self.0.load(Acquire);
        if cfg!(target_pointer_width = "64") || likely!(!lock_bit_set(epoch)) {
            // See fetch and tick for justification.
            unsafe {
                assume!(
                    !lock_bit_set(epoch),
                    "EpochClock overflowed into the lock bit"
                );
                Some(QuiesceEpoch::new_unchecked(epoch))
            }
        } else {
            // Program would probly have to be running for several centuries, see below.
            None
        }
    }

    /// Increments the clock, and returns the previous epoch
    #[inline]
    pub fn fetch_and_tick(&self) -> QuiesceEpoch {
        // To calculate how long overflow will take:
        // - MSB is reserved for the lock bit
        // - LSB is reserved for the unpark bit
        // - so on 64 bit platforms we have 2^62 * fetch_add_time(ns) / 1000000000(ns/s) /
        //   60(sec/min) / 60(min/hr) / 24(hr/day) / 365(day/yr)
        //  for a fetch_add time of 1ns (actually benched at around 5ns), we get 146.23yrs
        //  on 32 bit platforms we get just over 1sec until overflow (fetch_add=1ns)
        //
        // When this overflow happens, the lock bit becomes set for additional epochs causing
        // reads from EpochLocks to succeed even when locked by another thread. This is solved by
        // "now" checking for a set lock bit and returning None. Relies on us knowing that Rw
        // transactions' commit is the only thing that calls fetch_and_tick, and both Read,
        // and Rw transactions call now, before starting.
        //
        // NOTE: actually on 64 bit platforms we just assume overflowing into the lock bit is
        // impossible.
        let result = self.0.fetch_add(TICK_SIZE, Release);

        // Technically, this can wrap to 0 making this UB.
        // - EpochClock is at `end_of_time` - 0x7FFF_FFFF_FFFF_FFFF
        // - Now 2^63 + 2^62 + 1 or more threads start read write transactions.
        // - ...
        // - They all succeed and commit, causing epoch clock to overflow.
        //
        // Memory would run out well before that many threads could be created. This is true of 32
        // bit platforms as well.
        unsafe { QuiesceEpoch::new_unchecked(result) }
    }
}
