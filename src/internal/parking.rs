use crate::{
    internal::{
        epoch::{QuiesceEpoch, EPOCH_CLOCK},
        thread::{Logs, ParkPinMutRef, PinMutRef, PinRw},
    },
    stats,
};
use crossbeam_utils::Backoff;
use parking_lot_core::{FilterOp, ParkResult, ParkToken, DEFAULT_UNPARK_TOKEN};
use swym_htm::HardwareTx;

#[inline]
fn key() -> usize {
    // The EPOCH_CLOCK global is used as the key. This ties everything in this swym instance
    // together into the same queue.
    &EPOCH_CLOCK as *const _ as usize
}

fn parkable<'tx, 'tcell>(pin: PinMutRef<'tx, 'tcell>) -> bool {
    let logs = pin.logs();
    // parking a thread without any logs, will sleep the thread forever!
    !logs.read_log.is_empty() || !logs.write_log.is_empty()
}

unsafe fn try_clear_unpark_bits<'tcell>(logs: &Logs<'tcell>, pin_epoch: QuiesceEpoch) -> bool {
    const MAX_HTM_RETRIES: usize = 10;

    let mut retry_count = 0;
    let htx = if swym_htm::htm_supported() && logs.read_log.len() >= 3 {
        let retry_count = &mut retry_count;
        HardwareTx::new(|code| {
            if code.is_explicit_abort() || code.is_conflict() && !code.is_retry() {
                Err(false)
            } else if *retry_count < MAX_HTM_RETRIES {
                *retry_count += 1;
                Ok(())
            } else {
                Err(true)
            }
        })
    } else {
        Err(true)
    };
    let result = match htx {
        Ok(htx) => {
            // Try hardware transactional parking first. This could potentially eliminate many
            // cmpxchg's, and on park failure, all we have to do is abort the transaction.
            logs.read_log.clear_unpark_bits_htm(pin_epoch, &htx);
            logs.write_log.clear_unpark_bits_htm(pin_epoch, &htx);
            drop(htx);
            true
        }
        Err(true) => {
            // Software parking (e.g. cmpxchg).
            let park_statuses = logs.read_log.try_clear_unpark_bits(pin_epoch);
            match park_statuses {
                Some(statuses) => {
                    if logs.write_log.try_clear_unpark_bits(pin_epoch) {
                        true
                    } else {
                        logs.read_log.set_unpark_bits(statuses);
                        false
                    }
                }
                None => false,
            }
        }
        Err(false) => false,
    };
    stats::htm_park_conflicts(retry_count);
    result
}

#[inline]
unsafe fn should_unpark(ParkToken(token): ParkToken) -> bool {
    // TODO: parkpinmutref should be Send somehow
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

    // TODO: htm tag_parked
    let key = key();
    let park_token = ParkToken(parked_pin.park_token());
    let logs = &*parked_pin;
    let pin_epoch = parked_pin.pin_epoch;
    let validate = move || unsafe { try_clear_unpark_bits(logs, pin_epoch) };
    let before_sleep = || {};
    let timed_out = |_, _| {};

    match unsafe {
        parking_lot_core::park(key, validate, before_sleep, timed_out, park_token, None)
    } {
        ParkResult::Unparked(token) => {
            debug_assert_eq!(token, DEFAULT_UNPARK_TOKEN);
            let parked_size = logs.read_log.len() + logs.write_log.epoch_locks().count();
            stats::parked_size(parked_size);
            backoff.reset()
        }
        ParkResult::Invalid => {
            let parked_size = logs.read_log.len() + logs.write_log.epoch_locks().count();
            stats::park_failure_size(parked_size);
            backoff.snooze()
        }
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
    let mut not_unparked_count = 0;
    let unpark_result = unsafe {
        let not_unparked_count = &mut not_unparked_count;
        let filter = move |token| {
            if should_unpark(token) {
                FilterOp::Unpark
            } else {
                *not_unparked_count += 1;
                FilterOp::Skip
            }
        };
        parking_lot_core::unpark_filter(key, filter, callback)
    };
    stats::unparked_size(unpark_result.unparked_threads);
    stats::not_unparked_size(not_unparked_count);
}
