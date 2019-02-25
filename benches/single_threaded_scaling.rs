#![feature(test)]

extern crate test;

#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

mod single_threaded_scaling {
    use std::sync::{
        atomic::{AtomicUsize, Ordering::Relaxed},
        Mutex,
    };
    use swym::{tcell::TCell, thread_key, tx::Ordering};
    use test::Bencher;

    /// this demonstrates issues with the writelog
    macro_rules! write_count {
        ($name:ident, $lock_name:ident, $atomic_name:ident, $amount:expr) => {
            #[bench]
            fn $name(b: &mut Bencher) {
                const COUNT: usize = $amount;
                let mut x = Vec::new();
                for _ in 0..COUNT {
                    x.push(TCell::new(0))
                }
                let thread_key = thread_key::get();
                b.iter(|| {
                    thread_key.rw(|tx| {
                        for i in 0..COUNT {
                            let x_i = &x[i];
                            let next = x_i.get(tx, Ordering::Read)? + 1;
                            x_i.set(tx, next)?;
                        }
                        Ok(())
                    })
                })
            }

            #[bench]
            fn $lock_name(b: &mut Bencher) {
                const COUNT: usize = $amount;
                let mut x = Vec::new();
                for _ in 0..COUNT {
                    x.push(Mutex::new(0))
                }
                b.iter(|| {
                    for i in 0..COUNT {
                        let mut x_i = x[i].lock().unwrap();
                        *x_i += 1;
                    }
                })
            }

            #[bench]
            fn $atomic_name(b: &mut Bencher) {
                const COUNT: usize = $amount;
                let mut x = Vec::new();
                for _ in 0..COUNT {
                    x.push(AtomicUsize::new(0))
                }
                b.iter(|| {
                    for i in 0..COUNT {
                        x[i].fetch_add(1, Relaxed);
                    }
                })
            }
        };
        ($($names:ident, $lock_names:ident, $atomic_names:ident, $amounts:expr);*) => {
            $(write_count!{$names, $lock_names, $atomic_names, $amounts})*
        };
    }

    write_count! {
        write_01, lock_write_01, atomic_write_01, 1;
        write_02, lock_write_02, atomic_write_02, 2;
        write_04, lock_write_04, atomic_write_04, 4;
        write_08, lock_write_08, atomic_write_08, 8;
        write_16, lock_write_16, atomic_write_16, 16;
        write_32, lock_write_32, atomic_write_32, 32;

        // start to hit bloom filter failure here
        write_33, lock_write_33, atomic_write_33, 33;
        write_34, lock_write_34, atomic_write_34, 34;
        write_35, lock_write_35, atomic_write_35, 35;
        write_36, lock_write_36, atomic_write_36, 36;

        write_64, lock_write_64, atomic_write_64, 64
    }
}
