use crate::internal::{
    alloc::{dyn_vec::DynElemMut, DynVec},
    epoch::QuiesceEpoch,
    pointer::PtrExt,
    stats,
    tcell_erased::TCellErased,
    usize_aligned::ForcedUsizeAligned,
};
use std::{
    mem::{self, ManuallyDrop},
    num::NonZeroUsize,
    ptr::{self, NonNull},
    raw::TraitObject,
    sync::atomic::{self, Ordering::Release},
};

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
    fn tcell(&self) -> &'_ Option<&'_ TCellErased> {
        let this = self.data_ptr();
        unsafe { &*(this.as_ptr() as *mut _ as *const _) }
    }

    #[inline]
    fn tcell_mut(&mut self) -> &'_ mut Option<&'tcell TCellErased> {
        let this = self.data_ptr();
        unsafe { &mut *(this.as_ptr() as *mut _) }
    }

    #[inline]
    fn pending(&self) -> NonNull<usize> {
        unsafe { self.data_ptr().add(1).cast() }
    }

    #[inline]
    pub fn deactivate(&mut self) {
        debug_assert!(
            self.tcell().is_some(),
            "unexpectedly deactivating an inactive write log entry"
        );
        *self.tcell_mut() = None
    }

    #[inline]
    pub fn is_dest_tcell(&self, v: &TCellErased) -> bool {
        match self.tcell() {
            Some(tcell) => ptr::eq(*tcell, v),
            None => false,
        }
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
        PtrExt::read_as::<_>(downcast)
    }

    #[inline]
    #[must_use]
    pub fn try_lock(&self, pin_epoch: QuiesceEpoch) -> bool {
        match self.tcell() {
            Some(tcell) => tcell.current_epoch.try_lock(pin_epoch),
            None => true,
        }
    }

    #[inline]
    pub unsafe fn unlock(&self) {
        match self.tcell() {
            Some(tcell) => tcell.current_epoch.unlock_undo(),
            None => {}
        }
    }

    #[inline]
    pub unsafe fn perform_write(&self) {
        match self.tcell() {
            Some(tcell) => {
                let size = mem::size_of_val(self);
                assume!(
                    size % mem::size_of::<usize>() == 0,
                    "buggy alignment on `WriteEntry`"
                );
                let len = size / mem::size_of::<usize>() - 1;
                assume!(
                    len > 0,
                    "`WriteEntry` performing a write of size 0 unexpectedly"
                );
                self.pending()
                    .copy_to_n(NonNull::from(*tcell).cast().sub(len), len);
            }
            None => {}
        }
    }

    #[inline]
    pub unsafe fn publish(&self, publish_epoch: QuiesceEpoch) {
        match self.tcell() {
            Some(tcell) => tcell.current_epoch.unlock_publish(publish_epoch),
            None => {}
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Contained {
    No,
    Maybe,
}

/// TODO: WriteLog is very very slow if the bloom filter fails.
/// probably worth looking into some true hashmaps
#[repr(C)]
pub struct WriteLog<'tcell> {
    filter: usize,
    data:   DynVec<dyn WriteEntry + 'tcell>,
}

impl<'tcell> WriteLog<'tcell> {
    #[inline]
    pub fn new() -> Self {
        WriteLog {
            filter: 0,
            data:   DynVec::new(),
        }
    }

    #[inline]
    pub fn contained(&self, hash: NonZeroUsize) -> Contained {
        stats::bloom_check();
        if unlikely!(self.filter & hash.get() != 0) {
            Contained::Maybe
        } else {
            Contained::No
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.filter = 0;
        // TODO: NESTING: tx's can start here
        stats::write_word_size(self.data.word_len());
        self.data.clear();
    }

    #[inline]
    pub fn clear_no_drop(&mut self) {
        self.filter = 0;
        stats::write_word_size(self.data.word_len());
        self.data.clear_no_drop();
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        let empty = self.filter == 0;
        debug_assert_eq!(
            empty,
            self.data.is_empty(),
            "bloom filter and container out of sync"
        );
        empty
    }

    #[inline(never)]
    fn find_slow(&self, dest_tcell: &TCellErased) -> Option<&dyn WriteEntry> {
        let result = self
            .data
            .iter()
            .find(move |&entry| entry.is_dest_tcell(dest_tcell));
        if result.is_some() {
            stats::bloom_success_slow()
        } else {
            stats::bloom_failure()
        }
        result
    }

    // biased against finding the tcell
    #[inline]
    pub fn find(&self, dest_tcell: &TCellErased) -> Option<&dyn WriteEntry> {
        let hash = bloom_hash(dest_tcell);
        debug_assert!(hash.get() != 0, "bug in bloom_hash algorithm");
        if likely!(self.contained(hash) == Contained::No) {
            None
        } else {
            self.find_slow(dest_tcell)
        }
    }

    #[inline(never)]
    fn entry_slow<'a>(
        &'a mut self,
        dest_tcell: &TCellErased,
        hash: NonZeroUsize,
    ) -> Entry<'a, 'tcell> {
        match self
            .data
            .iter_mut()
            .find(|entry| entry.is_dest_tcell(dest_tcell))
        {
            // TODO: Borrow checker is a little off here. Without the transmute, the code does not
            // compile. But, replacing either branch's return with `unimplemented` compiles.
            // polonius would fix this.
            Some(entry) => {
                stats::bloom_success_slow();
                stats::double_write();
                Entry::new_occupied(unsafe { mem::transmute(entry) }, hash)
            }
            None => {
                stats::bloom_failure();
                Entry::new_hash(self, hash)
            }
        }
    }

    // biased against finding the tcell
    #[inline]
    pub fn entry<'a>(&'a mut self, dest_tcell: &TCellErased) -> Entry<'a, 'tcell> {
        let hash = bloom_hash(dest_tcell);
        debug_assert!(hash.get() != 0, "bug in dumb_reference_hash algorithm");
        if likely!(self.contained(hash) == Contained::No) {
            Entry::new_hash(self, hash)
        } else {
            self.entry_slow(dest_tcell, hash)
        }
    }

    #[inline]
    pub fn next_push_allocates<T>(&self) -> bool {
        self.data.next_push_allocates::<WriteEntryImpl<'tcell, T>>()
    }

    #[inline]
    pub fn push<T: 'static>(
        &mut self,
        dest_tcell: &'tcell TCellErased,
        val: T,
        hash: NonZeroUsize,
    ) {
        debug_assert!(
            self.data
                .iter()
                .find(|x| x.is_dest_tcell(dest_tcell))
                .is_none(),
            "attempt to add `TCell` to the `WriteLog` twice"
        );

        self.filter |= hash.get();
        self.data.push(WriteEntryImpl::new(dest_tcell, val));
    }

    #[inline]
    pub unsafe fn push_unchecked<T: 'static>(
        &mut self,
        dest_tcell: &'tcell TCellErased,
        val: T,
        hash: NonZeroUsize,
    ) {
        debug_assert!(
            self.data
                .iter()
                .find(|x| x.is_dest_tcell(dest_tcell))
                .is_none(),
            "attempt to add `TCell` to the `WriteLog` twice"
        );

        self.filter |= hash.get();
        self.data
            .push_unchecked(WriteEntryImpl::new(dest_tcell, val));
    }

    #[must_use]
    #[inline]
    pub fn try_lock_entries(&self, pin_epoch: QuiesceEpoch) -> bool {
        debug_assert!(!self.is_empty(), "attempt to lock empty write set");

        for entry in &self.data {
            if unlikely!(!entry.try_lock(pin_epoch)) {
                unsafe {
                    self.unlock_entries_until(entry);
                }
                return false;
            }
        }
        true
    }

    #[inline(never)]
    #[cold]
    unsafe fn unlock_entries_until(&self, entry: &dyn WriteEntry) {
        let iter = self.data.iter().take_while(move |&e| !ptr::eq(e, entry));
        for entry in iter {
            entry.unlock();
        }
    }

    #[inline]
    pub unsafe fn unlock_entries(&self) {
        for entry in &self.data {
            entry.unlock();
        }
    }

    #[inline]
    pub unsafe fn perform_writes(&self) {
        atomic::fence(Release);
        for entry in &self.data {
            entry.perform_write();
        }
    }

    #[inline]
    pub unsafe fn publish(&self, publish_epoch: QuiesceEpoch) {
        for entry in &self.data {
            entry.publish(publish_epoch);
        }
    }
}

pub enum Entry<'a, 'tcell> {
    Vacant {
        write_log: &'a mut WriteLog<'tcell>,
        hash:      NonZeroUsize,
    },
    Occupied {
        entry: DynElemMut<'a, dyn WriteEntry + 'tcell>,
        hash:  NonZeroUsize,
    },
}

impl<'a, 'tcell> Entry<'a, 'tcell> {
    #[inline]
    fn new_hash(write_log: &'a mut WriteLog<'tcell>, hash: NonZeroUsize) -> Self {
        Entry::Vacant { write_log, hash }
    }

    #[inline]
    fn new_occupied(entry: DynElemMut<'a, dyn WriteEntry + 'tcell>, hash: NonZeroUsize) -> Self {
        Entry::Occupied { entry, hash }
    }
}

#[inline]
const fn calc_shift() -> usize {
    (mem::align_of::<TCellErased>() > 1) as usize
        + (mem::align_of::<TCellErased>() > 2) as usize
        + (mem::align_of::<TCellErased>() > 4) as usize
        + (mem::align_of::<TCellErased>() > 8) as usize
        + 1 // In practice this +1 results in less failures, however it's not "correct". Any TCell with a
            // meaningful value happens to have a minimum size of mem::size_of::<usize>() + 1 which might
            // explain why the +1 is helpful for certain workloads.
}

#[inline]
pub fn bloom_hash(value: &TCellErased) -> NonZeroUsize {
    const SHIFT: usize = calc_shift();
    let raw_hash: usize = value as *const TCellErased as usize >> SHIFT;
    let result = 1 << (raw_hash & (mem::size_of::<NonZeroUsize>() * 8 - 1));
    debug_assert!(result > 0, "bloom_hash should not return 0");
    unsafe { NonZeroUsize::new_unchecked(result) }
}
