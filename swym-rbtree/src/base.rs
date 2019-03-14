// based off of https://en.wikipedia.org/wiki/Red%E2%80%93black_tree and linux kernel
// excuse the mess

use std::ptr;
use swym::{
    tcell::TCell,
    tptr::TPtr,
    tx::{Error, Ordering, Read, Rw, Write},
};
use RBRef::{Null, Valid};

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Color {
    Black = 0,
    Red = 1,
}

struct PtrColor<K, V> {
    raw: *const RBNode<K, V>,
}

impl<K, V> Clone for PtrColor<K, V> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}
impl<K, V> Copy for PtrColor<K, V> {}

unsafe impl<K, V> Send for PtrColor<K, V> {}
unsafe impl<K, V> Sync for PtrColor<K, V> {}

impl<K, V> PtrColor<K, V> {
    #[inline]
    fn color(self) -> Color {
        if self.raw as usize & 1 != Color::Black as _ {
            Color::Red
        } else {
            Color::Black
        }
    }

    #[inline]
    fn ptr(self) -> *const RBNode<K, V> {
        (self.raw as usize & !1) as _
    }
}

struct Ptr<T>(*const T);
unsafe impl<T: Send + Sync> Send for Ptr<T> {}
unsafe impl<T: Send + Sync> Sync for Ptr<T> {}
impl<T> Clone for Ptr<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Ptr<T> {}

/// packed pointer/color
pub struct RBPtrColor<K, V> {
    raw: TCell<Ptr<RBNode<K, V>>>,
}

impl<K, V> RBPtrColor<K, V> {
    #[inline]
    pub const fn null_black() -> Self {
        RBPtrColor {
            raw: TCell::new(Ptr(Color::Black as usize as _)),
        }
    }

    #[inline]
    fn _as_ptr_color<'tcell>(
        &'tcell self,
        tx: &impl Read<'tcell>,
        ordering: Ordering,
    ) -> Result<PtrColor<K, V>, Error> {
        self.raw
            .get(tx, ordering)
            .map(|raw| PtrColor { raw: raw.0 })
    }

    #[inline]
    pub fn set_mut<'tcell>(&mut self, parent: &RBNode<K, V>, color: Color) {
        *self.raw.borrow_mut() =
            Ptr(((parent as *const _ as usize) | color as usize) as *const RBNode<K, V>);
    }

    #[inline]
    pub fn set_red_parent_mut<'tcell>(&mut self, parent: &RBNode<K, V>) {
        self.set_mut(parent, Color::Red)
    }

    #[inline]
    pub fn set_black_parent_mut<'tcell>(&mut self, parent: &RBNode<K, V>) {
        self.set_mut(parent, Color::Black)
    }

    #[inline]
    pub fn parent_mut<'tcell>(&mut self) -> RBRef<'tcell, K, V> {
        let raw = self.raw.borrow_mut().0 as *mut RBNode<K, V>;
        debug_assert!(raw as usize & 1 == Color::Red as _);
        let raw = (raw as usize ^ 1) as *mut RBNode<K, V>;
        if raw.is_null() {
            Null
        } else {
            Valid(unsafe { &mut *raw })
        }
    }

    #[inline]
    pub fn as_ref_color<'tcell>(
        &'tcell self,
        tx: &impl Read<'tcell>,
        ordering: Ordering,
    ) -> Result<(RBRef<'tcell, K, V>, Color), Error> {
        self._as_ptr_color(tx, ordering).map(|ptr_color| {
            let ptr = ptr_color.ptr();
            let color = ptr_color.color();
            (
                if ptr.is_null() {
                    Null
                } else {
                    Valid(unsafe { &*ptr })
                },
                color,
            )
        })
    }

    #[inline]
    pub fn color<'tcell>(
        &'tcell self,
        tx: &impl Read<'tcell>,
        ordering: Ordering,
    ) -> Result<Color, Error> {
        self._as_ptr_color(tx, ordering)
            .map(|ptr_color| ptr_color.color())
    }

    #[inline]
    pub fn black_parent<'tcell>(
        &'tcell self,
        tx: &impl Read<'tcell>,
        ordering: Ordering,
    ) -> Result<RBRef<'tcell, K, V>, Error> {
        self.raw.get(tx, ordering).map(|ptr| {
            let ptr = ptr.0;
            debug_assert!(ptr as usize & 1 == Color::Black as _);
            let ptr = (ptr as usize ^ Color::Black as usize) as *const RBNode<K, V>;
            if ptr.is_null() {
                Null
            } else {
                Valid(unsafe { &*ptr })
            }
        })
    }

    #[inline]
    pub fn red_parent<'tcell>(
        &'tcell self,
        tx: &impl Read<'tcell>,
        ordering: Ordering,
    ) -> Result<RBRef<'tcell, K, V>, Error> {
        self.raw.get(tx, ordering).map(|ptr| {
            let ptr = ptr.0;
            debug_assert!(ptr as usize & 1 == Color::Red as _);
            let ptr = (ptr as usize ^ Color::Red as usize) as *const RBNode<K, V>;
            if ptr.is_null() {
                Null
            } else {
                Valid(unsafe { &*ptr })
            }
        })
    }
}

impl<K: Send + Sync + 'static, V: Send + Sync + 'static> RBPtrColor<K, V> {
    #[inline]
    pub fn set<'tcell>(
        &'tcell self,
        tx: &mut impl Write<'tcell>,
        value: RBRef<'tcell, K, V>,
        color: Color,
    ) -> Result<(), Error> {
        Ok(self
            .raw
            .set(tx, Ptr((value._as_ptr() as usize | color as usize) as _))?)
    }
}

/// handy wrapper around TPtr
pub struct RBPtr<K, V> {
    pub raw: TPtr<RBNode<K, V>>,
}

impl<K, V> RBPtr<K, V> {
    #[inline]
    pub const fn null() -> Self {
        RBPtr { raw: TPtr::null() }
    }

    #[inline]
    pub fn as_mut_ref(&mut self) -> Option<&mut RBNode<K, V>> {
        let raw = *self.raw.borrow_mut() as *mut RBNode<K, V>;
        if raw.is_null() {
            None
        } else {
            Some(unsafe { &mut *raw })
        }
    }

    #[inline]
    pub fn as_ref<'tcell>(
        &'tcell self,
        tx: &impl Read<'tcell>,
        ordering: Ordering,
    ) -> Result<RBRef<'tcell, K, V>, Error> {
        self.raw.as_ptr(tx, ordering).map(|ptr| {
            if ptr.is_null() {
                Null
            } else {
                Valid(unsafe { &*ptr })
            }
        })
    }
}

impl<K: Send + Sync + 'static, V: Send + Sync + 'static> RBPtr<K, V> {
    #[inline]
    pub fn set<'tcell>(
        &'tcell self,
        tx: &mut impl Write<'tcell>,
        value: RBRef<'tcell, K, V>,
    ) -> Result<(), Error> {
        Ok(self.raw.set(tx, value._as_ptr())?)
    }

    #[inline]
    pub fn set_mut(&mut self, value: &RBNode<K, V>) {
        *self.raw.borrow_mut() = value;
    }

    #[inline]
    pub fn publish_box<'tcell>(
        &'tcell self,
        tx: &mut impl Write<'tcell>,
        value: Box<RBNode<K, V>>,
    ) -> Result<(), Error> {
        Ok(self.raw.publish_box(tx, value)?)
    }
}

pub enum RBRef<'tcell, K, V> {
    Valid(&'tcell RBNode<K, V>),
    Null,
}

impl<'tcell, K, V> Clone for RBRef<'tcell, K, V> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<'tcell, K, V> Copy for RBRef<'tcell, K, V> {}

impl<'tcell, K, V> RBRef<'tcell, K, V> {
    #[inline]
    fn _as_ptr(self) -> *const RBNode<K, V> {
        match self {
            Valid(x) => x as _,
            Null => ptr::null(),
        }
    }

    #[inline]
    unsafe fn unwrap(self) -> &'tcell RBNode<K, V> {
        match self {
            Valid(r) => r,
            Null => {
                debug_assert!(false, "unwrapping a null ref");
                std::intrinsics::unreachable();
            }
        }
    }
}

impl<'tcell, K, V> PartialEq for RBRef<'tcell, K, V> {
    #[inline]
    fn eq(&self, rhs: &Self) -> bool {
        ptr::eq(self._as_ptr(), rhs._as_ptr())
    }
}

impl<'tcell, K, V> Eq for RBRef<'tcell, K, V> {}

impl<'tcell, 'a, K, V> PartialEq<&'a RBNode<K, V>> for RBRef<'tcell, K, V> {
    #[inline]
    fn eq(&self, rhs: &&'a RBNode<K, V>) -> bool {
        ptr::eq(self._as_ptr(), *rhs)
    }
}

impl<'tcell, 'a, K, V> PartialEq<RBRef<'tcell, K, V>> for &'a RBNode<K, V> {
    #[inline]
    fn eq(&self, rhs: &RBRef<'tcell, K, V>) -> bool {
        ptr::eq(*self, rhs._as_ptr())
    }
}

#[repr(C)]
pub struct RBNode<K, V> {
    pub left:         RBPtr<K, V>,
    pub right:        RBPtr<K, V>,
    pub parent_color: RBPtrColor<K, V>,
    pub key:          K, // effectively immutable
    pub value:        V, // can be wrapped in TCell by user
}

impl<K, V> RBNode<K, V> {
    pub unsafe fn destroy(&mut self) {
        self.left.as_mut_ref().map(|left| left.destroy());
        self.right.as_mut_ref().map(|right| right.destroy());
        Box::from_raw(self);
    }
}

impl<K: Ord + Send + Sync + 'static, V: Send + Sync + 'static> RBNode<K, V> {
    #[inline(always)]
    fn find_impl<'tcell, Q>(
        &'tcell self,
        tx: &impl Read<'tcell>,
        key: &Q,
        ordering: Ordering,
    ) -> Result<Location<'tcell, K, V>, Error>
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let mut this = self;
        Ok(loop {
            match key.cmp(this.key.borrow()) {
                std::cmp::Ordering::Less => {
                    if let Valid(left) = this.left.as_ref(tx, ordering)? {
                        this = left
                    } else {
                        break Location::Vacant(VacantLocation::Left { parent: this });
                    }
                }
                std::cmp::Ordering::Greater => {
                    if let Valid(right) = this.right.as_ref(tx, ordering)? {
                        this = right
                    } else {
                        break Location::Vacant(VacantLocation::Right { parent: this });
                    }
                }
                std::cmp::Ordering::Equal => break Location::Occupied { node: this },
            }
        })
    }

    pub fn insert_fixup<'tcell>(
        mut self: Box<Self>,
        tx: &mut impl Rw<'tcell>,
        parent: &'tcell Self,
        insert_on_left: bool,
    ) -> Result<RepairResult<'tcell, Self>, Error> {
        // we inline the Valid(parent) case from below in order to optimize for the &mut case which
        // requires less tx ops.
        let (gp, p_color) = parent.parent_color.as_ref_color(tx, Ordering::default())?;
        let side = if insert_on_left {
            &parent.left
        } else {
            &parent.right
        };

        let (mut this, mut left, mut right) = if p_color == Color::Black {
            // we have a parent, and it's black, so there's no repair to do, just insert
            self.parent_color.set_red_parent_mut(parent);
            side.publish_box(tx, self)?;
            return Ok(RepairResult::RootUnchanged);
        } else {
            // safe because parent is red, and the root must be black
            let gp = unsafe { gp.unwrap() };
            let uncle_on_left = gp.key < parent.key;
            let uncle = if uncle_on_left {
                gp.left.as_ref(tx, Ordering::default())?
            } else {
                gp.right.as_ref(tx, Ordering::default())?
            };
            match uncle {
                Valid(uncle)
                    if uncle.parent_color.color(tx, Ordering::default())? == Color::Red =>
                {
                    // insert then begin recursion
                    self.parent_color.set_red_parent_mut(parent);
                    side.publish_box(tx, self)?;
                    // toggle parent and uncle to black, and gp to red.
                    // now start the process over again for gp
                    let (left, right) = if uncle_on_left {
                        (uncle, parent)
                    } else {
                        (parent, uncle)
                    };
                    // we fix up node colors in the "next" iteration.
                    // this allows for some optimization (less writes).
                    (gp, left, right)
                }
                _ => {
                    return self.insert_case4_mut(tx, parent, gp, uncle_on_left, insert_on_left);
                }
            }
        };

        loop {
            let parent_ref = this.parent_color.black_parent(tx, Ordering::Read)?;
            match parent_ref {
                Null => {
                    // no parent, so this node must be root which is always black
                    debug_assert!(this.parent_color.color(tx, Ordering::Read)? == Color::Black);
                    left.parent_color.set(tx, Valid(this), Color::Black)?;
                    right.parent_color.set(tx, Valid(this), Color::Black)?;
                    return Ok(RepairResult::RootUnchanged);
                }
                Valid(parent) => {
                    let (gp, p_color) =
                        parent.parent_color.as_ref_color(tx, Ordering::default())?;
                    // we have a parent, and it's black, so there's no more work to do
                    if p_color == Color::Black {
                        left.parent_color.set(tx, Valid(this), Color::Black)?;
                        right.parent_color.set(tx, Valid(this), Color::Black)?;
                        this.parent_color.set(tx, Valid(parent), Color::Red)?;
                        return Ok(RepairResult::RootUnchanged);
                    } else {
                        // safe because parent is red, and the root must be black
                        let gp = unsafe { gp.unwrap() };
                        let uncle_on_left = gp.key < parent.key;
                        let uncle = if uncle_on_left {
                            gp.left.as_ref(tx, Ordering::default())?
                        } else {
                            gp.right.as_ref(tx, Ordering::default())?
                        };
                        match uncle {
                            Valid(uncle)
                                if uncle.parent_color.color(tx, Ordering::default())?
                                    == Color::Red =>
                            {
                                left.parent_color.set(tx, Valid(this), Color::Black)?;
                                right.parent_color.set(tx, Valid(this), Color::Black)?;
                                this.parent_color.set(tx, Valid(parent), Color::Red)?;
                                // toggle parent and uncle to black, and gp to red.
                                // now start the process over again for gp
                                let (_left, _right) = if uncle_on_left {
                                    (uncle, parent)
                                } else {
                                    (parent, uncle)
                                };
                                left = _left;
                                right = _right;
                                this = gp;
                            }
                            _ => {
                                return Ok(
                                    match this.insert_case4(
                                        tx,
                                        parent,
                                        gp,
                                        uncle_on_left,
                                        left,
                                        right,
                                    )? {
                                        Some(root) => RepairResult::NewRoot(root),
                                        None => RepairResult::RootUnchanged,
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    fn partial_rotate_right_red_helper<'tcell>(
        &'tcell self,
        tx: &mut impl Rw<'tcell>,
        left: &'tcell Self,
        left_right: &'tcell Self,
    ) -> Result<(), Error> {
        self.left.set(tx, Valid(left_right))?;
        self.parent_color.set(tx, Valid(left), Color::Red)?;
        left.right.set(tx, Valid(self))?;
        left_right.parent_color.set(tx, Valid(self), Color::Black)?;
        Ok(())
    }

    fn partial_rotate_left_red_helper<'tcell>(
        &'tcell self,
        tx: &mut impl Rw<'tcell>,
        right: &'tcell Self,
        right_left: &'tcell Self,
    ) -> Result<(), Error> {
        self.right.set(tx, Valid(right_left))?;
        self.parent_color.set(tx, Valid(right), Color::Red)?;
        right.left.set(tx, Valid(self))?;
        right_left.parent_color.set(tx, Valid(self), Color::Black)?;
        Ok(())
    }

    fn rotate_left_red_helper<'tcell>(
        &'tcell self,
        tx: &mut impl Rw<'tcell>,
        right: &'tcell Self,
        right_left: RBRef<'tcell, K, V>,
    ) -> Result<Option<&'tcell Self>, Error> {
        // read then immediate unconditional write is ReadWrite
        let parent = self.parent_color.black_parent(tx, Ordering::Read)?;
        self.parent_color.set(tx, Valid(right), Color::Red)?;

        self.right.set(tx, right_left)?;
        right.left.set(tx, Valid(self))?;
        right.parent_color.set(tx, parent, Color::Black)?;
        if let Valid(right_left) = right_left {
            right_left.parent_color.set(tx, Valid(self), Color::Black)?;
        }
        Ok(if let Valid(parent) = parent {
            if self.key < parent.key {
                parent.left.set(tx, Valid(right))?
            } else {
                parent.right.set(tx, Valid(right))?
            }
            None
        } else {
            Some(right)
        })
    }

    fn rotate_right_red_helper<'tcell>(
        &'tcell self,
        tx: &mut impl Rw<'tcell>,
        left: &'tcell Self,
        left_right: RBRef<'tcell, K, V>,
    ) -> Result<Option<&'tcell Self>, Error> {
        // read then immediate unconditional write is ReadWrite
        let parent = self.parent_color.black_parent(tx, Ordering::Read)?;
        self.parent_color.set(tx, Valid(left), Color::Red)?;

        self.left.set(tx, left_right)?;
        left.right.set(tx, Valid(self))?;
        left.parent_color.set(tx, parent, Color::Black)?;
        if let Valid(left_right) = left_right {
            left_right.parent_color.set(tx, Valid(self), Color::Black)?;
        }
        Ok(if let Valid(parent) = parent {
            if self.key < parent.key {
                parent.left.set(tx, Valid(left))?
            } else {
                parent.right.set(tx, Valid(left))?
            }
            None
        } else {
            Some(left)
        })
    }

    fn insert_case4<'tcell>(
        &'tcell self,
        tx: &mut impl Rw<'tcell>,
        parent: &'tcell Self,
        gp: &'tcell Self,
        uncle_on_left: bool,
        left: &'tcell Self,
        right: &'tcell Self,
    ) -> Result<Option<&'tcell Self>, Error> {
        if uncle_on_left {
            // Read Ok, written in both branches
            let parent_left = parent.left.as_ref(tx, Ordering::Read)?;
            let (right, right_left) = if parent_left == self {
                parent.partial_rotate_right_red_helper(tx, self, right)?;
                (self, Valid(left))
            } else {
                left.parent_color.set(tx, Valid(self), Color::Black)?;
                right.parent_color.set(tx, Valid(self), Color::Black)?;
                self.parent_color.set(tx, Valid(parent), Color::Red)?;
                (parent, parent_left)
            };
            gp.rotate_left_red_helper(tx, right, right_left)
        } else {
            // Read Ok, written in both branches
            let parent_right = parent.right.as_ref(tx, Ordering::Read)?;
            let (left, left_right) = if parent_right == self {
                parent.partial_rotate_left_red_helper(tx, self, left)?;
                (self, Valid(right))
            } else {
                left.parent_color.set(tx, Valid(self), Color::Black)?;
                right.parent_color.set(tx, Valid(self), Color::Black)?;
                self.parent_color.set(tx, Valid(parent), Color::Red)?;
                (parent, parent_right)
            };
            gp.rotate_right_red_helper(tx, left, left_right)
        }
    }

    fn insert_case4_mut<'tcell>(
        mut self: Box<Self>,
        tx: &mut impl Rw<'tcell>,
        parent: &'tcell Self,
        gp: &'tcell Self,
        uncle_on_left: bool,
        insert_on_left: bool,
    ) -> Result<RepairResult<'tcell, Self>, Error> {
        let self_ref = unsafe { &*(&*self as *const Self) };
        Ok(if uncle_on_left {
            if insert_on_left {
                // parent.rotate_right(tx)?;
                // gp.rotate_left(tx)?;

                self.right.set_mut(parent);
                self.left.set_mut(gp);
                parent.parent_color.set(tx, Valid(self_ref), Color::Red)?;

                // read followed immediately by write, ReadWrite
                let parent = gp.parent_color.black_parent(tx, Ordering::Read)?;
                gp.parent_color.set(tx, Valid(self_ref), Color::Red)?;

                gp.right.set(tx, Null)?;
                if let Valid(parent) = parent {
                    self.parent_color.set_black_parent_mut(parent);
                    if gp.key < parent.key {
                        parent.left.publish_box(tx, self)?
                    } else {
                        parent.right.publish_box(tx, self)?
                    }
                    RepairResult::RootUnchanged
                } else {
                    RepairResult::PublishRoot(self)
                }
            } else {
                self.parent_color.set_red_parent_mut(parent);
                parent.right.publish_box(tx, self)?;

                let parent_left = parent.left.as_ref(tx, Ordering::Read)?;
                match gp.rotate_left_red_helper(tx, parent, parent_left)? {
                    Some(root) => RepairResult::NewRoot(root),
                    None => RepairResult::RootUnchanged,
                }
            }
        } else {
            if !insert_on_left {
                // parent.rotate_left(tx)?;
                // gp.rotate_right(tx)?;

                self.left.set_mut(parent);
                self.right.set_mut(gp);
                parent.parent_color.set(tx, Valid(self_ref), Color::Red)?;

                // read followed immediately by write, ReadWrite
                let parent = gp.parent_color.black_parent(tx, Ordering::Read)?;
                gp.parent_color.set(tx, Valid(self_ref), Color::Red)?;

                gp.left.set(tx, Null)?;
                if let Valid(parent) = parent {
                    self.parent_color.set_black_parent_mut(parent);
                    if gp.key < parent.key {
                        parent.left.publish_box(tx, self)?
                    } else {
                        parent.right.publish_box(tx, self)?
                    }
                    RepairResult::RootUnchanged
                } else {
                    RepairResult::PublishRoot(self)
                }
            } else {
                self.parent_color.set_red_parent_mut(parent);
                parent.left.publish_box(tx, self)?;

                // gp.rotate_right(tx)?;
                let parent_left = parent.right.as_ref(tx, Ordering::Read)?;
                match gp.rotate_right_red_helper(tx, parent, parent_left)? {
                    Some(root) => RepairResult::NewRoot(root),
                    None => RepairResult::RootUnchanged,
                }
            }
        })
    }

    pub fn remove_nofix<'tcell>(
        &'tcell self,
        tx: &mut impl Rw<'tcell>,
    ) -> Result<(Option<RBRef<'tcell, K, V>>, RBRef<'tcell, K, V>), Error> {
        let mut new_root = None;
        let mut rebalance = Null;
        let node_right = self.right.as_ref(tx, Ordering::default())?;
        let node_left = self.left.as_ref(tx, Ordering::default())?;
        if node_left == Null {
            let (parent, color) = self.parent_color.as_ref_color(tx, Ordering::default())?;
            if let Valid(parent) = parent {
                if self.key < parent.key {
                    &parent.left
                } else {
                    &parent.right
                }
                .set(tx, node_right)?;
            } else {
                new_root = Some(node_right);
            }
            if let Valid(node_right) = node_right {
                node_right.parent_color.set(tx, parent, color)?;
            } else if color == Color::Black {
                rebalance = parent;
            }
        } else if node_right == Null {
            let node_left = unsafe { node_left.unwrap() };
            let (parent, color) = self.parent_color.as_ref_color(tx, Ordering::default())?;
            node_left.parent_color.set(tx, parent, color)?;
            if let Valid(parent) = parent {
                if self.key < parent.key {
                    &parent.left
                } else {
                    &parent.right
                }
                .set(tx, Valid(node_left))?;
            } else {
                new_root = Some(Valid(node_left));
            }
        } else {
            let mut parent;
            let child2;
            let child = unsafe { node_right.unwrap() };
            let mut successor = child;
            let mut node_right_left = child.left.as_ref(tx, Ordering::default())?;
            if Null == node_right_left {
                parent = successor;
                child2 = successor.right.as_ref(tx, Ordering::default())?;
            } else {
                loop {
                    let right_leftmost = unsafe { node_right_left.unwrap() };
                    parent = successor;
                    successor = right_leftmost;
                    node_right_left = right_leftmost.left.as_ref(tx, Ordering::default())?;
                    if Null == node_right_left {
                        break;
                    }
                }
                child2 = successor.right.as_ref(tx, Ordering::default())?;
                parent.left.set(tx, child2)?;
                successor.right.set(tx, Valid(child))?;
                let child_color = child.parent_color.color(tx, Ordering::Read)?;
                child.parent_color.set(tx, Valid(successor), child_color)?;
            }

            // TODO: remove this?
            let node_left = self.left.as_ref(tx, Ordering::default())?;
            successor.left.set(tx, node_left)?;
            let node_left = unsafe { node_left.unwrap() };
            let color = node_left.parent_color.color(tx, Ordering::Read)?;
            node_left.parent_color.set(tx, Valid(successor), color)?;

            let pc = self.parent_color.as_ref_color(tx, Ordering::default())?;
            if let Valid(p) = pc.0 {
                if self.key < p.key { &p.left } else { &p.right }.set(tx, Valid(successor))?;
            } else {
                new_root = Some(Valid(successor));
            }

            if let Valid(child2) = child2 {
                successor.parent_color.set(tx, pc.0, pc.1)?;
                child2.parent_color.set(tx, Valid(parent), Color::Black)?;
            } else {
                let pc2 = successor.parent_color.as_ref_color(tx, Ordering::Read)?;
                successor.parent_color.set(tx, pc.0, pc.1)?;
                if pc2.1 == Color::Black {
                    rebalance = Valid(parent);
                }
            }
        }
        unsafe { TPtr::privatize_as_box(tx, self) };
        Ok((new_root, rebalance))
    }

    // algorithm from here:
    // https://github.com/torvalds/linux/blob/master/lib/rbtree.c
    #[inline(never)]
    pub fn remove_fixup<'tcell>(
        &'tcell self,
        // parent
        tx: &mut impl Rw<'tcell>,
    ) -> Result<Option<&'tcell Self>, Error> {
        let mut result = None;
        let mut node: RBRef<'_, K, V> = Null;
        let mut parent = self;
        loop {
            debug_assert!(
                node == Null || {
                    let (p, c) = unsafe { node.unwrap() }
                        .parent_color
                        .as_ref_color(tx, Ordering::Read)?;
                    c == Color::Black && p != Null
                }
            );
            /*
             * Loop invariants:
             * - node is black (or NULL on first iteration)
             * - node is not the root (parent is not NULL)
             * - All leaf paths going through parent and node have a black node count that is 1
             *   lower than other leaf paths.
             */
            let sibling = parent.right.as_ref(tx, Ordering::default())?;
            if sibling != node {
                /* node == parent.left */
                let mut sibling = unsafe { sibling.unwrap() };
                let sibling_color = sibling.parent_color.color(tx, Ordering::default())?;
                if sibling_color == Color::Red {
                    // parent.rotate_left
                    let sibling_left = sibling.left.as_ref(tx, Ordering::Read)?;
                    let sibling_left = unsafe { sibling_left.unwrap() };
                    let new_root =
                        parent.rotate_left_red_helper(tx, sibling, Valid(sibling_left))?;
                    result = result.or(new_root);
                    sibling = sibling_left;
                }

                let mut sibling_right = sibling.right.as_ref(tx, Ordering::default())?;
                let sibling_right_black = if let Valid(sibling_right) = sibling_right {
                    sibling_right.parent_color.color(tx, Ordering::default())? == Color::Black
                } else {
                    true
                };
                if sibling_right_black {
                    let sibling_left = sibling.left.as_ref(tx, Ordering::default())?;
                    let sibling_left_black = if let Valid(sibling_left) = sibling_left {
                        sibling_left.parent_color.color(tx, Ordering::default())? == Color::Black
                    } else {
                        true
                    };
                    if sibling_left_black {
                        sibling.parent_color.set(tx, Valid(parent), Color::Red)?;
                        let (gp, p_color) =
                            parent.parent_color.as_ref_color(tx, Ordering::default())?;
                        if p_color == Color::Red {
                            parent.parent_color.set(tx, gp, Color::Black)?;
                        } else {
                            if let Valid(gp) = gp {
                                node = Valid(parent);
                                parent = gp;
                                continue;
                            }
                        }
                        break;
                    }
                    // sibling.rotate_right
                    let sibling_left = unsafe { sibling_left.unwrap() };
                    let sib_left_right = sibling_left.right.as_ref(tx, Ordering::default())?;
                    sibling.left.set(tx, sib_left_right)?;
                    sibling_left.right.set(tx, Valid(sibling))?;
                    parent.right.set(tx, Valid(sibling_left))?;
                    if let Valid(sib_left_right) = sib_left_right {
                        sib_left_right
                            .parent_color
                            .set(tx, Valid(sibling), Color::Black)?;
                    }
                    sibling_right = Valid(sibling);
                    sibling = sibling_left;
                }

                let sibling_right = unsafe { sibling_right.unwrap() };

                // parent.rotate_left
                let sibling_left = sibling.left.as_ref(tx, Ordering::Read)?;
                sibling.left.set(tx, Valid(parent))?;

                let (gp, p_color) = parent.parent_color.as_ref_color(tx, Ordering::Read)?;
                parent.parent_color.set(tx, Valid(sibling), Color::Black)?;

                parent.right.set(tx, sibling_left)?;
                if let Valid(sibling_left) = sibling_left {
                    let color = sibling_left.parent_color.color(tx, Ordering::Read)?;
                    sibling_left.parent_color.set(tx, Valid(parent), color)?;
                }

                sibling_right
                    .parent_color
                    .set(tx, Valid(sibling), Color::Black)?;

                sibling.parent_color.set(tx, gp, p_color)?;
                if let Valid(gp) = gp {
                    if sibling.key < gp.key {
                        &gp.left
                    } else {
                        &gp.right
                    }
                    .set(tx, Valid(sibling))?
                } else {
                    result = Some(sibling)
                }
                break;
            } else {
                let sibling = parent.left.as_ref(tx, Ordering::default())?;
                let mut sibling = unsafe { sibling.unwrap() };
                let sibling_color = sibling.parent_color.color(tx, Ordering::default())?;
                if sibling_color == Color::Red {
                    // parent.rotate_right
                    let sibling_right = sibling.right.as_ref(tx, Ordering::Read)?;
                    let sibling_right = unsafe { sibling_right.unwrap() };
                    let new_root =
                        parent.rotate_right_red_helper(tx, sibling, Valid(sibling_right))?;
                    result = result.or(new_root);
                    sibling = sibling_right;
                }

                let mut sibling_left = sibling.left.as_ref(tx, Ordering::default())?;
                let sibling_left_black = if let Valid(sibling_left) = sibling_left {
                    sibling_left.parent_color.color(tx, Ordering::default())? == Color::Black
                } else {
                    true
                };
                if sibling_left_black {
                    let sibling_right = sibling.right.as_ref(tx, Ordering::default())?;
                    let sibling_right_black = if let Valid(sibling_right) = sibling_right {
                        sibling_right.parent_color.color(tx, Ordering::default())? == Color::Black
                    } else {
                        true
                    };
                    if sibling_right_black {
                        sibling.parent_color.set(tx, Valid(parent), Color::Red)?;
                        let (gp, p_color) =
                            parent.parent_color.as_ref_color(tx, Ordering::default())?;
                        if p_color == Color::Red {
                            parent.parent_color.set(tx, gp, Color::Black)?;
                        } else {
                            if let Valid(gp) = gp {
                                node = Valid(parent);
                                parent = gp;
                                continue;
                            }
                        }
                        break;
                    }
                    // sibling.rotate_left
                    let sibling_right = unsafe { sibling_right.unwrap() };
                    let sib_right_left = sibling_right.left.as_ref(tx, Ordering::Read)?;
                    sibling_right.left.set(tx, Valid(sibling))?;
                    sibling.right.set(tx, sib_right_left)?;
                    parent.left.set(tx, Valid(sibling_right))?;
                    if let Valid(sib_right_left) = sib_right_left {
                        sib_right_left
                            .parent_color
                            .set(tx, Valid(sibling), Color::Black)?;
                    }
                    sibling_left = Valid(sibling);
                    sibling = sibling_right;
                }

                let sibling_left = unsafe { sibling_left.unwrap() };

                // parent.rotate_left
                let sibling_right = sibling.right.as_ref(tx, Ordering::Read)?;
                sibling.right.set(tx, Valid(parent))?;
                parent.left.set(tx, sibling_right)?;
                sibling_left
                    .parent_color
                    .set(tx, Valid(sibling), Color::Black)?;

                if let Valid(sibling_right) = sibling_right {
                    let color = sibling_right.parent_color.color(tx, Ordering::Read)?;
                    sibling_right.parent_color.set(tx, Valid(parent), color)?;
                }

                let (gp, p_color) = parent.parent_color.as_ref_color(tx, Ordering::Read)?;
                parent.parent_color.set(tx, Valid(sibling), Color::Black)?;
                sibling.parent_color.set(tx, gp, p_color)?;
                if let Valid(gp) = gp {
                    if sibling.key < gp.key {
                        &gp.left
                    } else {
                        &gp.right
                    }
                    .set(tx, Valid(sibling))?
                } else {
                    result = Some(sibling)
                }
                break;
            }
        }
        Ok(result)
    }

    // checks all the RBTree properties (for debugging)
    pub fn verify<'tcell>(&'tcell self, tx: &impl Read<'tcell>) -> Result<Verify<K>, Error>
    where
        K: Clone,
    {
        let (_, color) = self.parent_color.as_ref_color(tx, Ordering::default())?;
        if color == Color::Red {
            let left = self.left.as_ref(tx, Ordering::default())?;
            let left_verify = if let Valid(left) = left {
                let (lp, lc) = left.parent_color.as_ref_color(tx, Ordering::default())?;
                assert!(lc == Color::Black);
                assert!(ptr::eq(unsafe { lp.unwrap() }, self));
                left.verify(tx)?
            } else {
                Verify {
                    black_depth: 1,
                    min_depth:   1,
                    max_depth:   1,
                    min:         None,
                    max:         None,
                }
            };
            let right = self.right.as_ref(tx, Ordering::default())?;
            let right_verify = if let Valid(right) = right {
                let (rp, rc) = right.parent_color.as_ref_color(tx, Ordering::default())?;
                assert!(rc == Color::Black);
                assert!(ptr::eq(unsafe { rp.unwrap() }, self));
                right.verify(tx)?
            } else {
                Verify {
                    black_depth: 1,
                    min_depth:   1,
                    max_depth:   1,
                    min:         None,
                    max:         None,
                }
            };
            assert_eq!(left_verify.black_depth, right_verify.black_depth);
            assert!(left_verify.max.map(|m| m < self.key).unwrap_or(true));
            assert!(right_verify.min.map(|m| self.key < m).unwrap_or(true));
            let result = Verify {
                black_depth: left_verify.black_depth,
                min_depth:   left_verify.min_depth.min(right_verify.min_depth) + 1,
                max_depth:   left_verify.max_depth.max(right_verify.max_depth) + 1,
                min:         Some(left_verify.min.unwrap_or(self.key.clone())),
                max:         Some(right_verify.max.unwrap_or(self.key.clone())),
            };
            assert!(result.min_depth * 2 >= result.max_depth);
            Ok(result)
        } else {
            let left = self.left.as_ref(tx, Ordering::default())?;
            let left_verify = if let Valid(left) = left {
                let (lp, _) = left.parent_color.as_ref_color(tx, Ordering::default())?;
                assert!(ptr::eq(unsafe { lp.unwrap() }, self));
                left.verify(tx)?
            } else {
                Verify {
                    black_depth: 1,
                    min_depth:   1,
                    max_depth:   1,
                    min:         None,
                    max:         None,
                }
            };
            let right = self.right.as_ref(tx, Ordering::default())?;
            let right_verify = if let Valid(right) = right {
                let (rp, _) = right.parent_color.as_ref_color(tx, Ordering::default())?;
                assert!(ptr::eq(unsafe { rp.unwrap() }, self));
                right.verify(tx)?
            } else {
                Verify {
                    black_depth: 1,
                    min_depth:   1,
                    max_depth:   1,
                    min:         None,
                    max:         None,
                }
            };
            assert_eq!(left_verify.black_depth, right_verify.black_depth);
            assert!(left_verify.max.map(|m| m < self.key).unwrap_or(true));
            assert!(right_verify.min.map(|m| self.key < m).unwrap_or(true));
            let result = Verify {
                black_depth: left_verify.black_depth + 1,
                min_depth:   left_verify.min_depth.min(right_verify.min_depth) + 1,
                max_depth:   left_verify.max_depth.max(right_verify.max_depth) + 1,
                min:         Some(left_verify.min.unwrap_or(self.key.clone())),
                max:         Some(right_verify.max.unwrap_or(self.key.clone())),
            };
            assert!(result.min_depth * 2 >= result.max_depth);
            Ok(result)
        }
    }
}

pub struct Verify<K> {
    black_depth: usize,
    min_depth:   usize,
    max_depth:   usize,
    min:         Option<K>,
    max:         Option<K>,
}

pub enum VacantLocation<'a, K, V> {
    Empty,
    Left { parent: &'a RBNode<K, V> },
    Right { parent: &'a RBNode<K, V> },
}

pub enum Location<'a, K, V> {
    Vacant(VacantLocation<'a, K, V>),
    Occupied { node: &'a RBNode<K, V> },
}

pub enum RepairResult<'a, T> {
    NewRoot(&'a T),
    PublishRoot(Box<T>),
    RootUnchanged,
}

pub struct RBRoot<K, V> {
    root: RBPtr<K, V>,
}

impl<K, V> RBRoot<K, V> {
    pub const fn new() -> Self {
        RBRoot {
            root: RBPtr::null(),
        }
    }
}

impl<K, V> Drop for RBRoot<K, V> {
    fn drop(&mut self) {
        unsafe {
            self.root.as_mut_ref().map(|root| root.destroy());
        }
    }
}

impl<K: Send + Sync + Ord + 'static, V: Send + Sync + 'static> RBRoot<K, V> {
    fn root<'tcell>(&'tcell self, tx: &impl Read<'tcell>) -> Result<RBRef<'tcell, K, V>, Error> {
        self.root.as_ref(tx, Ordering::default())
    }

    pub fn location_read<'tcell, Q>(
        &'tcell self,
        tx: &impl Read<'tcell>,
        key: &Q,
    ) -> Result<Location<'tcell, K, V>, Error>
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let root = self.root(tx)?;
        let loc = if let Valid(root) = root {
            root.find_impl(tx, &key, Ordering::Read)?
        } else {
            Location::Vacant(VacantLocation::Empty)
        };
        Ok(loc)
    }

    pub fn location_readwrite<'tcell, Q>(
        &'tcell self,
        tx: &impl Read<'tcell>,
        key: &Q,
    ) -> Result<Location<'tcell, K, V>, Error>
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let root = self.root(tx)?;
        let loc = if let Valid(root) = root {
            root.find_impl(tx, &key, Ordering::default())?
        } else {
            Location::Vacant(VacantLocation::Empty)
        };
        Ok(loc)
    }

    #[inline]
    pub fn location<'tcell, Q>(
        &'tcell self,
        tx: &impl Read<'tcell>,
        key: &Q,
        ordering: Ordering,
    ) -> Result<Location<'tcell, K, V>, Error>
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match ordering {
            Ordering::Read => self.location_read(tx, key),
            Ordering::ReadWrite => self.location_readwrite(tx, key),
            _ => unimplemented!(),
        }
    }

    pub fn contains_key<'tcell, Q>(
        &'tcell self,
        tx: &impl Read<'tcell>,
        key: &Q,
    ) -> Result<bool, Error>
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord + ?Sized,
    {
        Ok(match self.location(tx, key, Ordering::default())? {
            Location::Vacant(..) => false,
            Location::Occupied { .. } => true,
        })
    }

    pub fn insert<'tx, 'tcell>(
        &'tcell self,
        tx: &'tx mut impl Rw<'tcell>,
        key: K,
        value: V,
        location: VacantLocation<'tcell, K, V>,
    ) -> Result<&'tx mut V, Error> {
        let mut n = Box::new(RBNode {
            left: RBPtr::null(),
            right: RBPtr::null(),
            parent_color: RBPtrColor::null_black(),
            key,
            value: value,
        });
        let n_ptr = &mut *n as *mut RBNode<K, V>;
        let (parent, on_left) = match location {
            VacantLocation::Empty => {
                self.root.publish_box(tx, n)?;
                return Ok(&mut unsafe { &mut *n_ptr }.value);
            }
            VacantLocation::Left { parent } => (parent, true),
            VacantLocation::Right { parent } => (parent, false),
        };
        let new_root = n.insert_fixup(tx, parent, on_left)?;
        match new_root {
            RepairResult::NewRoot(new_root) => self.root.set(tx, Valid(new_root))?,
            RepairResult::PublishRoot(root) => self.root.publish_box(tx, root)?,
            RepairResult::RootUnchanged => {}
        }
        Ok(&mut unsafe { &mut *n_ptr }.value)
    }

    pub fn remove<'tcell>(
        &'tcell self,
        tx: &mut impl Rw<'tcell>,
        node: &'tcell RBNode<K, V>,
    ) -> Result<&'tcell V, Error> {
        let value = &node.value;
        let (mut new_root, rebalance) = node.remove_nofix(tx)?;
        if let Valid(rebalance) = rebalance {
            new_root = rebalance.remove_fixup(tx)?.map(|n| Valid(n)).or(new_root);
        }
        if let Some(new_root) = new_root {
            self.root.set(tx, new_root)?;
        };
        Ok(value)
    }

    pub fn verify<'tcell>(&'tcell self, tx: &impl Read<'tcell>) -> Result<(), Error>
    where
        K: Clone,
    {
        let root = self.root(tx)?;
        if let Valid(root) = root {
            root.verify(tx)?;
            Ok(())
        } else {
            Ok(())
        }
    }
}
