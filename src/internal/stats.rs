use std::{cell::RefCell, sync::Mutex};

#[derive(Default, Debug)]
pub struct Size {
    min:   Option<u64>,
    max:   Option<u64>,
    total: Option<u64>,
    count: u64,
}

impl Size {
    pub fn record(&mut self, size: u64) {
        self.min = Some(self.min.map(|min| min.min(size)).unwrap_or(size));
        self.max = Some(self.max.map(|max| max.max(size)).unwrap_or(size));
        self.total = Some(self.total.map(|total| total + size).unwrap_or(size));
        self.count += 1;
    }

    pub fn merge(&mut self, rhs: &Self) {
        self.min = match (self.min, rhs.min) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.max = match (self.max, rhs.max) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        self.total = match (self.total, rhs.total) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
        self.count += rhs.count;
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
    read_transaction:          Event,
    write_transaction:         Event,
    read_transaction_failure:  Event,
    write_transaction_failure: Event,
    bloom_check:               Event,
    bloom_failure:             Event,
    bloom_success_slow:        Event,
    write_after_write:         Event,
    write_after_logged_read:   Size,
    write_word_size:           Size,
    read_size:                 Size,
}

impl Stats {
    fn print_summary(&self) {
        println!("{:#?}", self);
        let transactions = self.read_transaction.count + self.write_transaction.count;
        println!(
            "{:>12}: {:>12} {:>9} {:.2}%",
            "transactions",
            transactions,
            "fail rate",
            (self.read_transaction_failure.count + self.write_transaction_failure.count) as f64
                / transactions as f64
                * 100.0
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
