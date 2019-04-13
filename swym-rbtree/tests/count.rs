use crossbeam_utils::thread;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use swym_rbtree::RBTreeMap;

static COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Count(usize);
impl Clone for Count {
    fn clone(&self) -> Self {
        Count::new(self.0)
    }
}
impl Count {
    pub fn new(v: usize) -> Self {
        COUNT.fetch_add(1, Relaxed);
        Count(v)
    }
}

impl Drop for Count {
    fn drop(&mut self) {
        COUNT.fetch_sub(1, Relaxed);
    }
}

#[test]
fn count() {
    const ITER_COUNT: usize = 100_000;
    thread::scope(|scope| {
        scope.spawn(move |_| {
            let tree = RBTreeMap::new();
            for elem in 0..ITER_COUNT {
                tree.atomic(|mut tree| {
                    tree.entry(elem)?.or_insert(Count::new(0))?;
                    Ok(())
                })
            }
            swym::thread_key::get().read(|tx| {
                tree.raw.verify(tx)?;
                Ok(())
            });
        });
    })
    .unwrap();
    assert_eq!(COUNT.load(Relaxed), 0);
    swym::stats::print_stats();
}

#[test]
fn count_rev() {
    const ITER_COUNT: usize = 100_000;
    thread::scope(|scope| {
        scope.spawn(move |_| {
            let tree = RBTreeMap::new();
            for elem in (0..ITER_COUNT).rev() {
                tree.atomic(|mut tree| {
                    tree.entry(elem)?.or_insert(Count::new(0))?;
                    Ok(())
                })
            }
            swym::thread_key::get().read(|tx| {
                tree.raw.verify(tx)?;
                Ok(())
            });
        });
    })
    .unwrap();
    assert_eq!(COUNT.load(Relaxed), 0);
    swym::stats::print_stats();
}

#[test]
fn count_remove() {
    const ITER_COUNT: usize = 100_000;
    thread::scope(|scope| {
        scope.spawn(move |_| {
            let tree = RBTreeMap::new();
            for elem in 0..ITER_COUNT {
                tree.atomic(|mut tree| {
                    tree.insert(elem, Count::new(0))?;
                    Ok(())
                })
            }
            for elem in 0..ITER_COUNT {
                tree.remove(&elem).unwrap();
            }
            swym::thread_key::get().read(|tx| {
                tree.raw.verify(tx)?;
                Ok(())
            });
            std::mem::forget(tree);
        });
    })
    .unwrap();
    assert_eq!(COUNT.load(Relaxed), 0);
    swym::stats::print_stats();
}

#[test]
fn count_remove_rev() {
    const ITER_COUNT: usize = 100_000;
    thread::scope(|scope| {
        scope.spawn(move |_| {
            let tree = RBTreeMap::new();
            for elem in (0..ITER_COUNT).rev() {
                tree.atomic(|mut tree| {
                    tree.insert(elem, Count::new(0))?;
                    Ok(())
                })
            }
            for elem in (0..ITER_COUNT).rev() {
                tree.remove(&elem);
            }
            swym::thread_key::get().read(|tx| {
                tree.raw.verify(tx)?;
                Ok(())
            });
            std::mem::forget(tree);
        });
    })
    .unwrap();
    assert_eq!(COUNT.load(Relaxed), 0);
    swym::stats::print_stats();
}
