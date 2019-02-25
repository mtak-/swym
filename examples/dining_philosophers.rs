use crossbeam_utils::thread;
use swym::{
    tcell::TCell,
    thread_key,
    tx::{Error, Ordering},
};

const NUM_PHILOSOPHERS: usize = 5;
const FOOD_ITERATIONS: usize = 1_000_000;

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
                for _ in 0..FOOD_ITERATIONS {
                    thread_key.rw(|tx| {
                        if left_fork.in_use.get(tx, Ordering::Read)?
                            || right_fork.in_use.get(tx, Ordering::Read)?
                        {
                            Err(Error::RETRY)
                        } else {
                            left_fork.in_use.set(tx, true)?;
                            right_fork.in_use.set(tx, true)?;
                            Ok(())
                        }
                    });

                    // om nom nom

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
