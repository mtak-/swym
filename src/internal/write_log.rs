use crate::{
    internal::{
        alloc::dyn_vec::{self, DynElemMut, TraitObject},
        bloom::{Bloom, Contained},
        epoch::{EpochLock, QuiesceEpoch},
        tcell_erased::TCellErased,
        usize_aligned::ForcedUsizeAligned,
    },
    stats,
};
use core::{
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    ptr::{self, NonNull},
};
use std::collections::hash_map::{Entry as HashMapEntry, OccupiedEntry as HashMapOccupiedEntry};

#[repr(C)]
pub struct WriteEntryImpl<'tcell, T> {
    dest:    Option<&'tcell TCellErased>,
    pending: ForcedUsizeAligned<T>,
}

impl<'tcell, T> WriteEntryImpl<'tcell, T> {
    #[inline]
    pub const fn new(dest: &'tcell TCellErased, pending: T) -> Self {
        WriteEntryImpl {
            dest:    Some(dest),
            pending: ForcedUsizeAligned::new(pending),
        }
    }
}

pub unsafe trait WriteEntry {}
unsafe impl<'tcell, T> WriteEntry for WriteEntryImpl<'tcell, T> {}

impl<'tcell> dyn WriteEntry + 'tcell {
    fn data_ptr(&self) -> NonNull<usize> {
        debug_assert!(
            mem::align_of_val(self) >= mem::align_of::<NonNull<usize>>(),
            "incorrect alignment on data_ptr"
        );
        // obtains a thin pointer to self
        unsafe {
            let raw: TraitObject = mem::transmute::<&Self, _>(self);
            NonNull::new_unchecked(raw.data as *mut _)
        }
    }

    #[inline]
    pub fn tcell(&self) -> &'_ Option<&'_ TCellErased> {
        let this = self.data_ptr();
        unsafe { &*(this.as_ptr() as *mut _ as *const _) }
    }

    #[inline]
    fn tcell_mut(&mut self) -> &'_ mut Option<&'tcell TCellErased> {
        let this = self.data_ptr();
        unsafe { &mut *(this.as_ptr() as *mut _) }
    }

    #[inline]
    pub fn pending(&self) -> NonNull<usize> {
        unsafe { NonNull::new_unchecked(self.data_ptr().as_ptr().add(1)) }
    }

    #[inline]
    pub unsafe fn read<T>(&self) -> ManuallyDrop<T> {
        debug_assert!(
            mem::size_of_val(self) == mem::size_of::<WriteEntryImpl<'tcell, T>>(),
            "destination size error during `WriteEntry::read`"
        );
        assert!(
            mem::size_of::<T>() > 0,
            "`WriteEntry` performing a read of size 0 unexpectedly"
        );
        let downcast = &(&*(self as *const _ as *const WriteEntryImpl<'tcell, T>)).pending
            as *const ForcedUsizeAligned<T>;
        if mem::align_of::<T>() > mem::align_of::<usize>() {
            ptr::read_unaligned::<ManuallyDrop<T>>(downcast as _)
        } else {
            ptr::read::<ManuallyDrop<T>>(downcast as _)
        }
    }
}

dyn_vec_decl! {struct DynVecWriteEntry: WriteEntry;}

/// TODO: WriteLog is very very slow if the bloom filter fails.
/// probably worth looking into some true hashmaps
#[repr(C)]
pub struct WriteLog<'tcell> {
    bloom: Bloom<'tcell, TCellErased>,
    data:  DynVecWriteEntry<'tcell>,
}

impl<'tcell> WriteLog<'tcell> {
    #[inline]
    pub fn new() -> Self {
        WriteLog {
            bloom: Bloom::new(),
            data:  DynVecWriteEntry::new(),
        }
    }

    #[inline]
    pub fn contained(&self, tcell: &'tcell TCellErased) -> Contained {
        stats::bloom_check();
        self.bloom.contained(tcell)
    }

    #[inline]
    pub fn contained_set(&self, tcell: &'tcell TCellErased) -> Contained {
        stats::bloom_check();
        self.bloom.insert_inline(tcell)
    }

    #[inline]
    pub fn word_len(&self) -> usize {
        self.data.word_len()
    }

    #[inline]
    pub fn clear(&mut self) {
        self.bloom.clear();
        // TODO: NESTING: tx's can start here
        stats::write_word_size(self.word_len());
        self.data.clear();
    }

    #[inline]
    pub fn clear_no_drop(&mut self) {
        self.bloom.clear();
        stats::write_word_size(self.word_len());
        self.data.clear_no_drop();
    }

    #[inline]
    pub unsafe fn drop_writes(&mut self) {
        for mut elem in self.data.iter_mut() {
            ptr::drop_in_place::<dyn WriteEntry>(&mut *elem)
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        let empty = self.data.is_empty();
        debug_assert_eq!(
            empty,
            self.bloom.is_empty(),
            "bloom filter and container out of sync"
        );
        empty
    }

    #[inline]
    pub fn epoch_locks<'a>(
        &'a self,
    ) -> std::iter::FlatMap<
        crate::internal::alloc::dyn_vec::Iter<'a, (dyn WriteEntry + 'tcell)>,
        Option<&'a EpochLock>,
        impl FnMut(&'a (dyn WriteEntry + 'tcell)) -> Option<&'a EpochLock>,
    > {
        self.data
            .iter()
            .flat_map(|entry| entry.tcell().map(|erased| &erased.current_epoch))
    }

    #[inline]
    pub fn write_entries<'a>(
        &'a self,
    ) -> crate::internal::alloc::dyn_vec::Iter<'a, (dyn WriteEntry + 'tcell)> {
        self.data.iter()
    }

    #[inline]
    fn overflow(&self) {
        unsafe {
            self.bloom
                .to_overflow(self.write_entries().flat_map(|elem| {
                    let raw = TraitObject::from_pointer(self.data.word_index_unchecked(0).into())
                        .data as usize;
                    let raw2 = TraitObject::from_pointer(elem.into()).data as usize;
                    elem.tcell()
                        .map(move |tcell| (tcell, (raw2 - raw) / mem::size_of::<usize>()))
                }));
        }
    }

    #[inline]
    pub fn find_skip_filter(&self, dest_tcell: &TCellErased) -> Option<&dyn WriteEntry> {
        self.overflow();
        let result = self.bloom.overflow_get(dest_tcell).map(|index| {
            debug_assert!(
                index < self.data.word_len(),
                "attempting to index at word {} of a {} word dynvec",
                index,
                self.data.word_len()
            );
            unsafe { self.data.word_index_unchecked(index) }
        });
        if result.is_some() {
            stats::bloom_success_slow()
        } else {
            stats::bloom_collision()
        }
        result
    }

    #[inline(never)]
    fn find_slow(&self, dest_tcell: &TCellErased) -> Option<&dyn WriteEntry> {
        self.find_skip_filter(dest_tcell)
    }

    // biased against finding the tcell
    #[inline]
    pub fn find(&self, dest_tcell: &TCellErased) -> Option<&dyn WriteEntry> {
        if likely!(self.bloom.contained(dest_tcell) == Contained::No) {
            None
        } else {
            self.find_slow(dest_tcell)
        }
    }

    #[inline]
    pub fn entry<'a>(&'a mut self, dest_tcell: &TCellErased) -> Entry<'a, 'tcell> {
        self.overflow();

        match self.bloom.overflow_entry(dest_tcell) {
            HashMapEntry::Occupied(o) => {
                stats::bloom_success_slow();
                stats::write_after_write();
                debug_assert!(*o.get() < self.data.word_len());
                Entry::new_occupied(o, &mut self.data)
            }
            HashMapEntry::Vacant(v) => {
                drop(v.insert(self.data.word_len()));
                stats::bloom_collision();
                Entry::Vacant
            }
        }
    }

    #[inline]
    pub fn next_push_allocates<T>(&self) -> bool {
        self.data.next_push_allocates::<WriteEntryImpl<'tcell, T>>()
    }

    #[inline]
    pub unsafe fn record_unchecked<T: 'static>(&mut self, dest_tcell: &'tcell TCellErased, val: T) {
        debug_assert!(
            self.epoch_locks()
                .find(|&x| ptr::eq(x, &dest_tcell.current_epoch))
                .is_none(),
            "attempt to add `TCell` to the `WriteLog` twice"
        );
        debug_assert!(self.bloom.contained(dest_tcell) == Contained::Maybe);

        self.data
            .push_unchecked(WriteEntryImpl::new(dest_tcell, val));
    }

    #[inline]
    pub fn record<T: 'static>(&mut self, dest_tcell: &'tcell TCellErased, val: T) {
        self.data.push(WriteEntryImpl::new(dest_tcell, val));
    }

    #[inline]
    pub fn validate_writes(&self, pin_epoch: QuiesceEpoch) -> bool {
        for epoch_lock in self.epoch_locks() {
            if !pin_epoch.read_write_valid_lockable(epoch_lock) {
                return false;
            }
        }
        true
    }
}

pub enum Entry<'a, 'tcell> {
    Vacant,
    Occupied(OccupiedEntry<'a, 'tcell>),
}

impl<'a, 'tcell> Entry<'a, 'tcell> {
    #[inline]
    fn new_occupied(
        entry: HashMapOccupiedEntry<'a, *const TCellErased, usize>,
        data: &'a mut DynVecWriteEntry<'tcell>,
    ) -> Self {
        Entry::Occupied(OccupiedEntry {
            entry,
            data,
            phantom: PhantomData,
        })
    }
}

pub struct OccupiedEntry<'a, 'tcell> {
    entry:   HashMapOccupiedEntry<'a, *const TCellErased, usize>,
    data:    &'a mut DynVecWriteEntry<'tcell>,
    phantom: PhantomData<Vec<&'tcell TCellErased>>,
}

impl<'a, 'tcell> OccupiedEntry<'a, 'tcell> {
    pub fn overwrite<T: 'static>(self, dest_tcell: &'tcell TCellErased, val: T) {
        let new_entry = WriteEntryImpl::new(dest_tcell, val);
        let new_vtable = dyn_vec::vtable::<dyn WriteEntry + 'tcell>(&new_entry);
        unsafe {
            let dyn_elem = self.data.word_index_unchecked_mut(*self.entry.get());
            DynElemMut::assign_unchecked(dyn_elem, new_vtable, new_entry)
        }
    }

    pub fn tombstone_replace<T: 'static>(mut self, dest_tcell: &'tcell TCellErased, val: T) {
        let prev = self.entry.insert(self.data.word_len());
        let mut entry = unsafe { self.data.word_index_unchecked_mut(prev) };
        debug_assert!(
            entry.tcell().is_some(),
            "unexpectedly tombstoning an already tombstoned write log entry"
        );
        *entry.tcell_mut() = None;
        self.data.push(WriteEntryImpl::new(dest_tcell, val));
    }
}
