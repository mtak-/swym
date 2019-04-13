use std::{
    cell::RefCell,
    fmt::{self, Debug, Formatter},
    sync::Mutex,
};

#[derive(Copy, Clone, Default, Debug)]
struct MinMaxTotal {
    min:   u64,
    max:   u64,
    total: u64,
}

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

macro_rules! stats_func {
    ($(#[$attr:meta])* $name:ident: Event) => {
        #[inline]
        $(#[$attr])*
        pub(crate) fn $name() {
            if cfg!(feature = "stats") {
                (THREAD_STAT.get().borrow_mut().0).$name.happened()
            }
        }
    };
    ($(#[$attr:meta])* $name:ident: Size) => {
        #[inline]
        $(#[$attr])*
        pub(crate) fn $name(size: usize) {
            if cfg!(feature = "stats") {
                let size = size as u64;
                (THREAD_STAT.get().borrow_mut().0).$name.record(size)
            }
        }
    };
}

macro_rules! stats {
    ($($(#[$attr:meta])* $names:ident: $kinds:tt),* $(,)*) => {
        #[derive(Default, Debug)]
        pub struct Stats {
            $($names: $kinds),*
        }

        impl Stats {
            fn merge(&mut self, rhs: &Self) {
                $(self.$names.merge(&rhs.$names));*
            }
        }

        $(stats_func!{$(#[$attr])* $names: $kinds})*
    };
}

stats! {
    /// Number of retries per successful read transaction.
    read_transaction_retries:         Size,

    /// Number of eager (before commit) retries per successful write transaction.
    write_transaction_eager_retries:  Size,

    /// Number of commit retries per successful write transaction.
    write_transaction_commit_retries: Size,

    /// Number of hardware retries per successful hardware transaction or software fallback.
    ///
    /// This is a less obvious metric. If a transaction completely fails and retries from the start
    /// 10 times, each one attempting a hardware commit, then this will be recorded 10 times with
    /// 10 different values.
    htm_retries:                      Size,

    /// Number of `TCell`s in the read log at commit time.
    read_size:                        Size,

    /// Number of cpu words in the write log at commit time. Each write is a minimum of 3 words.
    write_word_size:                  Size,

    /// A bloom filter check.
    bloom_check:                      Event,

    /// A bloom filter collision.
    bloom_collision:                  Event,

    /// A bloom filter hit that required a full lookup to verify.
    bloom_success_slow:               Event,

    /// A transactional read of data that exists in the write log. Considered slow.
    read_after_write:                 Event,

    /// A transactional overwrite of data that exists in the write log. Considered slow.
    write_after_write:                Event,

    /// Number of transactional writes to data that has been logged as read from first. Considered slowish.
    ///
    /// Writes after logged reads currently causes the commit algorithm to do more work.
    write_after_logged_read:          Size,
}

impl Stats {
    pub fn print_summary(&self) {
        println!("{:#?}", self);

        // Retries are recorded once after the transaction has completed. Eager retries and commit
        // retries are recorded in equal amounts, so just picking one of them is correct here.
        let successful_transactions =
            self.read_transaction_retries.count + self.write_transaction_eager_retries.count;

        let retries = self
            .read_transaction_retries
            .min_max_total
            .unwrap_or_default()
            .total
            + self
                .write_transaction_eager_retries
                .min_max_total
                .unwrap_or_default()
                .total
            + self
                .write_transaction_commit_retries
                .min_max_total
                .unwrap_or_default()
                .total;
        println!(
            "{:>12}: {:>12} {:>9}: {:.4} {:>13}: {:.4}",
            "transactions",
            successful_transactions,
            "retry avg",
            retries as f64 / successful_transactions as f64,
            "htm retry avg",
            self.htm_retries.min_max_total.unwrap_or_default().total as f64
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

#[derive(Default)]
struct ThreadStats(Stats);

impl Drop for ThreadStats {
    fn drop(&mut self) {
        self.flush()
    }
}

impl ThreadStats {
    pub fn flush(&mut self) {
        GLOBAL.get().lock().unwrap().merge(&self.0);
        self.0 = Default::default()
    }
}

phoenix_tls! {
    static THREAD_STAT: RefCell<ThreadStats> = {
        drop(GLOBAL.get()); // initialize global now, else we may get panics on drop because
                            // lazy_static uses thread_locals to initialize it.
        RefCell::default()
    };
}

fast_lazy_static! {
    static GLOBAL: Mutex<Stats> = Mutex::default();
}

pub fn print_stats() {
    if cfg!(feature = "stats") {
        GLOBAL.get().lock().unwrap().print_summary();
    } else {
        println!("`swym/stats` feature is not enabled")
    }
}

pub fn thread_flush() {
    if cfg!(feature = "stats") {
        THREAD_STAT.get().borrow_mut().flush()
    }
}
