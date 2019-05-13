use crate::{
    internal::{
        epoch::{EpochLock, ParkStatus, EPOCH_CLOCK},
        thread::PinRw,
    },
    stats,
};
use core::ptr;
use swym_htm::{BoundedHtxErr, HardwareTx};

const MAX_HTX_RETRIES: u8 = 3;

impl<'tx, 'tcell> PinRw<'tx, 'tcell> {
    /// The commit algorithm, called after user code has finished running without returning an
    /// error. Returns true if the transaction committed successfully.
    #[inline]
    pub fn commit(self) -> bool {
        if likely!(!self.logs().write_log.is_empty()) {
            self.commit_slow()
        } else {
            unsafe { self.commit_empty_write_log() }
        }
    }

    #[inline]
    unsafe fn commit_empty_write_log(self) -> bool {
        let (_, logs) = self.into_inner();
        // RwTx validates reads as they occur. As a result, if there are no writes, then we have
        // no work to do in our commit algorithm.
        //
        // On the off chance we do have garbage, with an empty write log. Then there's no way
        // that garbage could have been previously been shared, as the data cannot
        // have been made inaccessible via an STM write. It is a logic error in user
        // code, and requires `unsafe` to make that error. This assert helps to
        // catch that.
        debug_assert!(
            logs.garbage.is_speculative_bag_empty(),
            "Garbage queued, without any writes!"
        );
        logs.read_log.clear();
        true
    }

    #[inline]
    fn start_htx(&self, retry_count: &mut u8) -> Result<HardwareTx, BoundedHtxErr> {
        if swym_htm::htm_supported() && self.logs().write_log.word_len() >= 9 {
            HardwareTx::bounded(retry_count, MAX_HTX_RETRIES)
        } else {
            Err(BoundedHtxErr::SoftwareFallback)
        }
    }

    #[inline]
    fn commit_slow(self) -> bool {
        let mut retry_count = 0;
        match self.start_htx(&mut retry_count) {
            Ok(htx) => {
                let success = self.commit_hard(htx);
                stats::htm_conflicts(retry_count as _);
                success
            }
            Err(BoundedHtxErr::SoftwareFallback) => {
                stats::htm_conflicts(retry_count as _);
                self.commit_soft()
            }
            Err(BoundedHtxErr::AbortOrConflict) => {
                stats::htm_conflicts(retry_count as _);
                false
            }
        }
    }

    #[inline(never)]
    fn commit_hard(self, htx: HardwareTx) -> bool {
        unsafe {
            let (synch, logs) = self.into_inner();
            let current = synch.current_epoch();
            logs.read_log.validate_reads_htm(current, &htx);
            let park_status = logs.write_log.write_and_lock_htm(&htx, current);

            drop(htx);

            let sync_epoch = EPOCH_CLOCK.fetch_and_tick();

            // publish
            logs.write_log
                .epoch_locks()
                .for_each(|epoch_lock| epoch_lock.unlock_publish(sync_epoch.next()));

            logs.read_log.clear();
            logs.write_log.clear_no_drop();
            if unlikely!(park_status == ParkStatus::HasParked) {
                crate::internal::parking::unpark();
            }
            logs.garbage.seal_with_epoch(synch, sync_epoch);

            true
        }
    }

    /// This performs a lot of lock cmpxchgs, so inlining doesn't really doesn't give us much.
    #[inline(never)]
    fn commit_soft(mut self) -> bool {
        // Locking the write log, would cause validation of any reads to the same TCell to fail.
        // So we remove all TCells in the read log that are also in the write log, and assume all
        // TCells in the write log were also in the read log.
        unsafe { self.logs_mut().remove_writes_from_reads() };
        let logs = self.logs();

        // Locking the write set can fail if another thread has the lock, or if any TCell in the
        // write set has been updated since the transaction began.
        let mut park_status = ParkStatus::NoParked;
        let pin_epoch = self.pin_epoch();
        let mut unlock_until = None;
        for epoch_lock in logs.write_log.epoch_locks() {
            match epoch_lock.try_lock(pin_epoch) {
                Some(cur_status) => park_status = park_status.merge(cur_status),
                None => {
                    unlock_until = Some(epoch_lock as *const _);
                    break;
                }
            }
        }
        unsafe {
            if let Some(unlock_until) = unlock_until {
                self.write_log_lock_failure(unlock_until)
            } else {
                self.write_log_lock_success(park_status)
            }
        }
    }

    #[inline]
    unsafe fn write_log_lock_success(self, park_status: ParkStatus) -> bool {
        // after locking the write set, ensure nothing in the read set has been modified.
        if likely!(self.logs().read_log.validate_reads(self.pin_epoch())) {
            // The transaction can no longer fail, so proceed to modify and publish the TCells in
            // the write set.
            self.validation_success(park_status)
        } else {
            self.validation_failure()
        }
    }

    #[cold]
    #[inline(never)]
    fn write_log_lock_failure(self, unlock_until: *const EpochLock) -> bool {
        self.logs()
            .write_log
            .epoch_locks()
            .take_while(move |&e| !ptr::eq(e, unlock_until))
            .for_each(|epoch_lock| unsafe { epoch_lock.unlock_undo() });
        false
    }

    #[inline]
    unsafe fn validation_success(self, park_status: ParkStatus) -> bool {
        let (synch, logs) = self.into_inner();

        // The writes must be performed before the EPOCH_CLOCK is tick'ed.
        // Reads can get away with performing less work with this ordering.
        logs.write_log.perform_writes();

        let sync_epoch = EPOCH_CLOCK.fetch_and_tick();
        debug_assert!(
            synch.current_epoch() <= sync_epoch,
            "`EpochClock::fetch_and_tick` returned an earlier time than expected"
        );

        // unlocks everything in the write lock and sets the TCell epochs to sync_epoch.next()
        logs.write_log
            .epoch_locks()
            .for_each(|epoch_lock| epoch_lock.unlock_publish(sync_epoch.next()));
        logs.read_log.clear();
        logs.write_log.clear_no_drop();
        if unlikely!(park_status == ParkStatus::HasParked) {
            crate::internal::parking::unpark();
        }
        logs.garbage.seal_with_epoch(synch, sync_epoch);

        true
    }

    #[inline(never)]
    #[cold]
    unsafe fn validation_failure(self) -> bool {
        // on fail unlock the write set
        self.logs()
            .write_log
            .epoch_locks()
            .for_each(|epoch_lock| epoch_lock.unlock_undo());
        false
    }
}