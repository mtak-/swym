#![feature(test)]

extern crate test;

#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

mod thread_key {
    use test::Bencher;

    #[bench]
    fn thread_key(b: &mut Bencher) {
        const ITER_COUNT: usize = 1_000_000;
        drop(swym::thread_key::get());
        b.iter(|| {
            for _ in 0..ITER_COUNT {
                drop(swym::thread_key::get())
            }
        })
    }
}
