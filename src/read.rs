use crate::{
    internal::{epoch::QuiesceEpoch, thread::Pin},
    tcell::{Ref, TCell},
    tx::{Borrow, Error, Ordering, Read},
};
use std::{marker::PhantomData, mem, sync::atomic::Ordering::Acquire};

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
        let pin_epoch: QuiesceEpoch = pin.pin_epoch();
        unsafe { mem::transmute(pin_epoch) }
    }

    #[inline]
    fn pin_epoch(&self) -> QuiesceEpoch {
        // convert the reference back into the smuggled pinned epoch
        unsafe { mem::transmute(self) }
    }
}

impl<'tcell> Read<'tcell> for ReadTx<'tcell> {
    #[inline]
    fn borrow<'tx, T: Borrow>(
        &'tx self,
        tcell: &'tcell TCell<T>,
        _: Ordering,
    ) -> Result<Ref<'tx, T>, Error> {
        unsafe {
            if mem::size_of::<T>() > 0 {
                // In a read only transaction, there is no read log, write log or gc.
                // The only thing that needs to be done is reading of the value, and then a check,
                // to see if that value was written before this transaction began.x
                let value = Ref::new(tcell.erased.read_acquire::<T>());
                if likely!(self
                    .pin_epoch()
                    .read_write_valid_lockable(&tcell.erased.current_epoch, Acquire))
                {
                    Ok(value)
                } else {
                    Err(Error::RETRY)
                }
            } else {
                Ok(Ref::new(mem::zeroed()))
            }
        }
    }
}
