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
    ($name:ident: Event) => {
        #[inline]
        pub fn $name() {
            if cfg!(feature = "stats") {
                (THREAD_STAT.get().borrow_mut().0).$name.happened()
            }
        }
    };
    ($name:ident: Size) => {
        #[inline]
        pub fn $name(size: usize) {
            if cfg!(feature = "stats") {
                let size = size as u64;
                (THREAD_STAT.get().borrow_mut().0).$name.record(size)
            }
        }
    };
}

macro_rules! stats {
    ($($names:ident: $kinds:tt),* $(,)*) => {
        #[derive(Default, Debug)]
        pub struct Stats {
            $($names: $kinds),*
        }

        impl Stats {
            fn merge(&mut self, rhs: &Self) {
                $(self.$names.merge(&rhs.$names));*
            }
        }

        $(stats_func!{$names: $kinds})*
    };
}

stats! {
    read_transaction:                 Event,
    write_transaction:                Event,
    read_transaction_failure:         Event,
    write_transaction_eager_failure:  Event,
    write_transaction_commit_failure: Event,
    bloom_check:                      Event,
    bloom_failure:                    Event,
    bloom_success_slow:               Event,
    write_after_write:                Event,
    htm_failure_size:                 Size,
    write_after_logged_read:          Size,
    write_word_size:                  Size,
    read_size:                        Size,
}

impl Stats {
    fn print_summary(&self) {
        println!("{:#?}", self);
        let transactions = self.read_transaction.count + self.write_transaction.count;
        println!(
            "{:>12}: {:>12} {:>9} {:.2}% {:>9} {:.2}%",
            "transactions",
            transactions,
            "fail rate",
            (self.read_transaction_failure.count
                + self.write_transaction_eager_failure.count
                + self.write_transaction_commit_failure.count) as f64
                / transactions as f64
                * 100.0,
            "htm fails per tx rate",
            self.htm_failure_size
                .min_max_total
                .unwrap_or_default()
                .total as f64
                / transactions as f64
                * 100.0
        );
        println!(
            "{:>12}: {:>12.6}",
            "htm fail avg",
            self.htm_failure_size
                .min_max_total
                .unwrap_or_default()
                .total as f64
                / self.htm_failure_size.count as f64
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
