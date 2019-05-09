use crate::internal::{
    epoch::{QuiesceEpoch, EPOCH_CLOCK},
    thread::{Logs, ParkPinMutRef, PinMutRef, PinRw},
};
use crossbeam_utils::Backoff;
use parking_lot_core::{FilterOp, ParkResult, ParkToken, DEFAULT_UNPARK_TOKEN};

#[inline]
fn key() -> usize {
    // The EPOCH_CLOCK global is used as the key. This ties everything in this swym instance
    // together into the same queue.
    &EPOCH_CLOCK as *const _ as usize
}

fn parkable<'tx, 'tcell>(pin: PinMutRef<'tx, 'tcell>) -> bool {
    let logs = pin.logs();
    !logs.read_log.is_empty() || !logs.write_log.is_empty()
}

#[inline]
fn park_validate<'tcell>(logs: &Logs<'tcell>, pin_epoch: QuiesceEpoch) -> bool {
    if logs.read_log.try_clear_unpark_bits(pin_epoch) {
        if logs.write_log.try_clear_unpark_bits(pin_epoch) {
            true
        } else {
            logs.read_log.set_unpark_bits();
            false
        }
    } else {
        false
    }
}

#[inline]
unsafe fn should_unpark(ParkToken(token): ParkToken) -> bool {
    let parked_pin = ParkPinMutRef::from_park_token(token);
    let pin_epoch = parked_pin.pin_epoch;
    !parked_pin.read_log.validate_reads(pin_epoch)
        || !parked_pin.write_log.validate_writes(pin_epoch)
}

#[inline(never)]
#[cold]
pub fn park<'tx, 'tcell>(mut pin: PinRw<'tx, 'tcell>, backoff: &Backoff) {
    debug_assert!(
        parkable(pin.reborrow()),
        "`AWAIT_RETRY` on a transaction that has an empty read set causes the thread to sleep \
         forever in release"
    );

    let parked_pin = pin.parked();

    // TODO: stats
    // TODO: Attempt htm tag_parked
    let key = key();
    let park_token = ParkToken(parked_pin.park_token());
    let logs = &*parked_pin;
    let pin_epoch = parked_pin.pin_epoch;
    let validate = move || park_validate(logs, pin_epoch);
    let before_sleep = || {};
    let timed_out = |_, _| {};

    match unsafe {
        parking_lot_core::park(key, validate, before_sleep, timed_out, park_token, None)
    } {
        ParkResult::Unparked(token) => {
            debug_assert_eq!(token, DEFAULT_UNPARK_TOKEN);
            backoff.reset()
        }
        ParkResult::Invalid => backoff.snooze(),
        ParkResult::TimedOut => {
            if cfg!(debug_assertions) {
                panic!("unexpected timeout on parked thread")
            }
        }
    }
    drop(parked_pin);
}

#[inline(never)]
#[cold]
pub fn unpark() {
    let key = key();
    let callback = |_| DEFAULT_UNPARK_TOKEN;
    let _unpark_result = unsafe {
        let filter = move |token| {
            if should_unpark(token) {
                FilterOp::Unpark
            } else {
                FilterOp::Skip
            }
        };
        parking_lot_core::unpark_filter(key, filter, callback)
    };
    // TODO: stats logging
}
