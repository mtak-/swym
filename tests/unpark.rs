mod unpark {
    use crossbeam_utils::thread;
    use swym::{tcell::TCell, thread_key, tx::Status};

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic]
    fn empty_tx() {
        let key = thread_key::get();
        let () = key.rw(|_| Err(Status::AWAIT_RETRY));
    }

    // This test attempts to create a situation where a thread can fail to park on `a` & `b`.
    // Specifically, parking validation fails on `b`. This causes the thread to try to clear the
    // unpark bit it might have just set on `a`. This is incorrect if there is another thread parked
    // on `a`.
    #[test]
    fn park_failure() {
        const ITER: usize = 1_000;
        const TRIES: usize = 50;

        // if we haven't completed in a reasonable amount of time, abort, failing the test
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(60));
            std::process::abort();
        });

        for _ in 0..TRIES {
            let a = TCell::new(0);
            let b = TCell::new(0);

            thread::scope(|s| {
                // this thread updates b
                s.spawn(|_| {
                    let key = thread_key::get();
                    for x in 0..=ITER {
                        key.rw(|tx| {
                            b.set(tx, x)?;
                            Ok(())
                        });
                    }
                });
                // this thread attempts to ensure a == b
                s.spawn(|_| {
                    let key = thread_key::get();
                    loop {
                        let finished = key.rw(|tx| {
                            let snap_a = a.get(tx, Default::default())?;
                            let snap_b = b.get(tx, Default::default())?;
                            if snap_a != snap_b {
                                a.set(tx, snap_b)?;
                            }
                            Ok(snap_a == ITER)
                        });
                        if finished {
                            break;
                        }
                    }
                });
                // competing parked threads
                for _ in 0..16 {
                    // this thread waits for a == b
                    s.spawn(|_| {
                        let key = thread_key::get();
                        loop {
                            let finished = key.rw(|tx| {
                                let snap_a = a.get(tx, Default::default())?;
                                let snap_b = b.get(tx, Default::default())?;
                                if snap_a != snap_b {
                                    Err(Status::AWAIT_RETRY)
                                } else {
                                    Ok(snap_a == ITER)
                                }
                            });
                            if finished {
                                break;
                            }
                        }
                    });
                }
            })
            .unwrap()
        }
        swym::stats::print_stats()
    }
}
