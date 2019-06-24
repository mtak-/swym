#![deny(unused_must_use)]

#[macro_use]
extern crate criterion;

#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

mod rbtree {
    use criterion::{BatchSize, Benchmark, Criterion, Throughput};
    use crossbeam_utils::thread;
    use rand::{seq::SliceRandom, thread_rng};
    use std::{rc::Rc, time::Duration};
    use swym_rbtree::RBTreeMap;

    const SAMPLE_SIZE: usize = 24;
    const COUNT: usize = 100_000;
    // Caps the total allocation size at a value where the allocators performance doesnt start to
    // crumble
    const NUM_ITERATIONS: u64 = COUNT as u64 * 300 / 100_000;
    const WARMUP_TIME_NS: u64 = 1_000_000_000;

    fn random_data(count: usize) -> Rc<Vec<usize>> {
        let mut vec = Vec::new();
        for x in 0..count {
            vec.push(x);
        }
        let mut rng = thread_rng();
        vec.shuffle(&mut rng);
        Rc::new(vec)
    }

    fn spawn_chunked<F: Fn(usize) + Copy + Send + Sync>(data: &Vec<usize>, threads: usize, f: F) {
        let chunk = data.len() / threads;

        thread::scope(|scope| {
            for idx in 0..threads {
                let chunk = &data[idx * chunk..(idx + 1) * chunk];
                scope.spawn(move |_| {
                    for elem in chunk {
                        f(*elem)
                    }
                });
            }
        })
        .unwrap();
    }

    fn thread_benches<S, F, O>(
        name: &'static str,
        data: Rc<Vec<usize>>,
        setup: S,
        f: F,
    ) -> impl Iterator<Item = Benchmark>
    where
        S: Fn() -> O + Copy + 'static,
        F: Fn(&O, usize) + Copy + Send + Sync + 'static,
        O: Sync,
    {
        (1..=8).map(move |threads| {
            let throughput = Throughput::Elements(data.len() as _);
            let data = data.clone();
            Benchmark::new(
                format!(
                    "{name}_{threads:002}_{count}",
                    name = name,
                    threads = threads,
                    count = data.len()
                ),
                move |bencher| {
                    let data = &data;
                    bencher.iter_batched(
                        setup,
                        move |o| {
                            spawn_chunked(data, threads, |elem| f(&o, elem));
                            o
                        },
                        BatchSize::NumIterations(NUM_ITERATIONS),
                    );
                },
            )
            .sample_size(SAMPLE_SIZE)
            .warm_up_time(Duration::from_nanos(WARMUP_TIME_NS))
            .throughput(throughput)
        })
    }

    pub fn benches(c: &mut Criterion) {
        let data = random_data(COUNT);
        let const_tree = Box::new(RBTreeMap::new());
        spawn_chunked(&data, 8, |elem| drop(const_tree.insert(elem, 0)));
        let const_tree = unsafe { &*Box::into_raw(const_tree) };

        let benches = thread_benches(
            "insert",
            data.clone(),
            || RBTreeMap::new(),
            |tree, elem| drop(tree.insert(elem, 0)),
        )
        .chain(thread_benches(
            "entry",
            data.clone(),
            || RBTreeMap::new(),
            |tree, elem| {
                tree.atomic(move |mut tree| {
                    tree.entry(elem)?.or_insert(0)?;
                    Ok(())
                })
            },
        ))
        .chain(thread_benches(
            "get",
            data.clone(),
            || (),
            move |(), elem| {
                const_tree.get(&elem).unwrap();
            },
        ))
        .chain(thread_benches(
            "contains_key",
            data,
            || (),
            move |(), elem| {
                assert!(const_tree.contains_key(&elem));
            },
        ));
        for bench in benches {
            c.bench("rbtree", bench);
        }
        swym::stats::print_stats();
    }
}

criterion::criterion_group!(benches, rbtree::benches);
criterion::criterion_main!(benches);
