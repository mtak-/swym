use crate::internal::{
    epoch::EPOCH_CLOCK,
    thread::{PinMutRef, PinRef},
};
use parking_lot_core::{FilterOp, ParkResult, ParkToken, DEFAULT_UNPARK_TOKEN};

#[inline]
fn key() -> usize {
    // The EPOCH_CLOCK global is used as the key. This ties everything in this swym instance
    // together into the same queue.
    &EPOCH_CLOCK as *const _ as usize
}

#[inline(never)]
#[cold]
pub fn park<'tx, 'tcell>(pin: &PinMutRef<'tx, 'tcell>) {
    if pin.parkable() {
        // TODO: stats
        unsafe {
            let key = key();
            let park_token = ParkToken(pin.park_token());
            let validate = move || pin.park_validate();
            let before_sleep = || {};
            let timed_out = |_, _| {};
            match parking_lot_core::park(key, validate, before_sleep, timed_out, park_token, None) {
                ParkResult::Unparked(token) => debug_assert_eq!(token, DEFAULT_UNPARK_TOKEN),
                ParkResult::Invalid => {}
                ParkResult::TimedOut => {
                    if cfg!(debug_assertions) {
                        panic!("unexpected timeout on parked thread")
                    }
                }
            }
        }
    } else {
        panic!(
            "requesting a RETRY on a transaction with an empty read and write set puts the thread \
             to sleep forever"
        );
    }
}

#[inline(never)]
#[cold]
pub fn unpark() {
    let key = key();
    unsafe {
        let filter = move |ParkToken(token)| {
            if PinRef::should_unpark(token) {
                FilterOp::Unpark
            } else {
                FilterOp::Skip
            }
        };
        let callback = |_| DEFAULT_UNPARK_TOKEN;
        let _unpark_result = parking_lot_core::unpark_filter(key, filter, callback);
        // TODO: stats logging
    }
}
