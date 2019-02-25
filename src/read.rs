use crate::{
    internal::epoch::QuiesceEpoch,
    tcell::TCell,
    tx::{Error, Ordering, Read},
};
use std::{
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    sync::atomic::Ordering::Acquire,
};

/// A read only transaction.
pub struct ReadTx<'tcell>(PhantomData<fn(&'tcell ())>);
impl<'tcell> !Send for ReadTx<'tcell> {}
impl<'tcell> !Sync for ReadTx<'tcell> {}

impl<'tcell> ReadTx<'tcell> {
    #[inline]
    pub(crate) fn new<'a>(pin_epoch: QuiesceEpoch) -> &'a Self {
        unsafe { mem::transmute(pin_epoch) }
    }

    #[inline]
    fn pin_epoch(&self) -> QuiesceEpoch {
        unsafe { mem::transmute(self) }
    }

    #[inline]
    unsafe fn get_impl<T>(&self, tcell: &TCell<T>) -> Result<ManuallyDrop<T>, Error> {
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
        tcell: &TCell<T>,
        ordering: Ordering,
    ) -> Result<ManuallyDrop<T>, Error> {
        match ordering {
            Ordering::ReadWrite | Ordering::Read => self.get_impl(tcell),
        }
    }
}
