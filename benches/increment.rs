#![feature(test)]

extern crate test;

#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

mod increment {
    use crossbeam_utils::thread;
    use swym::{tcell::TCell, thread_key, tx::Ordering};
    use test::Bencher;

    #[bench]
    fn unlogged(b: &mut Bencher) {
        const ITER_COUNT: usize = 1_000_000;
        let x = TCell::new(0usize);
        thread::scope(|scope| {
            scope.spawn(move |_| {
                let thread_key = thread_key::get();
                b.iter(|| {
                    for _ in 0..ITER_COUNT {
                        thread_key.rw(|tx| {
                            x.set(tx, x.get(tx, Ordering::Read)?)?;
                            Ok(())
                        });
                    }
                })
            });
        })
        .unwrap();
        swym::stats::print_stats();
    }

    #[bench]
    fn logged(b: &mut Bencher) {
        const ITER_COUNT: usize = 1_000_000;
        let x = TCell::new(0usize);
        thread::scope(|scope| {
            scope.spawn(move |_| {
                let thread_key = thread_key::get();
                b.iter(|| {
                    for _ in 0..ITER_COUNT {
                        thread_key.rw(|tx| {
                            x.set(tx, x.get(tx, Ordering::default())?)?;
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
