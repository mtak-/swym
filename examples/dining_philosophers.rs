use crossbeam_utils::thread;
use swym::{
    tcell::TCell,
    thread_key,
    tx::{Error, Ordering},
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
    let mut forks = Vec::new();
    for _ in 0..NUM_PHILOSOPHERS {
        forks.push(Fork::new());
    }

    thread::scope(|scope| {
        for i in 0..NUM_PHILOSOPHERS {
            let left_fork = &forks[i];
            let right_fork = &forks[(i + 1) % NUM_PHILOSOPHERS];
            scope.spawn(move |_| {
                let thread_key = thread_key::get();
                for i in 0..FOOD_ITERATIONS {
                    thread_key.rw(|tx| {
                        if left_fork.in_use.get(tx, Ordering::default())?
                            || right_fork.in_use.get(tx, Ordering::default())?
                        {
                            Err(Error::RETRY)
                        } else {
                            left_fork.in_use.set(tx, true)?;
                            right_fork.in_use.set(tx, true)?;
                            Ok(())
                        }
                    });

                    println!("om nom nom {}", i);
                    std::thread::sleep(std::time::Duration::from_micros(EAT_TIME_MICROS));

                    thread_key.rw(|tx| {
                        left_fork.in_use.set(tx, false)?;
                        right_fork.in_use.set(tx, false)?;
                        Ok(())
                    })
                }
            });
        }
    })
    .unwrap();
}
