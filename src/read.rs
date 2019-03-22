use crate::{
    internal::{epoch::QuiesceEpoch, thread::Pin},
    tcell::{Ref, TCell},
    tx::{Borrow, Error, Ordering, Read},
};
use std::{marker::PhantomData, mem};

/// A read only transaction.
///
/// No instances of this type are ever created. References to values of this type are created by
/// transmuting QuiesceEpoch's.
///
/// The lifetime contravariance allows conversions of ReadTx<'a> into ReadTx<'static>.
pub struct ReadTx<'tcell>(PhantomData<fn(&'tcell ())>);
impl<'tcell> !Send for ReadTx<'tcell> {}
impl<'tcell> !Sync for ReadTx<'tcell> {}

impl<'tcell> ReadTx<'tcell> {
    #[inline]
    pub(crate) fn new<'tx>(pin: &'tx mut Pin<'tcell>) -> &'tx Self {
        assert!(mem::align_of::<Self>() == 1, "unsafe alignment on ReadTx");
        // we smuggle the pinned epoch through as a reference
        // QuiesceEpoch is NonZero
        let pin_epoch: QuiesceEpoch = pin.pin_epoch();
        unsafe { mem::transmute::<QuiesceEpoch, &'tx Self>(pin_epoch) }
    }

    #[inline]
    fn pin_epoch(&self) -> QuiesceEpoch {
        // convert the reference back into the smuggled pinned epoch
        unsafe { mem::transmute::<&Self, _>(self) }
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
            if mem::size_of::<T>() != 0 {
                // In a read only transaction, there is no read log, write log or gc.
                // The only thing that needs to be done is reading of the value, and then a check,
                // to see if that value was written before this transaction began.
                let value = Ref::new(tcell.optimistic_read_acquire());
                if likely!(self
                    .pin_epoch()
                    .read_write_valid_lockable(&tcell.erased.current_epoch))
                {
                    Ok(value)
                } else {
                    Err(Error::RETRY)
                }
            } else {
                // If the type is zero sized, there's no need to any synchronization.
                Ok(Ref::new(mem::zeroed()))
            }
        }
    }
}
