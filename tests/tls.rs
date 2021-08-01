mod tls {
    use crossbeam_utils::thread;
    use swym::{tcell::TCell, thread_key, tx::Ordering};

    #[test]
    fn try_rw_while_exiting() {
        struct Foo;

        impl Drop for Foo {
            fn drop(&mut self) {
                let tcell = TCell::new("foobar longish string".to_owned());
                thread_key::get()
                    .try_rw(|tx| {
                        let s = "more ".to_owned() + &tcell.borrow(tx, Ordering::default())?;
                        tcell.set(tx, s)?;
                        Ok(())
                    })
                    .unwrap();
            }
        }

        thread_local! {
            static FOO: Foo = Foo;
        }

        thread::scope(|scope| {
            scope.spawn(|_| {
                FOO.with(|_| ());
                drop(swym::thread_key::get());
            });
        })
        .unwrap();
    }
}
