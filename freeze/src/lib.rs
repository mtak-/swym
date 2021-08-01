use core::{
    marker::PhantomData,
    mem::{Discriminant, ManuallyDrop, MaybeUninit},
    num::{
        NonZeroI128, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI8, NonZeroIsize, NonZeroU128,
        NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU8, NonZeroUsize,
    },
    ptr::NonNull,
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet, LinkedList, VecDeque},
    rc::Rc,
    sync::Arc,
};

pub use freeze_macros::Freeze;

/// Auto trait for types lacking direct interior mutability.
///
/// These types can have a snapshot (memcpy style) taken of the current state as long as the
/// original value is not dropped. See [`TCell::borrow`].
///
/// As long as the interior mutability resides on the heap (through a pointer), then the type can
/// manually implement `Borrow`.
pub unsafe trait Freeze {}

unsafe impl<T: ?Sized> Freeze for PhantomData<T> {}
unsafe impl<T: ?Sized> Freeze for *const T {}
unsafe impl<T: ?Sized> Freeze for *mut T {}
unsafe impl<T: ?Sized> Freeze for &T {}
unsafe impl<T: ?Sized> Freeze for &mut T {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct AssertFreeze<T: ?Sized> {
    value: T,
}

impl<T> AssertFreeze<T> {
    #[inline]
    pub const unsafe fn new(value: T) -> Self {
        Self { value }
    }

    #[inline]
    pub fn into_inner(this: Self) -> T {
        this.value
    }
}

unsafe impl<T: ?Sized> Freeze for AssertFreeze<T> {}

impl<T: ?Sized> core::borrow::Borrow<T> for AssertFreeze<T> {
    #[inline]
    fn borrow(&self) -> &T {
        &self.value
    }
}

impl<T: ?Sized> core::borrow::BorrowMut<T> for AssertFreeze<T> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

impl<T: ?Sized> core::ops::Deref for AssertFreeze<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T: ?Sized> core::ops::DerefMut for AssertFreeze<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

macro_rules! unsafe_freeze_basic {
    ($x:ty) => {
        unsafe impl Freeze for $x {}
    };
    ($($x:ty),* $(,)*) => {
        $(unsafe_freeze_basic!{$x})*
    };
}

macro_rules! freeze_tuple {
    () => {
        unsafe impl Freeze for () {}
    };
    ($x:ident) => {
        freeze_tuple!{}
        unsafe impl<$x: Freeze> Freeze for ($x,) {}
    };
    ($head:ident, $($tail:ident),* $(,)*) => {
        freeze_tuple!{$($tail),*}
        unsafe impl<$head: Freeze, $($tail: Freeze),*> Freeze for ($head, $($tail),*) {}
    };
}

macro_rules! freeze_fn {
    () => {
        unsafe impl<RR: ?Sized> Freeze for fn() -> RR {}
    };
    ($x:ident) => {
        freeze_fn!{}
        unsafe impl<$x: ?Sized, RR: ?Sized> Freeze for fn($x) -> RR {}
    };
    ($head:ident, $($tail:ident),* $(,)*) => {
        freeze_fn!{$($tail),*}
        unsafe impl<RR: ?Sized, $head: ?Sized, $($tail: ?Sized),*> Freeze for fn($head, $($tail),*) -> RR {}
    };
}

macro_rules! unsafe_freeze_1type_sized {
    ($x:ident) => {
        unsafe impl<T> Freeze for $x<T> {}
    };
    ($($x:ident),* $(,)*) => {
        $(unsafe_freeze_1type_sized!{$x})*
    };
}

macro_rules! unsafe_freeze_1type {
    ($x:ident) => {
        unsafe impl<T: ?Sized> Freeze for $x<T> {}
    };
    ($($x:ident),* $(,)*) => {
        $(unsafe_freeze_1type!{$x})*
    };
}

macro_rules! unsafe_freeze_2type_sized {
    ($x:ident) => {
        unsafe impl<T, U> Freeze for $x<T, U> {}
    };
    ($($x:ident),* $(,)*) => {
        $(unsafe_freeze_2type_sized!{$x})*
    };
}

//////////////////
// primitive
//////////////////

unsafe_freeze_basic! {
    bool,
    i8, i16, i32, i64, i128, isize,
    u8, u16, u32, u64, u128, usize,
    f32, f64,
    char,
    str,
}

unsafe impl<T: Freeze, const X: usize> Freeze for [T; X] {}
unsafe impl<T: Freeze> Freeze for [T] {}

freeze_tuple! {
    A,B,C,D,E,F,G,H,I,J,K,L,M,N,O,P,Q,R,S,T,U,V,W,X,Y,Z
}

freeze_fn! {
    A,B,C,D,E,F,G,H,I,J,K,L,M,N,O,P,Q,R,S,T,U,V,W,X,Y,Z
}

//////////////////
// NOT primitive
//////////////////

unsafe_freeze_basic! {
    String,
    NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI128, NonZeroIsize,
    NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU128, NonZeroUsize,
}

unsafe_freeze_1type_sized! {
    BinaryHeap,
    BTreeSet,
    Discriminant,
    HashSet,
    LinkedList,
    MaybeUninit,
    Vec,
    VecDeque,
}

unsafe_freeze_2type_sized! {
    BTreeMap,
    HashMap,
}

unsafe_freeze_1type! {
    Arc,
    Box,
    ManuallyDrop,
    NonNull,
    Rc,
}

unsafe impl<T: Freeze> Freeze for Option<T> {}
unsafe impl<T: Freeze, E: Freeze> Freeze for Result<T, E> {}
unsafe impl<'a, T: ?Sized + ToOwned> Freeze for Cow<'a, T> where <T as ToOwned>::Owned: Freeze {}
