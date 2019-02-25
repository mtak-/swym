mod reentrancy {
    use crossbeam_utils::thread;
    use std::cell::Cell;
    use swym::{tcell::TCell, thread_key};

    #[test]
    fn try_rw_on_drop() {
        const THREAD_COUNT: usize = 4;
        const ITER_COUNT: usize = 10_000;

        thread_local! {
            static THREAD_END: Cell<bool> = Cell::new(false);
        }

        struct TxOnDrop(String, [usize; 64]);
        impl Drop for TxOnDrop {
            fn drop(&mut self) {
                let x = TCell::new(([0usize; 128], "hello there".to_owned()));
                for _ in 0..128 {
                    let tx_result = thread_key::get().try_rw(|tx| {
                        x.set(tx, ([0; 128], "hello there".to_owned()))?;
                        Ok(())
                    });
                    assert!(tx_result.is_err() || THREAD_END.try_with(|b| b.get()).unwrap_or(true));
                }
            }
        }
        let x = TCell::new(TxOnDrop("hello there".to_owned(), [0; 64]));

        for thread_count in 0..(THREAD_COUNT + 1) {
            thread::scope(|scope| {
                for _ in 0..thread_count {
                    scope.spawn(|_| {
                        for _ in 0..ITER_COUNT {
                            thread_key::get()
                                .try_rw(|tx| {
                                    x.set(tx, TxOnDrop("hello there".to_owned(), [0; 64]))?;
                                    Ok(())
                                })
                                .unwrap()
                        }
                        THREAD_END.with(|b| b.set(true));
                    });
                }
            })
            .unwrap();
        }
        std::mem::forget(x);
    }
}
