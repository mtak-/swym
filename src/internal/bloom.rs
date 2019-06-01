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

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Contained {
    No,
    Maybe,
}

#[derive(Copy, Clone, Debug)]
enum Filter {
    Inline(usize),
    Overflow,
}

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
            filter:   Cell::new(Filter::Inline(0)),
            overflow: Default::default(),
            phantom:  PhantomData,
        }
    }

    fn overflow(&self) -> &mut FxHashMap<*const K, usize> {
        unsafe { &mut *self.overflow.get() }
    }

    #[inline]
    fn has_overflowed(&self) -> bool {
        match self.filter.get() {
            Filter::Overflow => true,
            Filter::Inline(_) => false,
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        match *self.filter.get_mut() {
            Filter::Overflow => self.overflow().clear(),
            Filter::Inline(_) => {}
        }
        *self.filter.get_mut() = Filter::Inline(0);
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
        match self.filter.get() {
            Filter::Inline(filter) => filter == 0,
            Filter::Overflow => self.overflow().is_empty(),
        }
    }

    #[inline]
    pub fn to_overflow(&self, offsets: impl Iterator<Item = (&'tcell K, usize)>) {
        match self.filter.get() {
            Filter::Overflow => {}
            Filter::Inline(_) => self.run_overflow(offsets),
        }
    }

    #[inline(never)]
    #[cold]
    fn run_overflow(&self, offsets: impl Iterator<Item = (&'tcell K, usize)>) {
        self.filter.set(Filter::Overflow);
        let overflow = self.overflow();
        overflow.extend(offsets.map(|(k, v)| (k as *const K, v)));
    }

    #[inline]
    pub fn contained(&self, key: &K) -> Contained {
        match self.filter.get() {
            Filter::Inline(filter) => {
                let bit = bloom_bit(key);

                if unlikely!(filter & bit.0.get() != 0) {
                    Contained::Maybe
                } else {
                    Contained::No
                }
            }
            Filter::Overflow => Contained::Maybe,
        }
    }

    #[inline]
    pub fn insert_inline(&self, key: &'tcell K) -> Contained {
        match self.filter.get() {
            Filter::Inline(filter) => {
                let bit = bloom_bit(key);

                if unlikely!(filter & bit.0.get() != 0) {
                    Contained::Maybe
                } else {
                    self.filter.set(Filter::Inline(filter | bit.0.get()));
                    Contained::No
                }
            }
            Filter::Overflow => Contained::Maybe,
        }
    }

    #[inline(never)]
    #[cold]
    pub fn insert_overflow(&self, key: &'tcell K, index: usize) -> bool {
        debug_assert!(self.has_overflowed());
        self.overflow().insert(key, index).is_some()
    }

    #[inline]
    pub fn overflow_get(&self, key: &K) -> Option<usize> {
        debug_assert!(self.has_overflowed());
        self.overflow().get(&(key as _)).cloned()
    }
}

#[inline]
const fn calc_shift<T>() -> usize {
    (mem::align_of::<T>() > 1) as usize
        + (mem::align_of::<T>() > 2) as usize
        + (mem::align_of::<T>() > 4) as usize
        + (mem::align_of::<T>() > 8) as usize
        + 1 // In practice this +1 results in less failures, however it's not "correct". Any TCell with a
            // meaningful value happens to have a minimum size of mem::size_of::<usize>() * 2 which might
            // explain why the +1 is helpful for certain workloads.
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
