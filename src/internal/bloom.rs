//! A simple 64bit bloom filter that falls back to an actual HashMap.
//!
//!
//!
//! Potentially relevant paper: http://www.eecg.toronto.edu/~steffan/papers/jeffrey_spaa11.pdf

use core::{
    cell::{Cell, UnsafeCell},
    marker::PhantomData,
    mem,
    num::NonZeroUsize,
};
use fxhash::FxHashMap;
use std::collections::hash_map::Entry;

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Contained {
    No,
    Maybe,
}

type Filter = usize;

const OVERFLOWED: Filter = !0;

#[derive(Debug)]
pub struct Bloom<'tcell, K> {
    filter:   Cell<Filter>,
    overflow: UnsafeCell<FxHashMap<*const K, usize>>,
    phantom:  PhantomData<&'tcell K>,
}

impl<'tcell, K> Bloom<'tcell, K> {
    #[inline]
    pub fn new() -> Self {
        Bloom {
            filter:   Cell::new(0),
            overflow: Default::default(),
            phantom:  PhantomData,
        }
    }

    fn overflow(&self) -> &mut FxHashMap<*const K, usize> {
        unsafe { &mut *self.overflow.get() }
    }

    #[inline]
    fn has_overflowed(&self) -> bool {
        self.filter.get() == OVERFLOWED
    }

    #[inline]
    pub fn clear(&mut self) {
        let filter = *self.filter.get_mut();
        if filter == OVERFLOWED {
            self.overflow().clear()
        }
        *self.filter.get_mut() = 0;
        debug_assert!(
            self.overflow().is_empty(),
            "`clear` failed to empty the container"
        );
        debug_assert!(self.is_empty(), "`clear` failed to empty the container");
        debug_assert!(
            !self.has_overflowed(),
            "`clear` failed to reset to `Inline` storage"
        );
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.filter.get() == 0
    }

    #[inline]
    pub fn to_overflow(&self, offsets: impl Iterator<Item = (&'tcell K, usize)>) {
        if self.filter.get() != OVERFLOWED {
            self.run_overflow(offsets)
        }
    }

    #[inline(never)]
    #[cold]
    fn run_overflow(&self, offsets: impl Iterator<Item = (&'tcell K, usize)>) {
        self.filter.set(OVERFLOWED);
        let overflow = self.overflow();
        overflow.extend(offsets.map(|(k, v)| (k as *const K, v)));
    }

    #[inline]
    pub fn contained(&self, key: &K) -> Contained {
        let bit = bloom_bit(key);

        if unlikely!(self.filter.get() & bit.0.get() != 0) {
            Contained::Maybe
        } else {
            Contained::No
        }
    }

    // If this returns Maybe, then there's no guarantee the value was inserted. At that time,
    // overflowing is required.
    #[inline]
    pub fn insert_inline(&self, key: &'tcell K) -> Contained {
        let filter = self.filter.get();
        let bit = bloom_bit(key);

        if unlikely!(filter & bit.0.get() != 0) {
            Contained::Maybe
        } else {
            let new_filter = filter | bit.0.get();
            if new_filter != OVERFLOWED {
                self.filter.set(new_filter);
                Contained::No
            } else {
                Contained::Maybe
            }
        }
    }

    #[inline]
    pub fn overflow_get(&self, key: &K) -> Option<usize> {
        debug_assert!(self.has_overflowed());
        self.overflow().get(&(key as _)).cloned()
    }

    #[inline]
    pub fn overflow_entry(&mut self, key: &K) -> Entry<'_, *const K, usize> {
        debug_assert!(self.has_overflowed());
        self.overflow().entry(key as _)
    }
}

#[inline]
const fn calc_shift<T>() -> usize {
    (mem::align_of::<T>() > 1) as usize
        + (mem::align_of::<T>() > 2) as usize
        + (mem::align_of::<T>() > 4) as usize
        + (mem::align_of::<T>() > 8) as usize
        + 1 // In practice this +1 results in less failures, however it's not "correct". Any TCell
            // with a meaningful value happens to have a minimum size of
            // mem::size_of::<usize>() * 2 which might explain why the +1 is helpful for
            // certain workloads.
}

#[inline]
fn bloom_bit<T>(value: *const T) -> BloomBit {
    let shift = calc_shift::<T>();
    let raw_hash: usize = value as usize >> shift;
    let result = 1 << (raw_hash & (mem::size_of::<NonZeroUsize>() * 8 - 1));
    debug_assert!(result > 0, "bloom_hash should not return 0");
    let hash = unsafe { NonZeroUsize::new_unchecked(result) };
    BloomBit(hash)
}

#[derive(Copy, Clone, PartialEq, Eq)]
struct BloomBit(NonZeroUsize);
