use crossbeam_utils::thread;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use swym::{
    tcell::TCell,
    thread_key,
    tx::{Ordering, Status},
};

const NUM_PHILOSOPHERS: usize = 5;
const FOOD_ITERATIONS: usize = 100_000;
const EAT_TIME_MICROS: u64 = 1;

struct Fork {
    in_use: TCell<bool>,
}

impl Fork {
    const fn new() -> Self {
        Fork {
            in_use: TCell::new(false),
        }
    }
}

/// a bit contrived
fn main() {
    let total_retry_count = AtomicUsize::new(0);
    let total_retry_count = &total_retry_count;

    let mut forks = Vec::new();
    for _ in 0..NUM_PHILOSOPHERS {
        forks.push(Fork::new());
    }

    thread::scope(|scope| {
        for i in 0..NUM_PHILOSOPHERS {
            let left_fork = &forks[i];
            let right_fork = &forks[(i + 1) % NUM_PHILOSOPHERS];
            scope.spawn(move |_| {
                let mut retry_count = 0;
                let thread_key = thread_key::get();
                for _i in 0..FOOD_ITERATIONS {
                    thread_key.rw(|tx| {
                        if left_fork.in_use.get(tx, Ordering::default())?
                            || right_fork.in_use.get(tx, Ordering::default())?
                        {
                            retry_count += 1;
                            Err(Status::AWAIT_RETRY)
                        } else {
                            left_fork.in_use.set(tx, true)?;
                            right_fork.in_use.set(tx, true)?;
                            Ok(())
                        }
                    });

                    // println!("om nom nom {}", _i);
                    std::thread::sleep(std::time::Duration::from_micros(EAT_TIME_MICROS));

                    thread_key.rw(|tx| {
                        left_fork.in_use.set(tx, false)?;
                        right_fork.in_use.set(tx, false)?;
                        Ok(())
                    })
                }
                total_retry_count.fetch_add(retry_count, Relaxed);
            });
        }
    })
    .unwrap();
    println!("Total Retry Count: {:?}", total_retry_count.load(Relaxed));
}
