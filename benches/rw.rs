#![feature(test)]

extern crate test;

#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

mod rw {
    use swym::thread_key;
    use test::Bencher;

    #[bench]
    fn standard_key(b: &mut Bencher) {
        const ITER_COUNT: usize = 1_000_000;
        let thread_key = thread_key::get();
        b.iter(|| {
            for _ in 0..ITER_COUNT {
                thread_key.rw(|_| Ok(()))
            }
        })
    }

    #[bench]
    fn try_key(b: &mut Bencher) {
        const ITER_COUNT: usize = 1_000_000;
        let thread_key = thread_key::get();
        b.iter(|| {
            for _ in 0..ITER_COUNT {
                thread_key.try_rw(|_| Ok(())).unwrap()
            }
        })
    }
}
