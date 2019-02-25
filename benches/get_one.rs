#![feature(test)]

extern crate test;

#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

mod get_one {
    use swym::{tcell::TCell, thread_key, tx::Ordering};
    use test::Bencher;

    mod read {
        mod boxed {
            use super::super::*;

            #[bench]
            fn run(b: &mut Bencher) {
                const ITER_COUNT: usize = 1_000_000;
                let thread_key = thread_key::get();
                let x = TCell::new(Box::new(0usize));
                b.iter(|| {
                    for _ in 0..ITER_COUNT {
                        thread_key.read(|tx| Ok(**x.borrow(tx, Ordering::default())?));
                    }
                })
            }
        }

        mod usize {
            use super::super::*;

            #[bench]
            fn run(b: &mut Bencher) {
                const ITER_COUNT: usize = 1_000_000;
                let thread_key = thread_key::get();
                let x = TCell::new(0usize);
                b.iter(|| {
                    for _ in 0..ITER_COUNT {
                        thread_key.read(|tx| Ok(x.get(tx, Ordering::default())?));
                    }
                })
            }
        }
    }

    mod rw_logged {
        mod boxed {
            use super::super::*;

            #[bench]
            fn run(b: &mut Bencher) {
                const ITER_COUNT: usize = 1_000_000;
                let thread_key = thread_key::get();
                let x = TCell::new(Box::new(0usize));
                b.iter(|| {
                    for _ in 0..ITER_COUNT {
                        thread_key.rw(|tx| Ok(**x.borrow(tx, Ordering::default())?));
                    }
                })
            }
        }

        mod usize {
            use super::super::*;

            #[bench]
            fn run(b: &mut Bencher) {
                const ITER_COUNT: usize = 1_000_000;
                let thread_key = thread_key::get();
                let x = TCell::new(0usize);
                b.iter(|| {
                    for _ in 0..ITER_COUNT {
                        thread_key.rw(|tx| Ok(*x.borrow(tx, Ordering::default())?));
                    }
                })
            }
        }
    }

    mod rw_unlogged {
        mod usize {
            use super::super::*;

            #[bench]
            fn run(b: &mut Bencher) {
                const ITER_COUNT: usize = 1_000_000;
                let thread_key = thread_key::get();
                let x = TCell::new(0usize);
                b.iter(|| {
                    for _ in 0..ITER_COUNT {
                        thread_key.rw(|tx| Ok(*x.borrow(tx, Ordering::Read)?));
                    }
                })
            }
        }

        mod boxed {
            use super::super::*;

            #[bench]
            fn run(b: &mut Bencher) {
                const ITER_COUNT: usize = 1_000_000;
                let thread_key = thread_key::get();
                let x = TCell::new(Box::new(0usize));
                b.iter(|| {
                    for _ in 0..ITER_COUNT {
                        thread_key.rw(|tx| Ok(**x.borrow(tx, Ordering::Read)?));
                    }
                })
            }
        }
    }
}
