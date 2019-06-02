#![feature(test)]

extern crate test;

#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

mod single_threaded_scaling {
    use parking_lot::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    use swym::{tcell::TCell, thread_key, tx::Ordering};
    use test::Bencher;

    /// This should reveal performance cliffs and regressions in the write log.
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
                        let mut x_i = x[i].lock();
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
        write_001, lock_write_001, atomic_write_001, 1;
        write_002, lock_write_002, atomic_write_002, 2;
        write_004, lock_write_004, atomic_write_004, 4;
        write_008, lock_write_008, atomic_write_008, 8;
        write_016, lock_write_016, atomic_write_016, 16;
        write_032, lock_write_032, atomic_write_032, 32;
        write_063, lock_write_063, atomic_write_063, 63;

        // start to hit bloom filter failure here
        write_064, lock_write_064, atomic_write_064, 64;
        write_065, lock_write_065, atomic_write_065, 65;
        write_066, lock_write_066, atomic_write_066, 66;
        write_067, lock_write_067, atomic_write_067, 67;
        write_068, lock_write_068, atomic_write_068, 68;

        write_128, lock_write_128, atomic_write_128, 128;
        write_256, lock_write_256, atomic_write_256, 256;
        write_512, lock_write_512, atomic_write_512, 512
    }
}
