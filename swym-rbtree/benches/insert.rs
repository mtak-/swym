#![feature(test)]

extern crate test;

mod insert {
    use swym_rbtree::RBTreeMap;
    use test::Bencher;

    #[bench]
    fn insert(b: &mut Bencher) {
        b.iter(|| {
            let map = RBTreeMap::new();

            let mut num = 0 as u64;
            for _ in 0..1_000 {
                num = num.wrapping_mul(17).wrapping_add(255);
                map.insert(num, !num);
            }
        });
    }

    #[bench]
    fn insert_remove(b: &mut Bencher) {
        crossbeam_utils::thread::scope(|s| {
            s.spawn(|_| {
                b.iter(|| {
                    let map = RBTreeMap::new();

                    let mut num = 0 as u64;
                    for _ in 0..1_000 {
                        num = num.wrapping_mul(17).wrapping_add(255);
                        map.insert(num, !num);
                    }

                    let mut num = 0 as u64;
                    for _ in 0..1_000 {
                        num = num.wrapping_mul(17).wrapping_add(255);
                        assert!(map.remove(&num).is_some());
                    }
                });
            });
        })
        .unwrap();
        swym::stats::print_stats();
    }
}
