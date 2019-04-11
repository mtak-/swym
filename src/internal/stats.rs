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
    pub fn record(&mut self, size: u64) {
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

    pub fn merge(&mut self, rhs: &Self) {
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
        $($attr)*
        pub fn $name() {
            if cfg!(feature = "stats") {
                (THREAD_STAT.get().borrow_mut().0).$name.happened()
            }
        }
    };
    ($(#[$attr:meta])* $name:ident: Size) => {
        #[inline]
        $(#[$attr])*
        pub fn $name(size: usize) {
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
    read_transaction_retries:         Size,
    write_transaction_eager_retries:  Size,
    write_transaction_commit_retries: Size,
    htm_retries:                      Size,
    read_size:                        Size,
    write_word_size:                  Size,
    bloom_check:                      Event,
    bloom_failure:                    Event,
    bloom_success_slow:               Event,
    write_after_write:                Event,
    write_after_logged_read:          Size,
}

impl Stats {
    fn print_summary(&self) {
        println!("{:#?}", self);

        // Retries are recorded once after the transaction has completed. Eager retries and commit
        // retries are recorded in equal amounts, so just picking one of them is correct here.
        let successful_transactions =
            self.read_transaction_retries.count + self.write_transaction_eager_retries.count;

        let failures = self
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
            "{:>12}: {:>12} {:>9} {:.2}% {:>9} {:.2}%",
            "transactions",
            successful_transactions,
            "fail avg",
            failures as f64 / successful_transactions as f64 * 100.0,
            "htm fails avg",
            self.htm_retries.min_max_total.unwrap_or_default().total as f64
                / successful_transactions as f64
                * 100.0
        );
        println!(
            "{:>12}: {:>12.6}",
            "htm retry avg",
            self.htm_retries.min_max_total.unwrap_or_default().total as f64
                / self.htm_retries.count as f64
        );
        println!(
            "{:>12}: {:>12} {:>9} {:.2}% {:>9} {:.2}%",
            "bloom checks",
            self.bloom_check.count,
            "fail rate",
            self.bloom_failure.count as f64 / self.bloom_check.count as f64 * 100.0,
            "slow rate",
            (self.bloom_success_slow.count + self.bloom_failure.count) as f64
                / self.bloom_check.count as f64
                * 100.0
        );
    }
}

#[derive(Default)]
struct ThreadStats(Stats);

impl Drop for ThreadStats {
    fn drop(&mut self) {
        GLOBAL.get().lock().unwrap().merge(&self.0);
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
