//! A low level transaction memory library.
//!
//! `swym` is an experimental STM that can be used to implement concurrent data structures with
//! performance not far from lock-free data structures.
//!
//! # Examples
//!
//! Getting a handle to swym's thread local state:
//! ```
//! use swym::thread_key;
//!
//! let thread_key = thread_key::get();
//! ```
//!
//! Creating new transactional memory cells:
//! ```
//! use swym::tcell::TCell;
//!
//! static A: TCell<i32> = TCell::new(0);
//! let b = TCell::new(42);
//! ```
//!
//! Performing a transaction to swap the two values:
//! ```
//! # use swym::{thread_key, tcell::TCell};
//! # let thread_key = thread_key::get();
//! # static A: TCell<i32> = TCell::new(0);
//! # let b = TCell::new(42);
//! thread_key.rw(|tx| {
//!     let temp = A.get(tx, Default::default())?;
//!     A.set(tx, b.get(tx, Default::default())?)?;
//!     b.set(tx, temp)?;
//!     Ok(())
//! });
//! assert_eq!(b.into_inner(), 0);
//! assert_eq!(thread_key.read(|tx| Ok(A.get(tx, Default::default())?)), 42);
//! ```
//!
//! # Features
//!
//! * Behaves as though a single global lock were held for the duration of every transaction. This
//!   can be relaxed a bit using `Ordering::Read` for loads.
//! * Highly optimized for read mostly data structures and modern caches. `TCell` stores all of its
//!   data inline. Read only transactions don't modify any global state, and read write transactions
//!   only modify global state on commit.
//! * Parking retry is supported via [`AWAIT_RETRY`](crate::tx::Status::AWAIT_RETRY).
//! * The number of allocations imposed by swym per transaction should average 0 through reuse of
//!   read logs/write logs/garbage bags.
//! * Support for building recursive data structures using `TPtr` is still experimental but looks
//!   promising - see examples on github.
//! * Backed by a custom epoch based reclaimation style garbage collector (still lots of
//!   optimization work to do there).
//! * Support for nested transactions is planned, but not yet implemented.
//!
//! ## Shared Memory
//!
//! * [`TCell`], a low level transactional memory location - does not perform any heap allocation.
//! * [`TPtr`], a low level transactional pointer for building heap allocated data structures.
//!
//! ## Running Transactions
//!
//! * [`rw`], starts a read write transaction.
//! * [`read`], starts a read only transaction.
//! * [`try_rw`], starts a read write transaction returning an error if the transaction could not be
//!   started.
//! * [`try_read`], starts a read only transaction returning an error if the transaction could not
//!   be started.
//!
//! [`TCell`]: tcell/struct.TCell.html
//! [`TPtr`]: tptr/struct.TPtr.html
//! [`rw`]: thread_key/struct.ThreadKey.html#method.rw
//! [`read`]: thread_key/struct.ThreadKey.html#method.read
//! [`try_rw`]: thread_key/struct.ThreadKey.html#method.try_rw
//! [`try_read`]: thread_key/struct.ThreadKey.html#method.try_read

#![feature(optin_builtin_traits)]
#![cfg_attr(feature = "nightly", feature(cfg_target_thread_local))]
#![cfg_attr(feature = "nightly", feature(core_intrinsics))]
#![cfg_attr(feature = "nightly", feature(thread_local))]
#![cfg_attr(all(test, feature = "nightly"), feature(raw))]
#![warn(macro_use_extern_crate)]
#![warn(missing_debug_implementations)]
// #![warn(missing_docs)]
#![warn(unused_lifetimes)]
#![cfg_attr(not(test), warn(unused_results))]
#![deny(intra_doc_link_resolution_failure)]
#![deny(rust_2018_compatibility)]
#![deny(rust_2018_idioms)]
#![deny(unused_must_use)]

#[macro_use]
mod internal;

mod read;
mod rw;
pub mod stats;
pub mod tcell;
pub mod thread_key;
pub mod tptr;
pub mod tx;

pub use read::ReadTx;
pub use rw::RwTx;
#[doc(inline)]
pub use swym_htm as htm;

#[cfg(test)]
mod memory {
    use crate::{tcell::TCell, thread_key, tx::Ordering};
    use crossbeam_utils::thread;

    #[test]
    fn leak_single() {
        const ITER_COUNT: usize = 100_000;
        let x = TCell::new(fvec![1, 2, 3, 4]);
        thread::scope(|s| {
            s.spawn(|_| {
                let thread_key = thread_key::get();
                for _ in 0..ITER_COUNT {
                    thread_key
                        .try_rw(|tx| {
                            x.set(tx, fvec![1, 2, 3, 4])?;
                            Ok(())
                        })
                        .unwrap();
                }
            })
            .join()
            .unwrap();
        })
        .unwrap();
        drop(x);
    }

    #[test]
    fn leak_multi() {
        const ITER_COUNT: usize = 10_000;
        const THREAD_COUNT: usize = 16;
        let x = TCell::new(fvec![1, 2, 3, 4]);
        thread::scope(|s| {
            for i in 0..THREAD_COUNT {
                s.builder()
                    .name(format!("scoped_thread#{}", i))
                    .spawn(|_| {
                        let thread_key = thread_key::get();
                        for _ in 0..ITER_COUNT {
                            thread_key
                                .try_rw(|tx| {
                                    x.set(tx, fvec![1, 2, 3, 4])?;
                                    Ok(())
                                })
                                .unwrap();
                        }
                    })
                    .unwrap();
            }
        })
        .unwrap();
        drop(x)
    }

    #[test]
    fn overaligned() {
        #[repr(align(1024))]
        struct Over(u8);
        impl Drop for Over {
            fn drop(&mut self) {
                assert_eq!(self as *mut _ as usize % 1024, 0);
            }
        }

        const ITER_COUNT: usize = 10_000;
        const THREAD_COUNT: usize = 16;
        let x = TCell::new(Over(0));
        thread::scope(|s| {
            for _ in 0..THREAD_COUNT {
                s.spawn(|_| {
                    let thread_key = thread_key::get();
                    for _ in 0..ITER_COUNT {
                        thread_key
                            .try_rw(|tx| {
                                let y = x.borrow(tx, Ordering::default())?.0;
                                x.set(tx, Over(y.wrapping_add(1)))?;
                                Ok(())
                            })
                            .unwrap();
                    }
                });
            }
        })
        .unwrap();
        drop(x);
    }

    #[test]
    fn zero_sized() {
        struct ZeroNoDrop;

        const ITER_COUNT: usize = 10_000;
        const THREAD_COUNT: usize = 16;
        let x = TCell::new(ZeroNoDrop);
        thread::scope(|s| {
            for _ in 0..THREAD_COUNT {
                s.spawn(|_| {
                    let thread_key = thread_key::get();
                    for _ in 0..ITER_COUNT {
                        thread_key
                            .try_rw(|tx| {
                                drop(x.borrow(tx, Ordering::default())?);
                                x.set(tx, ZeroNoDrop)?;
                                Ok(())
                            })
                            .unwrap();
                    }
                });
            }
        })
        .unwrap();
        drop(x);
    }

    #[test]
    fn zero_sized_drop() {
        struct Zero;
        impl Drop for Zero {
            fn drop(&mut self) {}
        }

        const ITER_COUNT: usize = 10_000;
        const THREAD_COUNT: usize = 16;
        let x = TCell::new(Zero);
        thread::scope(|s| {
            for _ in 0..THREAD_COUNT {
                s.spawn(|_| {
                    let thread_key = thread_key::get();
                    for _ in 0..ITER_COUNT {
                        thread_key
                            .try_rw(|tx| {
                                drop(x.borrow(tx, Ordering::default())?);
                                x.set(tx, Zero)?;
                                Ok(())
                            })
                            .unwrap();
                    }
                });
            }
        })
        .unwrap();
        drop(x);
    }
}

#[cfg(test)]
mod panic {
    use crate::{tcell::TCell, thread_key, tx::Ordering};
    use crossbeam_utils::thread;
    use std::panic::{self, AssertUnwindSafe};

    #[test]
    fn simple() {
        thread::scope(|s| {
            s.spawn(|_| {
                let thread_key = thread_key::get();
                match panic::catch_unwind(AssertUnwindSafe(|| {
                    thread_key
                        .try_rw(|_| -> Result<(), _> { panic!("test panic") })
                        .unwrap()
                })) {
                    Ok(_) => unreachable!(),
                    Err(_) => assert!(
                        thread_key.try_rw(|_| Ok(())).is_ok(),
                        "failed to recover from a panic within a tx"
                    ),
                }
            });
        })
        .unwrap();

        thread::scope(|s| {
            s.spawn(|_| {
                let thread_key = thread_key::get();
                match panic::catch_unwind(AssertUnwindSafe(|| {
                    thread_key
                        .try_read(|_| -> Result<(), _> { panic!("test panic") })
                        .unwrap()
                })) {
                    Ok(_) => unreachable!(),
                    Err(_) => assert!(
                        thread_key.try_rw(|_| Ok(())).is_ok(),
                        "failed to recover from a panic within a tx"
                    ),
                }
            });
        })
        .unwrap();

        thread::scope(|s| {
            s.spawn(|_| {
                let thread_key = thread_key::get();
                match panic::catch_unwind(AssertUnwindSafe(|| {
                    thread_key
                        .try_rw(|_| -> Result<(), _> { panic!("test panic") })
                        .unwrap()
                })) {
                    Ok(_) => unreachable!(),
                    Err(_) => assert!(
                        thread_key.try_read(|_| Ok(())).is_ok(),
                        "failed to recover from a panic within a tx"
                    ),
                }
            });
        })
        .unwrap();

        thread::scope(|s| {
            s.spawn(|_| {
                let thread_key = thread_key::get();
                match panic::catch_unwind(AssertUnwindSafe(|| {
                    thread_key
                        .try_read(|_| -> Result<(), _> { panic!("test panic") })
                        .unwrap()
                })) {
                    Ok(_) => unreachable!(),
                    Err(_) => assert!(
                        thread_key.try_read(|_| Ok(())).is_ok(),
                        "failed to recover from a panic within a tx"
                    ),
                }
            });
        })
        .unwrap();
    }

    #[test]
    fn write_log() {
        let tcell = TCell::new("hello".to_owned());
        thread::scope(|s| {
            s.spawn(|_| {
                let thread_key = thread_key::get();
                match panic::catch_unwind(AssertUnwindSafe(|| {
                    thread_key
                        .try_rw(|tx| -> Result<(), _> {
                            tcell.set(tx, "world".to_owned())?;
                            panic!("test panic")
                        })
                        .unwrap()
                })) {
                    Ok(_) => unreachable!(),
                    Err(_) => {
                        let old = thread_key.try_rw(|tx| {
                            let old = tcell.borrow(tx, Ordering::default())?.clone();
                            tcell.set(tx, "world".to_owned())?;
                            Ok(old)
                        });
                        assert!(old.is_ok(), "failed to recover from a panic within a tx");
                        assert!(
                            old.unwrap() == "hello",
                            "failed to recover from a panic within a tx"
                        );
                    }
                }
            });
        })
        .unwrap();
    }

    #[test]
    fn nest_fail() {
        thread::scope(|s| {
            s.spawn(|_| {
                let thread_key = thread_key::get();
                assert!(
                    thread_key
                        .try_rw(|_| {
                            assert!(
                                thread_key.try_rw(|_| Ok(())).is_err(),
                                "nesting unexpectedly did not cause an error"
                            );
                            Ok(())
                        })
                        .is_ok(),
                    "nesting prevented the root transaction from committing"
                );

                assert!(
                    thread_key
                        .try_rw(|_| {
                            assert!(
                                thread_key.try_read(|_| Ok(())).is_err(),
                                "nesting unexpectedly did not cause an error"
                            );
                            Ok(())
                        })
                        .is_ok(),
                    "nesting prevented the root transaction from committing"
                );

                assert!(
                    thread_key
                        .try_read(|_| {
                            assert!(
                                thread_key.try_read(|_| Ok(())).is_err(),
                                "nesting unexpectedly did not cause an error"
                            );
                            Ok(())
                        })
                        .is_ok(),
                    "nesting prevented the root transaction from committing"
                );

                assert!(
                    thread_key
                        .try_read(|_| {
                            assert!(
                                thread_key.try_rw(|_| Ok(())).is_err(),
                                "nesting unexpectedly did not cause an error"
                            );
                            Ok(())
                        })
                        .is_ok(),
                    "nesting prevented the root transaction from committing"
                );
            });
        })
        .unwrap();
    }
}
