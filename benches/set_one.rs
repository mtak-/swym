#![feature(test)]

extern crate test;

#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

mod set_one {
    use swym::{tcell::TCell, thread_key};
    use test::Bencher;

    mod boxed {
        use super::*;

        #[bench]
        fn run(b: &mut Bencher) {
            const ITER_COUNT: usize = 1_000_000;
            let thread_key = thread_key::get();
            let x = TCell::new(Box::new(0usize));
            b.iter(|| {
                for _ in 0..ITER_COUNT {
                    thread_key.rw(|tx| {
                        x.set(tx, Box::new(0))?;
                        Ok(())
                    });
                }
            })
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
            let thread_key = thread_key::get();
            let x = TCell::new(NeedsDrop(0));
            b.iter(|| {
                for _ in 0..ITER_COUNT {
                    thread_key.rw(|tx| {
                        x.set(tx, NeedsDrop(0))?;
                        Ok(())
                    });
                }
            })
        }
    }

    mod usize {
        use super::*;

        #[bench]
        fn run(b: &mut Bencher) {
            const ITER_COUNT: usize = 1_000_000;
            let thread_key = thread_key::get();
            let x = TCell::new(0usize);
            b.iter(|| {
                for _ in 0..ITER_COUNT {
                    thread_key.rw(|tx| {
                        x.set(tx, 0)?;
                        Ok(())
                    });
                }
            })
        }
    }
}
