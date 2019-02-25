#![feature(test)]

extern crate test;

use swym_rbtree::RBTreeMap;
use test::Bencher;

#[bench]
fn insert(b: &mut Bencher) {
    b.iter(|| {
        let map = RBTreeMap::new();

        let mut num = 0 as u64;
        for _ in 0..1_000 {
            num = num.wrapping_mul(17).wrapping_add(255);
            map.atomic(|mut map| {
                map.insert(num, !num)?;
                Ok(())
            });
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
                    map.atomic(|mut map| {
                        map.insert(num, !num)?;
                        Ok(())
                    })
                }

                let mut num = 0 as u64;
                for _ in 0..1_000 {
                    num = num.wrapping_mul(17).wrapping_add(255);
                    map.atomic(|mut map| {
                        map.remove(&num)?;
                        Ok(())
                    });
                }
            });
        });
    })
    .unwrap();
    swym::print_stats();
}
