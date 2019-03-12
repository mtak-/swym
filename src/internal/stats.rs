use std::cell::RefCell;
#[cfg(feature = "stats")]
use std::sync::Mutex;

#[derive(Default, Debug)]
pub struct Size {
    min:   usize,
    max:   usize,
    avg:   f64,
    count: usize,
}

impl Size {
    pub fn record(&mut self, size: usize) {
        self.min = self.min.min(size);
        self.max = self.max.max(size);
        self.avg = (self.avg * self.count as f64 + size as f64) / ((self.count + 1) as f64);
        self.count += 1;
    }

    pub fn merge(&mut self, rhs: &Self) {
        self.min = self.min.min(rhs.min);
        self.max = self.max.max(rhs.max);
        self.avg = (self.avg * self.count as f64 + rhs.avg * rhs.count as f64)
            / (self.count + rhs.count) as f64;
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
                THREAD_STAT.with(|ts| (ts.borrow_mut().0).$name.happened())
            }
        }
    };
    ($name:ident: Size) => {
        #[inline]
        pub fn $name(size: usize) {
            if cfg!(feature = "stats") {
                THREAD_STAT.with(|ts| (ts.borrow_mut().0).$name.record(size))
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
            #[cfg_attr(not(feature = "stats"), allow(unused))]
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
    double_write:              Event,
    write_word_size:           Size,
    read_size:                 Size,
    unnecessary_read_size:     Size,
}

impl Stats {
    #[cfg_attr(not(feature = "stats"), allow(unused))]
    fn print_summary(&self) {
        println!("{:#?}", self);
        let transactions = self.read_transaction.count + self.write_transaction.count;
        println!(
            "{:>20}: {:>12} {:>9} {:.2}%",
            "transactions",
            transactions,
            "fail rate",
            (self.read_transaction_failure.count + self.write_transaction_failure.count) as f64
                / transactions as f64
                * 100.0
        );
        println!(
            "{:>20}: {:>12} {:>9} {:.2}% {:>9} {:.2}%",
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
        #[cfg(feature = "stats")]
        GLOBAL.lock().unwrap().merge(&self.0);
    }
}

thread_local! {
    static THREAD_STAT: RefCell<ThreadStats> = {
        #[cfg(feature = "stats")]
        drop(&*GLOBAL); // initialize global now, else we may get panics on drop because lazy_static
                        // uses thread_locals to initialize it.
        RefCell::default()
    };
}

#[cfg(feature = "stats")]
lazy_static! {
    static ref GLOBAL: Mutex<Stats> = Mutex::default();
}

pub fn print_stats() {
    #[cfg(feature = "stats")]
    GLOBAL.lock().unwrap().print_summary();
}
