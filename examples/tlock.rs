use crossbeam_utils::thread;
use lock_api::{GuardSend, RawMutex};
use swym::{tcell::TCell, thread_key, tx::Status};

/// A proof of concept lock built on top of STM. This is probably an anti-pattern, but educational
/// nonetheless.
struct RawTLock {
    held: TCell<bool>,
}

unsafe impl RawMutex for RawTLock {
    const INIT: Self = RawTLock {
        held: TCell::new(false),
    };
    type GuardMarker = GuardSend;

    /// Acquires this mutex, blocking the current thread until it is able to do so.
    fn lock(&self) {
        thread_key::get().rw(|tx| {
            if self.held.get(tx, Default::default())? {
                // Sleep, until the lock is released
                //
                // The `Ordering` - `Default::default` - used above puts `held` in our read set.
                // `swym` will watch for modifications to `held`, and wake this thread up when it is
                // next modified.
                return Err(Status::AWAIT_RETRY);
            } else {
                // The lock is not held, so grab it!
                self.held.set(tx, true)?;
            }
            Ok(())
        })
    }

    /// Attempts to acquire this mutex without blocking.
    fn try_lock(&self) -> bool {
        thread_key::get().rw(|tx| {
            Ok(if self.held.get(tx, Default::default())? {
                // The lock is held, return false
                false
            } else {
                // The lock is not held, grab it
                self.held.set(tx, true)?;
                true
            })
        })
    }

    /// Unlocks this mutex.
    fn unlock(&self) {
        thread_key::get().rw(|tx| Ok(self.held.set(tx, false)?))
    }
}

type TLock<T> = lock_api::Mutex<RawTLock, T>;

fn main() {
    let string = TLock::new("some string".to_owned());

    // If you definitely want a lock, `parking_lot`'s are faster.
    // let string = parking_lot::Mutex::new("some string".to_owned());

    // `std::sync::Mutex` tends on OSX tends to perform worse than `TLock`
    // let string = std::sync::Mutex::new("some string".to_owned());

    let string = &string;

    // Uncomment the line below to see deadlock (and 0% cpu usage showing all threads parked).
    // let _deadlock = string.lock();
    thread::scope(|s| {
        for i in 0..8 {
            s.spawn(move |_| {
                for _ in 0..1_000_000 {
                    let mut guard = string.lock();
                    *guard = format!("hello there from thread {}", i);
                }
            });
        }
    })
    .unwrap();

    println!("last thread's message:\n    '{}'", *string.lock())
}
