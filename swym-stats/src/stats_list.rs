stats! {
    /// Number of conflicts per successful read transaction.
    read_transaction_conflicts:         Size @ READ_TRANSACTION_CONFLICTS,

    /// Number of eager (before commit) conflicts per successful write transaction.
    write_transaction_eager_conflicts:  Size @ WRITE_TRANSACTION_EAGER_CONFLICTS,

    /// Number of commit conflicts per successful write transaction.
    write_transaction_commit_conflicts: Size @ WRITE_TRANSACTION_COMMIT_CONFLICTS,

    /// Number of hardware conflicts per successful hardware transaction or software fallback.
    ///
    /// This is a less obvious metric. If a transaction completely fails and conflicts from the
    /// start 10 times, each one attempting a hardware commit, then this will be recorded 10 times
    /// with 10 different values.
    htm_conflicts:                      Size @ HTM_CONFLICTS,

    /// Number of hardware transactional retries before an explicit hardware transaction abort.
    htm_abort:                          Size @ HTM_ABORT,

    /// Number of `TCell`s in the read log at commit time.
    read_size:                          Size @ READ_SIZE,

    /// Number of cpu words in the write log at commit time. Each write is a minimum of 3 words.
    write_word_size:                    Size @ WRITE_WORD_SIZE,

    /// A bloom filter check.
    bloom_check:                       Event @ BLOOM_CHECK,

    /// A bloom filter collision.
    bloom_collision:                   Event @ BLOOM_CHECK,

    /// A bloom filter hit that required a full lookup to verify.
    bloom_success_slow:                Event @ BLOOM_SUCCESS_SLOW,

    /// A transactional read of data that exists in the write log. Considered slow.
    read_after_write:                  Event @ READ_AFTER_WRITE,

    /// A transactional overwrite of data that exists in the write log. Considered slow.
    write_after_write:                 Event @ WRITE_AFTER_WRITE,

    /// Number of transactional writes to data that has been logged as read from first. Considered
    /// slowish.
    ///
    /// Writes after logged reads currently causes the commit algorithm to do more work.
    write_after_logged_read:            Size @ WRITE_AFTER_LOGGED_READ,

    /// Number of threads that were blocked by a starvation event.
    blocked_by_starvation:              Size @ BLOCKED_BY_STARVATION,

    /// Number of times the starvation control was handed off from thread to thread.
    starvation_handoff:                Event @ STARVATION_HANDOFF,

    /// Number of times a garbage collection cycle hit the maximum Backoff during quiescing.
    should_park_gc:                     Size @ SHOULD_PARK_GC,

    /// Number of threads awaiting retry ([`AWAIT_RETRY`](crate::tx::Status::AWAIT_RETRY)) woken up
    /// per call to unpark.
    unparked_size:                      Size @ UNPARKED_SIZE,

    /// Number of threads awaiting retry ([`AWAIT_RETRY`](crate::tx::Status::AWAIT_RETRY)) that were
    /// _not_ woken up per call to unpark.
    not_unparked_size:                  Size @ NOT_UNPARKED_SIZE,

    /// When a thread attempts to [`AWAIT_RETRY`](crate::tx::Status::AWAIT_RETRY), this is the
    /// number of times it attempted a HTM park, and failed.
    htm_park_conflicts:                 Size @ HTM_PARK_CONFLICTS,

    /// When a thread attempts to [`AWAIT_RETRY`](crate::tx::Status::AWAIT_RETRY), but one of the
    /// waited on `EpochLock`'s gets modified before being parked.
    park_failure_size:                  Size @ PARK_FAILURE_SIZE,

    /// Number of `EpochLock`s a parked (via [`AWAIT_RETRY`](crate::tx::Status::AWAIT_RETRY)) thread
    /// can be woken up from.
    parked_size:                        Size @ PARKED_SIZE,
}
