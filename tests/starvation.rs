#![feature(test)]

mod starvation {
    use crossbeam_utils::thread;
    use swym::{tcell::TCell, thread_key};

    #[test]
    fn large_tx() {
        const CONTENDED_IDX: usize = 0;
        const TX_SIZE: usize = 50_000;
        let data = unsafe { &mut *Box::into_raw(Box::new(Vec::new())) };
        for _ in 0..TX_SIZE {
            data.push(TCell::new(0));
        }
        let data = &*data;

        // abort if test lasts too long (failure)
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(10));
            std::process::abort();
        });

        // thread that starves other threads
        std::thread::spawn(move || loop {
            thread_key::get().rw(|tx| {
                data[CONTENDED_IDX].set(tx, 0)?;
                Ok(())
            })
        });

        // rw
        thread::scope(|s| {
            // starving thread
            s.spawn(|_| {
                thread_key::get().rw(|tx| {
                    for i in 0..TX_SIZE {
                        data[i].set(tx, 0)?;
                    }
                    Ok(())
                })
            });
        })
        .unwrap();
        swym::stats::print_stats();

        // read
        thread::scope(|s| {
            // starving thread
            s.spawn(|_| {
                thread_key::get().read(|tx| {
                    // read the contended variable last
                    for i in (0..TX_SIZE).rev() {
                        drop(data[i].get(tx, Default::default())?);
                    }
                    Ok(())
                })
            });
        })
        .unwrap();
        swym::stats::print_stats();

        // check for gc deadlock
        let string = TCell::new("blah blah".to_owned());
        let other = TCell::new(0);
        thread::scope(|s| {
            s.spawn(|_| {
                thread_key::get().rw(|tx| {
                    // block gc
                    std::thread::sleep(std::time::Duration::from_millis(1_000));
                    // causes a park before commit
                    other.set(tx, 0)?;
                    Ok(())
                })
            });
            // starving thread
            s.spawn(|_| {
                // run enough times that gc happens
                for _ in 0..128 {
                    thread_key::get().rw(|tx| {
                        // doom the transaction
                        drop(data[CONTENDED_IDX].get(tx, Default::default())?);
                        std::thread::sleep(std::time::Duration::from_millis(1));

                        // create work for the gc
                        string.set(tx, "blah".to_owned())?;
                        Ok(())
                    })
                }
            });
        })
        .unwrap();
        swym::stats::print_stats();
    }
}
