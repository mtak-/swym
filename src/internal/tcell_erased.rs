use crate::internal::epoch::EpochLock;

// A "dynamic" type that can have references to instances of it put into a collection and still have
// meaning. The type the TCell contains is not recoverable, but it's ok to load from, or store to
// it, as long as you know the type (through some other means) or the len respectively.
//
// This relies heavily on repr() and the layout of TCell. In order to handle overaligned types
// (align_of::<T>() > align_of::<usize>()) TCellErased is stored after UsizeAligned<T> in the TCell.
// A nice side benefit is that reads always read T first then the EpochLock, so this layout is
// likely better for the cache.
#[repr(transparent)]
#[derive(Debug)]
pub struct TCellErased {
    pub current_epoch: EpochLock,
}

impl TCellErased {
    #[inline]
    pub const fn new() -> TCellErased {
        TCellErased {
            current_epoch: EpochLock::first(),
        }
    }
}
