// based off of https://en.wikipedia.org/wiki/Red%E2%80%93black_tree
// probly lots to optimize and cleanup

#![feature(test)]
#![deny(unused_must_use)]

extern crate test;

#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

mod rbtree {
    use crossbeam_utils::thread;
    use rand::{seq::SliceRandom, thread_rng};
    use swym_rbtree::RBTreeMap;
    use test::Bencher;

    macro_rules! insert_bench {
        ($name:ident, $count:expr, $threads:expr) => {
            #[bench]
            fn $name(bencher: &mut Bencher) {
                const COUNT: usize = $count;
                const THREADS: usize = $threads;

                let mut vec = Vec::new();
                for x in 0..COUNT {
                    vec.push(x);
                }
                let mut rng = thread_rng();
                vec.shuffle(&mut rng);
                let vec = &vec;

                bencher.iter(move || {
                    let _tree = RBTreeMap::new();
                    let tree = &_tree;
                    thread::scope(|scope| {
                        for idx in 0..THREADS {
                            scope.spawn(move |_| {
                                for elem in
                                    &vec[(idx * COUNT / THREADS)..((idx + 1) * COUNT / THREADS)]
                                {
                                    let elem = *elem;
                                    tree.insert(elem, 0);
                                }
                            });
                        }
                    })
                    .unwrap();
                    std::mem::forget(_tree);
                });
                swym::print_stats();
            }
        };
    }
    insert_bench! {insert_01_100000, 100000, 1}
    insert_bench! {insert_02_100000, 100000, 2}
    insert_bench! {insert_03_100000, 100000, 3}
    insert_bench! {insert_04_100000, 100000, 4}
    insert_bench! {insert_05_100000, 100000, 5}
    insert_bench! {insert_06_100000, 100000, 6}
    insert_bench! {insert_07_100000, 100000, 7}
    insert_bench! {insert_08_100000, 100000, 8}

    macro_rules! entry_bench {
        ($name:ident, $count:expr, $threads:expr) => {
            #[bench]
            fn $name(bencher: &mut Bencher) {
                const COUNT: usize = $count;
                const THREADS: usize = $threads;

                let mut vec = Vec::new();
                for x in 0..COUNT {
                    vec.push(x);
                }
                let mut rng = thread_rng();
                vec.shuffle(&mut rng);
                let vec = &vec;

                bencher.iter(move || {
                    let _tree = RBTreeMap::new();
                    let tree = &_tree;
                    thread::scope(|scope| {
                        for idx in 0..THREADS {
                            scope.spawn(move |_| {
                                for elem in
                                    &vec[(idx * COUNT / THREADS)..((idx + 1) * COUNT / THREADS)]
                                {
                                    let elem = *elem;
                                    tree.atomic(|mut tree| {
                                        tree.entry(elem)?.or_insert(0)?;
                                        Ok(())
                                    })
                                }
                            });
                        }
                    })
                    .unwrap();
                });
                swym::print_stats();
            }
        };
    }
    entry_bench! {entry_01_100000, 100000, 1}
    entry_bench! {entry_02_100000, 100000, 2}
    entry_bench! {entry_03_100000, 100000, 3}
    entry_bench! {entry_04_100000, 100000, 4}
    entry_bench! {entry_05_100000, 100000, 5}
    entry_bench! {entry_06_100000, 100000, 6}
    entry_bench! {entry_07_100000, 100000, 7}
    entry_bench! {entry_08_100000, 100000, 8}

    macro_rules! get_bench {
        ($name:ident, $count:expr, $threads:expr) => {
            #[bench]
            fn $name(bencher: &mut Bencher) {
                const COUNT: usize = $count;
                const THREADS: usize = $threads;

                let mut vec = Vec::new();
                for x in 0..COUNT {
                    vec.push(x);
                }
                let mut rng = thread_rng();
                vec.shuffle(&mut rng);
                let vec = &vec;
                let _tree = RBTreeMap::new();
                let tree = &_tree;
                thread::scope(|scope| {
                    for idx in 0..8 {
                        scope.spawn(move |_| {
                            for elem in &vec[(idx * COUNT / 8)..((idx + 1) * COUNT / 8)] {
                                tree.insert(*elem, 0);
                            }
                        });
                    }
                })
                .unwrap();

                bencher.iter(move || {
                    thread::scope(|scope| {
                        for idx in 0..THREADS {
                            scope.spawn(move |_| {
                                for elem in
                                    &vec[(idx * COUNT / THREADS)..((idx + 1) * COUNT / THREADS)]
                                {
                                    tree.get(elem).unwrap();
                                }
                            });
                        }
                    })
                    .unwrap();
                });
                swym::print_stats();
            }
        };
    }
    get_bench! {get_01_100000, 100000, 1}
    get_bench! {get_02_100000, 100000, 2}
    get_bench! {get_03_100000, 100000, 3}
    get_bench! {get_04_100000, 100000, 4}
    get_bench! {get_05_100000, 100000, 5}
    get_bench! {get_06_100000, 100000, 6}
    get_bench! {get_07_100000, 100000, 7}
    get_bench! {get_08_100000, 100000, 8}

    macro_rules! contains_key_bench {
        ($name:ident, $count:expr, $threads:expr) => {
            #[bench]
            fn $name(bencher: &mut Bencher) {
                const COUNT: usize = $count;
                const THREADS: usize = $threads;

                let mut vec = Vec::new();
                for x in 0..COUNT {
                    vec.push(x);
                }
                let mut rng = thread_rng();
                vec.shuffle(&mut rng);
                let vec = &vec;
                let _tree = RBTreeMap::new();
                let tree = &_tree;
                thread::scope(|scope| {
                    for idx in 0..8 {
                        scope.spawn(move |_| {
                            for elem in &vec[(idx * COUNT / 8)..((idx + 1) * COUNT / 8)] {
                                tree.insert(*elem, 0);
                            }
                        });
                    }
                })
                .unwrap();

                bencher.iter(move || {
                    thread::scope(|scope| {
                        for idx in 0..THREADS {
                            scope.spawn(move |_| {
                                for elem in
                                    &vec[(idx * COUNT / THREADS)..((idx + 1) * COUNT / THREADS)]
                                {
                                    assert!(tree.contains_key(elem));
                                }
                            });
                        }
                    })
                    .unwrap();
                });
                swym::print_stats();
            }
        };
    }
    contains_key_bench! {contains_key_01_100000, 100000, 1}
    contains_key_bench! {contains_key_02_100000, 100000, 2}
    contains_key_bench! {contains_key_03_100000, 100000, 3}
    contains_key_bench! {contains_key_04_100000, 100000, 4}
    contains_key_bench! {contains_key_05_100000, 100000, 5}
    contains_key_bench! {contains_key_06_100000, 100000, 6}
    contains_key_bench! {contains_key_07_100000, 100000, 7}
    contains_key_bench! {contains_key_08_100000, 100000, 8}
}
