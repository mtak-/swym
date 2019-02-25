#![feature(test)]

mod memory {
    use crossbeam_utils::thread;
    use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};
    use swym::{tcell::TCell, thread_key, tx::Ordering};

    #[test]
    fn _assert_single_threaded() {
        assert_eq!(
            std::env::var("RUST_TEST_THREADS")
                .expect("`RUST_TEST_THREADS` should be set to 1 for tests")
                .parse::<usize>()
                .expect("`RUST_TEST_THREADS` should be set to 1 for tests"),
            1,
            "`RUST_TEST_THREADS` should be set to 1 for tests"
        );
    }

    #[test]
    fn set_tcell_in_tptr() {
        const ITER_COUNT: usize = 1_000;
        const INNER_ITER_COUNT: usize = 10;
        const THREAD_COUNT: usize = 16;
        static ALLOC_COUNT: AtomicIsize = AtomicIsize::new(0);

        struct Foo(TCell<String>);
        impl Foo {
            fn new(x: &str) -> Self {
                ALLOC_COUNT.fetch_add(1, Relaxed);
                Foo(TCell::new(x.to_owned()))
            }
        }
        impl Drop for Foo {
            fn drop(&mut self) {
                ALLOC_COUNT.fetch_sub(1, Relaxed);
            }
        }

        use swym::tptr::TPtr;
        let x = TPtr::new(Box::into_raw(Box::new(Foo::new("hello there"))));
        thread::scope(|s| {
            for _ in 0..THREAD_COUNT {
                s.spawn(|_| {
                    let thread_key = thread_key::get();
                    for _ in 0..ITER_COUNT {
                        thread_key.rw(|tx| {
                            for _ in 0..INNER_ITER_COUNT {
                                let ptr = x.as_ptr(tx, Ordering::default())?;
                                if !ptr.is_null() {
                                    let ptr_ref = unsafe { &*ptr };
                                    drop(ptr_ref.0.borrow(tx, Ordering::default())?);
                                    ptr_ref.0.set(tx, "hello here".to_owned())?;
                                    unsafe { TPtr::privatize_as_box(tx, ptr) };
                                    x.set(tx, std::ptr::null_mut())?;
                                } else {
                                    x.publish_box(tx, Box::new(Foo::new("hello everywhere")))?
                                };
                            }
                            Ok(())
                        })
                    }
                });
            }
        })
        .unwrap();
        let ptr = x.into_inner();
        if !ptr.is_null() {
            unsafe { Box::from_raw(ptr as *mut Foo) };
        }
        assert_eq!(ALLOC_COUNT.load(Relaxed), 0);
    }
}
