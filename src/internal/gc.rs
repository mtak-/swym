//! swym's garbage collection. Very similar to crossbeams garbage collection, but with the concept
//! of speculation, and piggy backing on the epoch scheme required by the Transactional Locking II
//! algorithm.
//!
//! Crossbeam-epoch is not used for the following reasons:
//! - we'd have to have a separate speculative garbage bag which gets published on commit to
//!   crossbeam
//! - we'd still have to have our own epoch system to implement read/write validation. That means
//!   there'd be two cmpxchgs on commit to global counters leading to more ping-ponging.
//! - we'd be likely to wind up performing more allocations per transaction than we currently do
//!   (average of 0). Allocations (page faults/`lock` prefixes) abort hardware transactions giving
//!   us less flexibility to explore hardware transactions in the future.
//!
//! The algorithm works as follows.
//!
//! 0. Pin the current thread to the global epoch.
//! 1. Start speculatively accumulating garbage
//! 2. a. On transaction failure, "forget" all of the garbage.
//! 2. b. On transaction success (assuming any garbage was queued for collection)
//!     - The EpochClock was ticked by swym, and the current bag is sealed with the immediately
//!       preceding epoch.
//!     - The sealed bag is now guaranteed to be collected, and pushed into a list of other sealed
//!       bags.
//!     - If that list of sealed bags is now full (after pushing)
//!         - We look at the epoch of the oldest bag
//!         - Then Freeze the global thread list, and iterate through the list of threads checking
//!           their pinned epochs.
//!         - If a threads pinned epoch is <= the epoch of the oldest bag, we wait for it to change.
//!         - Else if the epoch is lower than any other observed epoch we record it.
//!         - After iterating we now know the oldest pinned epoch.
//!         - Then we collect all bags with epochs < than that epoch (and are guaranteed to atleast
//!           collect the oldest bag).
//! 3. Unpin the current thread (set current_epoch to INACTIVE)

mod queued;
mod quiesce;
mod thread_garbage;

pub use self::{
    quiesce::{FreezeList, GlobalSynchList, OwnedSynch, SynchList, Write},
    thread_garbage::ThreadGarbage,
};
