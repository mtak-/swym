#![feature(test)]

extern crate test;

#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

mod set_one {
    use crossbeam_utils::thread;
    use swym::{tcell::TCell, thread_key};
    use test::Bencher;

    mod boxed {
        use super::*;

        #[bench]
        fn run(b: &mut Bencher) {
            const ITER_COUNT: usize = 1_000_000;
            let x = TCell::new(Box::new(0usize));
            thread::scope(|scope| {
                scope.spawn(move |_| {
                    let thread_key = thread_key::get();
                    b.iter(|| {
                        for _ in 0..ITER_COUNT {
                            thread_key.rw(|tx| {
                                x.set(tx, Box::new(0))?;
                                Ok(())
                            });
                        }
                    })
                });
            })
            .unwrap();
            swym::stats::print_stats();
        }
    }

    mod drop {
        use super::*;

        struct NeedsDrop(u8);
        impl Drop for NeedsDrop {
            #[inline]
            fn drop(&mut self) {}
        }

        #[bench]
        fn run(b: &mut Bencher) {
            const ITER_COUNT: usize = 1_000_000;
            let x = TCell::new(NeedsDrop(0));
            thread::scope(|scope| {
                scope.spawn(move |_| {
                    let thread_key = thread_key::get();
                    b.iter(|| {
                        for _ in 0..ITER_COUNT {
                            thread_key.rw(|tx| {
                                x.set(tx, NeedsDrop(0))?;
                                Ok(())
                            });
                        }
                    })
                });
            })
            .unwrap();
            swym::stats::print_stats();
        }
    }

    mod usize {
        use super::*;

        #[bench]
        fn run(b: &mut Bencher) {
            const ITER_COUNT: usize = 1_000_000;
            let x = TCell::new(0usize);
            thread::scope(|scope| {
                scope.spawn(move |_| {
                    let thread_key = thread_key::get();
                    b.iter(|| {
                        for _ in 0..ITER_COUNT {
                            thread_key.rw(|tx| {
                                x.set(tx, 0)?;
                                Ok(())
                            });
                        }
                    })
                });
            })
            .unwrap();
            swym::stats::print_stats();
        }
    }
}
