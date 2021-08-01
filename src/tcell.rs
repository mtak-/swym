//! The core transactional memory primitive [`tcell::TCell`].
//!
//! # Examples
//!
//! Creating a `TCell` using [`new`]:
//!
//! ```
//! use swym::tcell::TCell;
//!
//! let x = TCell::new(String::from("abcdefghijklmnopqrstuvwxyz"));
//!
//! static X: TCell<usize> = TCell::new(42);
//! ```
//!
//! Extracting values out of a `TCell` using [`borrow`]:
//!
//! ```
//! use swym::{tcell::TCell, thread_key};
//!
//! let x = TCell::new(String::from("abcdefghijklmnopqrstuvwxyz"));
//! thread_key::get().rw(|tx| {
//!     let string = x.borrow(tx, Default::default())?;
//!     assert_eq!(&*string, "abcdefghijklmnopqrstuvwxyz");
//!     Ok(())
//! });
//! ```
//!
//! Modifying `TCell` using [`set`]:
//!
//! ```
//! use swym::{tcell::TCell, thread_key};
//!
//! let x = TCell::new(String::from("abcdefghijklmnopqrstuvwxyz"));
//! thread_key::get().rw(|tx| {
//!     x.set(tx, "hello".to_owned())?;
//!     Ok(())
//! });
//! assert_eq!(x.into_inner(), "hello");
//! ```
//!
//! [`new`]: struct.TCell.html#method.new
//! [`borrow`]: struct.TCell.html#method.borrow
//! [`set`]: struct.TCell.html#method.set

use crate::{
    internal::{tcell_erased::TCellErased, usize_aligned::UsizeAligned},
    tx::{Error, Ordering, Read, Rw, SetError, Write, _TValue},
};
use core::{
    cell::UnsafeCell,
    fmt::{self, Debug, Formatter},
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
    ptr,
    sync::atomic::{self, Ordering::Acquire},
};
use freeze::{AssertFreeze, Freeze};

/// A transactional memory location.
///
/// `TCell` stores an extra `usize` representing the current version of the memory.
///
/// The current value is stored directly in the `TCell` meaning it's not `Box`ed, `Arc`'ed, etc.
#[repr(C)]
pub struct TCell<T> {
    value:             UnsafeCell<UsizeAligned<T>>,
    pub(crate) erased: TCellErased,
}

unsafe impl<T: Send> Send for TCell<T> {}
unsafe impl<T: Send + Sync> Sync for TCell<T> {}

impl<T> Debug for TCell<T> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TCell")
            .field("erased", &self.erased)
            .field("value", &"...")
            .finish()
    }
}

impl<T: Default> Default for TCell<T> {
    #[inline]
    fn default() -> TCell<T> {
        TCell::new(Default::default())
    }
}

impl<T> From<T> for TCell<T> {
    #[inline]
    fn from(value: T) -> TCell<T> {
        TCell::new(value)
    }
}

impl<T> TCell<T> {
    /// Construct a new `TCell` from an initial value.
    ///
    /// This does not perform any memory allocation or synchronization.
    ///
    /// # Examples
    ///
    /// ```
    /// use swym::{tcell::TCell, thread_key};
    ///
    /// static ZERO: TCell<i32> = TCell::new(0);
    /// assert_eq!(
    ///     thread_key::get().read(|tx| Ok(ZERO.get(tx, Default::default())?)),
    ///     0
    /// );
    /// ```
    #[inline]
    pub const fn new(value: T) -> TCell<T> {
        TCell {
            value:  UnsafeCell::new(UsizeAligned::new(value)),
            erased: TCellErased::new(),
        }
    }

    /// Consumes this `TCell`, returning the underlying data.
    ///
    /// # Examples
    ///
    /// ```
    /// use swym::tcell::TCell;
    ///
    /// let x = TCell::new(42);
    /// assert_eq!(x.into_inner(), 42);
    /// ```
    #[inline]
    pub fn into_inner(self) -> T {
        self.value.into_inner().into_inner()
    }

    /// Returns a mutable reference to the underlying data.
    ///
    /// Since this call borrows the `TCell` mutably, no synchronization needs to take place. The
    /// mutable borrow statically guarantees no other threads are accessing this data.
    ///
    /// # Examples
    ///
    /// ```
    /// use swym::tcell::TCell;
    ///
    /// let mut x = TCell::new("hello");
    /// *x.borrow_mut() = "world";
    /// assert_eq!(*x.borrow_mut(), "world");
    /// ```
    #[inline]
    pub fn borrow_mut(&mut self) -> &mut T {
        // safe due to mutable borrow
        unsafe { &mut *self.value.get() }
    }

    #[inline]
    pub fn view<'tcell, Tx>(&'tcell self, transaction: Tx) -> View<'tcell, T, Tx>
    where
        Tx: Deref,
        Tx::Target: Read<'tcell> + Sized,
    {
        View {
            tx:    transaction,
            tcell: self,
        }
    }

    #[inline]
    pub(crate) unsafe fn optimistic_read_acquire(&self) -> ManuallyDrop<T> {
        let result = self.optimistic_read_relaxed();
        atomic::fence(Acquire);
        result
    }

    #[inline]
    pub(crate) unsafe fn optimistic_read_relaxed(&self) -> ManuallyDrop<T> {
        ptr::read_volatile(self.value.get() as _)
    }
}

impl<T: Freeze> TCell<T> {
    /// Gets a reference to the contained value using the specified memory [`Ordering`].
    ///
    /// Statically requires that the `TCell` outlives the current transaction.
    ///
    /// # Errors
    ///
    /// If another thread has written to this `TCell` during the current transaction, an error is
    /// returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use swym::{tcell::TCell, thread_key, tx::Ordering};
    ///
    /// let x = TCell::new("hello");
    /// let hello = thread_key::get().read(|tx| Ok(*x.borrow(tx, Ordering::Read)?));
    /// assert_eq!(hello, "hello");
    /// ```
    #[inline]
    pub fn borrow<'tx, 'tcell>(
        &'tcell self,
        tx: &'tx impl Read<'tcell>,
        ordering: Ordering,
    ) -> Result<Ref<'tx, T>, Error> {
        tx.borrow(self, ordering)
    }
}

impl<T: Copy> TCell<T> {
    /// Gets a copy of the contained value using the specified memory [`Ordering`].
    ///
    /// Statically requires that the `TCell` outlives the current transaction.
    ///
    /// # Errors
    ///
    /// If another thread has written to this `TCell` during the current transaction, an error is
    /// returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use swym::{tcell::TCell, thread_key, tx::Ordering};
    ///
    /// let x = TCell::new("hello");
    /// let hello = thread_key::get().read(|tx| Ok(x.get(tx, Ordering::Read)?));
    /// assert_eq!(hello, "hello");
    /// ```
    #[inline]
    #[must_use = "Calling `TCell::get` without using the result unnecessarily increases the chance \
                  of transaction failure"]
    pub fn get<'tcell>(
        &'tcell self,
        tx: &impl Read<'tcell>,
        ordering: Ordering,
    ) -> Result<T, Error> {
        // Calling borrow on a non Borrow type is Ok if all you do is Copy the value because this
        // cannot cause any internal mutation to happen.
        let this = unsafe { &*(self as *const Self as *const TCell<AssertFreeze<T>>) };
        this.borrow(tx, ordering).map(|v| **v)
    }
}

impl<T: 'static + Send> TCell<T> {
    #[inline]
    fn set_impl<'tcell>(
        &'tcell self,
        tx: &mut impl Write<'tcell>,
        value: impl _TValue<T>,
    ) -> Result<(), SetError<T>> {
        tx.set(self, value)
    }

    /// Sets the contained value.
    ///
    /// Statically requires that the `TCell` outlives the current transaction.
    ///
    /// # Errors
    ///
    /// If another thread has written to this `TCell` during the current transaction, the value is
    /// not set, and an error is returned. It is typical to route this error back to
    /// [`ThreadKey::rw`] where the transaction will be retried, however, this is not required.
    ///
    /// # Examples
    ///
    /// ```
    /// use swym::{tcell::TCell, thread_key};
    ///
    /// let x = TCell::new("hello");
    /// thread_key::get().rw(|tx| Ok(x.set(tx, "world")?));
    /// assert_eq!(x.into_inner(), "world");
    /// ```
    ///
    /// [`ThreadKey::rw`]: ../thread_key/struct.ThreadKey.html#method.rw
    #[inline]
    pub fn set<'tcell>(
        &'tcell self,
        tx: &mut impl Write<'tcell>,
        value: T,
    ) -> Result<(), SetError<T>> {
        self.set_impl(tx, value)
    }

    /// # Resource Publication
    ///
    /// Publication is the operation for sharing some resource - typically memory - acquired outside
    /// of the STM with other threads. Generally this is paired with a matching `Write::privatize`
    /// call in the inverse operation (e.g. in a linked list, push would publish a new heap
    /// allocated node, and pop would privatize that node, freeing its memory).
    ///
    /// # Performance
    ///
    /// `publish` has no overhead compared with `set`.
    #[inline]
    pub(crate) fn publish<'tcell, TV: _TValue<T>>(
        &'tcell self,
        tx: &mut impl Write<'tcell>,
        value: TV,
    ) -> Result<(), SetError<T>> {
        // it's important to bundle value and undo together early for panic safety
        self.set_impl(tx, value)
    }
}

impl<T: 'static + Freeze + Clone + Send> TCell<T> {
    pub fn replace<'tcell, 'tx>(
        &'tcell self,
        tx: &'tx mut impl Rw<'tcell>,
        value: T,
    ) -> Result<T, SetError<T>> {
        let prev = match self.borrow(tx, Ordering::Read) {
            Ok(prev) => prev.clone(),
            Err(error) => return Err(SetError { value, error }),
        };
        self.set(tx, value)?;
        Ok(prev)
    }
}

/// A snapshot of a [`TCell`] valid for the duration of the current transaction.
///
/// A `Ref` can be obtained using [`TCell::borrow`].
#[must_use = "Acquiring a Ref without using it unnecessarily increases the chance of transaction \
              failure"]
#[derive(Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ref<'tx, T> {
    snapshot: ManuallyDrop<T>,
    lifetime: PhantomData<&'tx T>,
}

impl<'tx, T: Debug> Debug for Ref<'tx, T> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        self.snapshot.deref().fmt(formatter)
    }
}

impl<'tx, T> Ref<'tx, T> {
    #[inline]
    pub fn new(snapshot: ManuallyDrop<T>) -> Self {
        Ref {
            snapshot,
            lifetime: PhantomData,
        }
    }

    #[inline]
    pub unsafe fn downcast<'tcell>(this: Self, _: &'tx impl Rw<'tcell>) -> Ref<'tcell, T> {
        Ref {
            snapshot: this.snapshot,
            lifetime: PhantomData,
        }
    }
}

impl<'tx, T: Freeze> From<&'tx T> for Ref<'tx, T> {
    #[inline]
    fn from(reference: &'tx T) -> Self {
        // lifetime + Freeze guarantees this is safe
        Ref::new(unsafe { ptr::read(reference as *const T as *const ManuallyDrop<T>) })
    }
}

impl<'tx, T: Freeze> From<&'tx mut T> for Ref<'tx, T> {
    #[inline]
    fn from(reference: &'tx mut T) -> Self {
        Ref::from(&*reference)
    }
}

impl<'tx, T> Deref for Ref<'tx, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &*self.snapshot
    }
}

impl<'tx, T> core::borrow::Borrow<T> for Ref<'tx, T> {
    #[inline]
    fn borrow(&self) -> &T {
        &*self.snapshot
    }
}

/// A view of a `TCell`'s memory.
#[derive(Debug)]
pub struct View<'tcell, T, Tx> {
    tx:    Tx,
    tcell: &'tcell TCell<T>,
}

impl<'tx, 'tcell, T: Freeze, Tx: Read<'tcell>> View<'tcell, T, &'tx Tx> {
    #[inline]
    pub fn into_borrow(self) -> Result<Ref<'tx, T>, Error>
    where
        Tx: 'tx,
    {
        self.tcell.borrow(self.tx, Ordering::default())
    }
}

impl<'tx, 'tcell, T: Freeze, Tx: Read<'tcell>> View<'tcell, T, &'tx mut Tx> {
    #[inline]
    pub fn into_borrow(self) -> Result<Ref<'tx, T>, Error>
    where
        Tx: 'tx,
    {
        self.tcell.borrow(self.tx, Ordering::default())
    }
}

impl<'tcell, T: Copy, Tx: Deref> View<'tcell, T, Tx>
where
    Tx::Target: Read<'tcell> + Sized,
{
    #[inline]
    pub fn get(&self) -> Result<T, Error> {
        self.get_ordered(Ordering::default())
    }

    #[inline]
    pub fn get_ordered(&self, ordering: Ordering) -> Result<T, Error> {
        self.tcell.get(&*self.tx, ordering)
    }
}

impl<'tcell, T: Freeze, Tx: Deref> View<'tcell, T, Tx>
where
    Tx::Target: Read<'tcell> + Sized,
{
    #[inline]
    pub fn borrow<'a>(&'a self) -> Result<Ref<'a, T>, Error> {
        self.borrow_ordered(Ordering::default())
    }

    #[inline]
    pub fn borrow_ordered<'a>(&'a self, ordering: Ordering) -> Result<Ref<'a, T>, Error> {
        self.tcell.borrow(&*self.tx, ordering)
    }
}

impl<'tcell, T: Send + 'static, Tx: DerefMut> View<'tcell, T, Tx>
where
    Tx::Target: Write<'tcell> + Sized,
{
    pub fn set(&mut self, value: T) -> Result<(), SetError<T>> {
        self.tcell.set(&mut *self.tx, value)
    }
}

#[cfg(test)]
mod test {
    use crate::{
        tcell::TCell,
        thread_key,
        tx::{Error, _TValue},
    };
    use core::{
        mem::ManuallyDrop,
        ptr,
        sync::atomic::{AtomicBool, Ordering},
    };
    use crossbeam_utils::thread;

    struct CustomUndo<T, F: FnOnce(T)> {
        value: ManuallyDrop<T>,
        undo:  ManuallyDrop<F>,
    }

    impl<T, F: FnOnce(T)> Drop for CustomUndo<T, F> {
        #[inline]
        fn drop(&mut self) {
            unsafe { (ptr::read(&*self.undo))(ptr::read(&*self.value)) }
        }
    }

    impl<T: 'static, F: FnOnce(T) + 'static> CustomUndo<T, F> {
        #[inline]
        fn new(value: T, undo: F) -> Self {
            CustomUndo {
                value: ManuallyDrop::new(value),
                undo:  ManuallyDrop::new(undo),
            }
        }
    }

    unsafe impl<T: 'static, F: FnOnce(T) + 'static> _TValue<T> for CustomUndo<T, F> {
        const REQUEST_TCELL_LIFETIME: bool = true;
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn publish_conflict() {
        static TRIGGERED: AtomicBool = AtomicBool::new(false);
        let x = TCell::new(42);

        thread::scope(|s| {
            s.spawn(|_| {
                let thread_key = thread_key::get();
                thread_key.rw(|tx| {
                    if TRIGGERED.load(Ordering::Relaxed) {
                        Ok(())
                    } else {
                        x.publish(
                            tx,
                            CustomUndo::new(1, |prev| {
                                assert_eq!(prev, 1);
                                TRIGGERED.store(true, Ordering::Relaxed);
                            }),
                        )?;
                        Err(Error::CONFLICT.into())
                    }
                });
            });
        })
        .unwrap();

        drop(x);
        assert!(
            TRIGGERED.load(Ordering::Relaxed),
            "failed to trigger custom undo"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn publish_3x() {
        static TRIGGERED: AtomicBool = AtomicBool::new(false);
        static TRIGGERED2: AtomicBool = AtomicBool::new(false);
        let x = TCell::new(42);

        thread::scope(|s| {
            s.spawn(|_| {
                let thread_key = thread_key::get();
                thread_key.rw(|tx| {
                    if TRIGGERED.load(Ordering::Relaxed) {
                        x.publish(
                            tx,
                            CustomUndo::new(4, |_| panic!("should not have been triggered")),
                        )?;
                        Ok(())
                    } else {
                        x.publish(
                            tx,
                            CustomUndo::new(1, |prev| {
                                assert_eq!(prev, 1);
                                TRIGGERED.store(true, Ordering::Relaxed);
                            }),
                        )?;
                        x.publish(
                            tx,
                            CustomUndo::new(2, |prev| {
                                TRIGGERED2.store(true, Ordering::Relaxed);
                                assert_eq!(prev, 2);
                            }),
                        )?;
                        x.set(tx, 3)?;
                        Err(Error::CONFLICT.into())
                    }
                });
            });
        })
        .unwrap();

        drop(x);
        assert!(
            TRIGGERED.load(Ordering::Relaxed),
            "failed to trigger custom undo"
        );
        assert!(
            TRIGGERED2.load(Ordering::Relaxed),
            "failed to trigger custom undo"
        );
    }
}
