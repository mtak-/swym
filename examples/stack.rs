use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use swym::{
    tcell::{Ref, TCell},
    thread_key,
    tptr::TPtr,
    tx::{Borrow, Error, Ordering, Read, Rw},
};

#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

// used to verify the STM isn't leaking or double-freeing
static COUNT: AtomicUsize = AtomicUsize::new(0);

// nodes in our stack contain the current value, and a pointer to the next node
struct Node<T> {
    value: TCell<T>,
    next:  TPtr<Node<T>>,
}

struct TStack<T> {
    head: TPtr<Node<T>>,
}

impl<T> Drop for TStack<T> {
    fn drop(&mut self) {
        // destruction does not have to go through the STM since the &mut guarantees we are the sole
        // owner.
        let mut ptr = *self.head.borrow_mut() as *mut Node<T>;
        while !ptr.is_null() {
            let mut b = unsafe { Box::from_raw(ptr) };
            ptr = *b.next.borrow_mut() as *mut Node<T>;
        }
    }
}

impl<T> TStack<T> {
    const fn new() -> Self {
        // null means the stack is empty
        TStack { head: TPtr::null() }
    }
}

impl<T: 'static + Send + Sync + Borrow> TStack<T> {
    fn push<'tcell>(&'tcell self, tx: &mut impl Rw<'tcell>, value: T) -> Result<(), Error> {
        // the `next` pointer of our new node will be the current head pointer
        let next = self.head.as_ptr(tx, Ordering::Read)?;

        // allocate our new using `Box::new`
        let new_head = Box::new(Node {
            value: TCell::new(value),
            next:  TPtr::new(next),
        });

        // publish the new head pointer, essentially calling `Box::into_raw` on commit
        self.head.publish_box(tx, new_head)?;
        Ok(())
    }

    fn pop<'tcell, 'tx>(
        &'tcell self,
        tx: &'tx mut impl Rw<'tcell>,
    ) -> Result<Option<Ref<'tx, T>>, Error> {
        // get a pointer to the node we wish to pop
        let to_pop = self.head.as_ptr(tx, Ordering::default())?;

        // if it is null, then the stack is empty, so return None
        if to_pop.is_null() {
            return Ok(None);
        }

        // else, tell the STM that we want to deallocate the pointer sometime after the transaction
        let to_pop = unsafe {
            TPtr::privatize_as_box(tx, to_pop);

            // the pointer is still valid for the lifetime of the transaction.
            &*to_pop
        };

        // set head to to_pop's next pointer
        let new = to_pop.next.as_ptr(tx, Ordering::default())?;
        self.head.set(tx, new)?;

        // borrow the value we are popping, and return it
        to_pop.value.borrow(tx, Ordering::default()).map(Some)
    }

    fn iter<'tcell, 'tx, Tx: Read<'tcell>>(
        &'tcell self,
        tx: &'tx Tx,
    ) -> Result<Iter<'tx, T, Tx>, Error> {
        let cur = self.head.as_ptr(tx, Ordering::default())?;
        Ok(Iter { tx, cur })
    }
}

pub struct Iter<'tx, T, Tx: ?Sized> {
    tx:  &'tx Tx,
    cur: *const Node<T>,
}

impl<'tcell, 'tx, T: 'static + Borrow, Tx: Read<'tcell>> Iterator for Iter<'tx, T, Tx> {
    type Item = Result<Ref<'tx, T>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cur.is_null() {
            return None;
        }
        let cur_ref = unsafe { &*self.cur };
        let next = match cur_ref.next.as_ptr(self.tx, Ordering::default()) {
            Ok(next) => next,
            Err(e) => return Some(Err(e)),
        };
        self.cur = next;
        Some(cur_ref.value.borrow(self.tx, Ordering::default()))
    }
}

fn main() {
    struct Count(usize);
    impl Count {
        fn new(x: usize) -> Self {
            COUNT.fetch_add(1, Relaxed);
            Count(x)
        }
    }

    impl Drop for Count {
        fn drop(&mut self) {
            COUNT.fetch_sub(1, Relaxed);
        }
    }

    static LIST: TStack<Count> = TStack::new();
    const ITER_COUNT: usize = 2_000_000;

    let t1 = std::thread::spawn(|| {
        let thread_key = thread_key::get();
        let mut iters = 0;
        let mut total = 0;
        while iters < ITER_COUNT {
            let front = thread_key.rw(|tx| {
                let front = LIST.pop(tx)?;
                Ok(front.map(|x| x.0))
            });
            front.map(|x| {
                total += x;
                iters += 1;
            });
        }
        assert_eq!(total, (ITER_COUNT - 1) * ITER_COUNT / 2);
        println!("done t1");
    });
    let t0 = std::thread::spawn(|| {
        let thread_key = thread_key::get();
        for x in 0..ITER_COUNT {
            thread_key.rw(move |tx| {
                LIST.push(tx, Count::new(x))?;
                Ok(())
            });
        }
        println!("done t0");
    });
    t0.join().unwrap();
    t1.join().unwrap();
    let elems = thread_key::get().read(|tx| {
        Ok(LIST
            .iter(tx)?
            .map(|ok| ok.map(|ok| ok.0))
            .collect::<Result<Vec<_>, _>>()?)
    });
    assert!(elems.is_empty());
    drop(elems);
    assert_eq!(COUNT.load(Relaxed), 0);
}
