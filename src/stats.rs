//! Statistics collection. Enabled with `--features stats`.

use crate::internal::phoenix_tls::PhoenixTarget;
use core::{
    cell::RefCell,
    fmt::{self, Debug, Formatter},
    ops::{Deref, DerefMut},
};
use parking_lot::Mutex;

#[derive(Copy, Clone, Default, Debug)]
struct MinMaxTotal {
    min:   u64,
    max:   u64,
    total: u64,
}

#[doc(hidden)]
#[derive(Default)]
pub struct Size {
    min_max_total: Option<MinMaxTotal>,
    count:         u64,
}

impl Debug for Size {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Size")
            .field("count", &self.count)
            .field("min", &self.min_max_total.map(|x| x.min))
            .field("max", &self.min_max_total.map(|x| x.max))
            .field("total", &self.min_max_total.map(|x| x.total))
            .field(
                "avg",
                &self
                    .min_max_total
                    .map(|x| x.total as f64 / self.count as f64),
            )
            .finish()
    }
}

impl Size {
    pub(crate) fn record(&mut self, size: u64) {
        self.count += 1;
        if let Some(ref mut min_max_total) = &mut self.min_max_total {
            min_max_total.min = min_max_total.min.min(size);
            min_max_total.max = min_max_total.max.max(size);
            min_max_total.total += size;
        } else {
            self.min_max_total = Some(MinMaxTotal {
                min:   size,
                max:   size,
                total: size,
            });
        }
    }

    pub(crate) fn merge(&mut self, rhs: &Self) {
        self.count += rhs.count;
        self.min_max_total = match (self.min_max_total, rhs.min_max_total) {
            (Some(a), Some(b)) => Some(MinMaxTotal {
                min:   a.min.min(b.min),
                max:   a.max.max(b.max),
                total: a.total + b.total,
            }),
            (a, b) => a.or(b),
        };
    }
}

#[doc(hidden)]
#[derive(Default, Debug)]
pub struct Event {
    count: usize,
}

impl Event {
    fn happened(&mut self) {
        self.count += 1
    }

    fn merge(&mut self, rhs: &Self) {
        self.count += rhs.count
    }
}

macro_rules! env_var_set {
    ($env_var:ident) => {
        option_env!(concat!("SWYM_", stringify!($env_var))) == Some("1")
    };
}

macro_rules! stats_func {
    ($(#[$attr:meta])* $name:ident: Event @ $env_var:ident) => {
        #[inline]
        $(#[$attr])*
        pub(crate) fn $name() {
            if cfg!(feature = "stats") || env_var_set!($env_var) {
                THREAD_STAT.get().get().$name.happened()
            }
        }
    };
    ($(#[$attr:meta])* $name:ident: Size @ $env_var:ident) => {
        #[inline]
        $(#[$attr])*
        pub(crate) fn $name(size: usize) {
            if cfg!(feature = "stats") || env_var_set!($env_var) {
                let size = size as u64;
                THREAD_STAT.with(move |x| x.get().$name.record(size))
            }
        }
    };
}

macro_rules! stats {
    ($($(#[$attr:meta])* $names:ident: $kinds:tt @ $env_var:ident),* $(,)*) => {
        /// A collection of swym statistics.
        #[derive(Default, Debug)]
        pub struct Stats {
            $($(#[$attr])*pub $names: $kinds,)*

            __private: (),
        }

        impl Stats {
            fn merge(&mut self, rhs: &Self) {
                $(self.$names.merge(&rhs.$names));*
            }
        }

        fn any_stats_active() -> bool {
            cfg!(feature = "stats") $(|| env_var_set!($env_var))*
        }

        $(stats_func!{$(#[$attr])* $names: $kinds @ $env_var})*
    };
}

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

    /// Number of times a read transaction hit the maximum Backoff.
    should_park_read:                   Size @ SHOULD_PARK_READ,

    /// Number of times a read/write transaction hit the maximum Backoff.
    should_park_write:                  Size @ SHOULD_PARK_WRITE,

    /// Number of times a garbage collection cycle hit the maximum Backoff during quiescing.
    should_park_gc:                     Size @ SHOULD_PARK_GC,

    /// Number of threads awaiting retry ([`AWAIT_RETRY`]) woken up per call to unpark.
    unparked_size:                      Size @ UNPARKED_SIZE,

    /// Number of threads awaiting retry ([`AWAIT_RETRY`]) that were _not_ woken up per call to
    /// unpark.
    not_unparked_size:                  Size @ NOT_UNPARKED_SIZE,

    /// When a thread attempts to [`AWAIT_RETRY`], this is the number of times it attempted a HTM
    /// park, and failed.
    htm_park_conflicts:                 Size @ HTM_PARK_CONFLICTS,

    /// When a thread attempts to [`AWAIT_RETRY`], but one of the waited on `EpochLock`'s gets
    /// modified before being parked.
    park_failure_size:                  Size @ PARK_FAILURE_SIZE,

    /// Number of `EpochLock`s a parked (via [`AWAIT_RETRY`]) thread can be woken up from.
    parked_size:                        Size @ PARKED_SIZE,
}

impl Stats {
    /// Prints a summary of the stats object.
    pub fn print_summary(&self) {
        println!("{:#?}", self);

        // Retries are recorded once after the transaction has completed. Eager conflicts and commit
        // conflicts are recorded in equal amounts, so just picking one of them is correct here.
        let successful_transactions =
            self.read_transaction_conflicts.count + self.write_transaction_eager_conflicts.count;

        let conflicts = self
            .read_transaction_conflicts
            .min_max_total
            .unwrap_or_default()
            .total
            + self
                .write_transaction_eager_conflicts
                .min_max_total
                .unwrap_or_default()
                .total
            + self
                .write_transaction_commit_conflicts
                .min_max_total
                .unwrap_or_default()
                .total;
        println!(
            "{:>12}: {:>12} {:>9}: {:.4} {:>13}: {:.4}",
            "transactions",
            successful_transactions,
            "conflict avg",
            conflicts as f64 / successful_transactions as f64,
            "htm conflict avg",
            self.htm_conflicts.min_max_total.unwrap_or_default().total as f64
                / successful_transactions as f64
        );
        println!(
            "{:>12}: {:>12} {:>9}: {:.4} {:>13}: {:.4}",
            "bloom checks",
            self.bloom_check.count,
            "fail rate",
            self.bloom_collision.count as f64 / self.bloom_check.count as f64,
            "slow rate",
            (self.bloom_success_slow.count + self.bloom_collision.count) as f64
                / self.bloom_check.count as f64
        );
    }
}

/// Thread local statistics.
///
/// To reduce overhead of stats tracking, each thread has it's own `Stats` object which is flushed
/// to the global `Stats` object on thread exit or when manually requested.
#[derive(Debug)]
pub struct ThreadStats(RefCell<Stats>);

impl Default for ThreadStats {
    #[inline]
    fn default() -> Self {
        fn force(_: &Mutex<Stats>) {}
        force(&GLOBAL); // initialize global now, else we may get panics on drop because
                        // lazy_static uses thread_locals to initialize it.
        ThreadStats(Default::default())
    }
}

impl Drop for ThreadStats {
    #[inline]
    fn drop(&mut self) {
        self.flush()
    }
}

impl PhoenixTarget for ThreadStats {}

impl ThreadStats {
    /// Returns the actual statistics object.
    pub fn get<'a>(&'a self) -> impl DerefMut<Target = Stats> + 'a {
        self.0.borrow_mut()
    }

    /// Flushes the thread stats to the global thread stats object.
    ///
    /// After flushing, `self` is reset.
    pub fn flush(&mut self) {
        let mut borrow = self.get();
        GLOBAL.lock().merge(&*borrow);
        *borrow = Default::default()
    }
}

phoenix_tls! {
    static THREAD_STAT: ThreadStats
}

lazy_static::lazy_static! {
    static ref GLOBAL: Mutex<Stats> = Mutex::default();
}

/// Returns the global stats object, or None if the feature is disabled.
pub fn stats() -> Option<impl Deref<Target = Stats>> {
    if any_stats_active() {
        Some(GLOBAL.lock())
    } else {
        None
    }
}

/// Returns the thread local stats object, or None if the feature is disabled.
pub fn thread_stats() -> Option<impl Deref<Target = ThreadStats>> {
    if any_stats_active() {
        Some(THREAD_STAT.get())
    } else {
        None
    }
}

/// Prints a summary of the global stats object.
///
/// It may be necessary to run `stats::thread_stats().borrow_mut().flush()` first.
pub fn print_stats() {
    match self::stats() {
        Some(stats) => stats.print_summary(),
        None => println!("`swym/stats` feature is not enabled"),
    }
}
