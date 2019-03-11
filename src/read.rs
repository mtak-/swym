use crate::{
    internal::{epoch::QuiesceEpoch, thread::Pin},
    tcell::TCell,
    tx::{Error, Ordering, Read},
};
use std::{
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    sync::atomic::Ordering::Acquire,
};

/// A read only transaction.
//
// No instances of this type are ever created. References to values of this type are created by
// transmuting QuiesceEpoch's.
pub struct ReadTx<'tcell>(PhantomData<fn(&'tcell ())>);
impl<'tcell> !Send for ReadTx<'tcell> {}
impl<'tcell> !Sync for ReadTx<'tcell> {}

impl<'tcell> ReadTx<'tcell> {
    #[inline]
    pub(crate) fn new<'tx>(pin: &'tx mut Pin<'tcell>) -> &'tx Self {
        assert!(mem::align_of::<Self>() == 1, "unsafe alignment on ReadTx");
        // we smuggle the pinned epoch through as a reference
        unsafe { mem::transmute(pin.pin_epoch()) }
    }

    #[inline]
    fn pin_epoch(&self) -> QuiesceEpoch {
        // convert the reference back into the smuggled pinned epoch
        unsafe { mem::transmute(self) }
    }

    #[inline]
    unsafe fn get_impl<T>(&self, tcell: &'tcell TCell<T>) -> Result<ManuallyDrop<T>, Error> {
        // In a read only transaction, there is no read log, write log or gc.
        // The only thing that needs to be done is reading of the value, and then a check, to see if
        // that value was written before this transaction began.
        let value = tcell.erased.read_acquire::<T>();
        if likely!(self
            .pin_epoch()
            .read_write_valid_lockable(&tcell.erased.current_epoch, Acquire))
        {
            Ok(value)
        } else {
            Err(Error::RETRY)
        }
    }
}

unsafe impl<'tcell> Read<'tcell> for ReadTx<'tcell> {
    #[inline]
    unsafe fn _get_unchecked<T>(
        &self,
        tcell: &'tcell TCell<T>,
        ordering: Ordering,
    ) -> Result<ManuallyDrop<T>, Error> {
        match ordering {
            Ordering::ReadWrite | Ordering::Read => self.get_impl(tcell),
        }
    }
}
