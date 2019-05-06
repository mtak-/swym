#![feature(core_intrinsics)]
#![deny(unused_must_use)]

mod base;

use crate::base::{Location, RBNode, RBRoot, VacantLocation};
use swym::{
    tcell::{Ref, TCell, View},
    thread_key,
    tx::{Borrow, Error, Ordering, Read, SetError, Status},
    RwTx,
};

pub struct RBTreeMapRaw<K, V> {
    pub root: RBRoot<K, TCell<V>>,
}

impl<K, V> RBTreeMapRaw<K, V> {
    pub const fn new() -> Self {
        RBTreeMapRaw {
            root: RBRoot::new(),
        }
    }
}

impl<K: Clone + Send + Sync + Ord + 'static, V: Send + Sync + 'static> RBTreeMapRaw<K, V> {
    pub fn verify<'tcell, Q>(&'tcell self, tx: &impl Read<'tcell>) -> Result<(), Error>
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.root.verify(tx)
    }
}

impl<K: Send + Sync + Ord + 'static, V: Send + Sync + 'static> RBTreeMapRaw<K, V> {
    pub fn contains_key<'tcell, Q>(
        &'tcell self,
        tx: &impl Read<'tcell>,
        key: &Q,
    ) -> Result<bool, Error>
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.root.contains_key(tx, key)
    }

    pub fn entry<'tx, 'tcell>(
        &'tcell self,
        tx: &'tx mut RwTx<'tcell>,
        key: K,
    ) -> Result<Entry<'tx, 'tcell, K, V>, Error> {
        Ok(match self.root.location(tx, &key, Ordering::default())? {
            Location::Vacant(location) => Entry::Vacant(VacantEntry {
                location,
                tree: self,
                tx,
                key,
            }),
            Location::Occupied { node } => Entry::Occupied(OccupiedEntry {
                node,
                tree: self,
                tx,
                key,
            }),
        })
    }
}

impl<K: Send + Sync + Ord + 'static, V: Borrow + Send + Sync + 'static> RBTreeMapRaw<K, V> {
    pub fn get<'tx, 'tcell, Q>(
        &'tcell self,
        tx: &'tx impl Read<'tcell>,
        key: &Q,
    ) -> Result<Option<Ref<'tx, V>>, Error>
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let loc = self.root.location(tx, &key, Ordering::default())?;
        let result = match loc {
            Location::Vacant(_) => None,
            Location::Occupied { node } => Some(node.value.borrow(tx, Ordering::default())?),
        };
        Ok(result)
    }

    pub fn get_mut<'tx, 'tcell, Q>(
        &'tcell self,
        tx: &'tx mut RwTx<'tcell>,
        key: &Q,
    ) -> Result<Option<View<'tcell, V, &'tx mut RwTx<'tcell>>>, Error>
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let loc = self.root.location(tx, &key, Ordering::default())?;
        let result = match loc {
            Location::Vacant(_) => None,
            Location::Occupied { node } => Some(node.value.view(tx)),
        };
        Ok(result)
    }
}

impl<K: Send + Sync + Ord + 'static, V: Borrow + Clone + Send + Sync + 'static> RBTreeMapRaw<K, V> {
    pub fn insert<'tcell>(
        &'tcell self,
        tx: &mut RwTx<'tcell>,
        key: K,
        value: V,
    ) -> Result<Option<V>, Error> {
        let loc = self.root.location(tx, &key, Ordering::Read)?;
        let result = match loc {
            Location::Vacant(vacant) => {
                self.root.insert(tx, key, TCell::new(value), vacant)?;
                None
            }
            Location::Occupied { node } => Some(node.value.replace(tx, value)?),
        };
        Ok(result)
    }
}

impl<K: Send + Sync + Ord + 'static, V: Borrow + Send + Sync + 'static> RBTreeMapRaw<K, V> {
    pub fn remove<'tcell, Q>(
        &'tcell self,
        tx: &mut RwTx<'tcell>,
        key: &Q,
    ) -> Result<Option<Ref<'tcell, V>>, Error>
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord,
    {
        let loc = self.root.location(tx, &key, Ordering::Read)?;
        match loc {
            Location::Vacant(..) => Ok(None),
            Location::Occupied { node } => {
                let v = self.root.remove(tx, node)?;
                Ok(Some(unsafe {
                    Ref::downcast(v.borrow(tx, Ordering::default())?, tx)
                }))
            }
        }
    }
}

pub struct RBTreeMap<K, V> {
    pub raw: RBTreeMapRaw<K, V>,
}

impl<K, V> RBTreeMap<K, V> {
    pub const fn new() -> Self {
        RBTreeMap {
            raw: RBTreeMapRaw::new(),
        }
    }
}

impl<K: Send + Sync + Ord + 'static, V: Borrow + Send + Sync + 'static> RBTreeMap<K, V> {
    pub fn with<'tx, 'tcell>(
        &'tcell self,
        tx: &'tx mut RwTx<'tcell>,
    ) -> RBTreeWith<'tx, 'tcell, K, V> {
        RBTreeWith {
            tree: &self.raw,
            tx,
        }
    }

    pub fn atomic<F, R>(&self, mut f: F) -> R
    where
        F: for<'tx, 'tcell> FnMut(RBTreeWith<'tx, 'tcell, K, V>) -> Result<R, Status>,
    {
        thread_key::get().rw(move |tx| f(self.with(tx)))
    }

    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord + ?Sized,
    {
        thread_key::get().read(move |tx| Ok(self.raw.contains_key(tx, key)?))
    }

    pub fn get<Q>(&self, key: &Q) -> Option<V>
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord + ?Sized,
        V: Clone,
    {
        thread_key::get().read(move |tx| {
            let r = self.raw.get(tx, key)?.map(|value| value.clone());
            Ok(r)
        })
    }

    pub fn insert(&self, key: K, value: V) -> Option<V>
    where
        K: Clone,
        V: Clone,
    {
        // todo: remove these clones
        self.atomic(move |mut tree| Ok(tree.insert(key.clone(), value.clone())?))
    }

    pub fn remove<Q>(&self, key: &Q) -> Option<V>
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord,
        V: Clone,
    {
        self.atomic(move |mut tree| {
            let value = tree.remove(key)?;
            Ok(value.map(|value| value.clone()))
        })
    }
}

pub struct RBTreeWith<'tx, 'tcell, K, V> {
    pub tree: &'tcell RBTreeMapRaw<K, V>,
    pub tx:   &'tx mut RwTx<'tcell>,
}

impl<'tx, 'tcell, K, V> RBTreeWith<'tx, 'tcell, K, V>
where
    K: Send + Sync + Ord + 'static,
    V: Borrow + Send + Sync + 'static,
{
    pub fn get<Q>(&self, key: &Q) -> Result<Option<Ref<'_, V>>, Error>
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.tree.get(self.tx, key)
    }

    pub fn contains_key<Q>(&self, key: &Q) -> Result<bool, Error>
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.tree.contains_key(self.tx, key)
    }

    pub fn entry<'a>(&'a mut self, key: K) -> Result<Entry<'a, 'tcell, K, V>, Error> {
        self.tree.entry(self.tx, key)
    }

    pub fn insert<'a>(&'a mut self, key: K, value: V) -> Result<Option<V>, Error>
    where
        V: Clone,
    {
        self.tree.insert(self.tx, key, value)
    }

    pub fn remove<'a, Q>(&'a mut self, key: &Q) -> Result<Option<Ref<'tx, V>>, Error>
    where
        K: std::borrow::Borrow<Q>,
        Q: Ord,
    {
        self.tree.remove(self.tx, key)
    }
}

pub struct VacantEntry<'tx, 'tcell, K, V> {
    location: VacantLocation<'tcell, K, TCell<V>>,
    tree:     &'tcell RBTreeMapRaw<K, V>,
    tx:       &'tx mut RwTx<'tcell>,
    key:      K,
}

impl<'tx, 'tcell, K, V> VacantEntry<'tx, 'tcell, K, V>
where
    K: Ord + Send + Sync + 'static,
    V: Borrow + Send + Sync + 'static,
{
    pub fn key(&self) -> &K {
        &self.key
    }

    pub fn into_key(self) -> K {
        self.key
    }

    pub fn insert(self, value: V) -> Result<&'tx mut V, Error> {
        let value = self
            .tree
            .root
            .insert(self.tx, self.key, TCell::new(value), self.location)?;
        Ok(value.borrow_mut())
    }
}

pub struct OccupiedEntry<'tx, 'tcell, K, V> {
    node: &'tcell RBNode<K, TCell<V>>,
    tree: &'tcell RBTreeMapRaw<K, V>,
    tx:   &'tx mut RwTx<'tcell>,
    key:  K,
}

impl<'tx, 'tcell, K, V> OccupiedEntry<'tx, 'tcell, K, V>
where
    K: Ord + Send + Sync + 'static,
    V: Borrow + Send + Sync + 'static,
{
    pub fn key(&self) -> &K {
        &self.key
    }

    pub fn view<'a>(&'a mut self) -> View<'tcell, V, &'a mut RwTx<'tcell>> {
        self.node.value.view(self.tx)
    }

    pub fn into_view(self) -> View<'tcell, V, &'tx mut RwTx<'tcell>> {
        self.node.value.view(self.tx)
    }

    pub fn insert(&mut self, value: V) -> Result<V, Error>
    where
        V: Clone,
    {
        Ok(self.node.value.replace(self.tx, value)?)
    }

    pub fn remove(self) -> Result<Ref<'tx, V>, Error> {
        let value = self.tree.root.remove(self.tx, self.node)?;
        value.borrow(self.tx, Ordering::default())
    }
}

pub enum Entry<'tx, 'tcell, K, V> {
    Vacant(VacantEntry<'tx, 'tcell, K, V>),
    Occupied(OccupiedEntry<'tx, 'tcell, K, V>),
}

impl<'tx, 'tcell, K, V> Entry<'tx, 'tcell, K, V>
where
    K: Ord + Send + Sync + 'static,
    V: Borrow + Send + Sync + 'static,
{
    #[inline]
    pub fn or_insert(self, default: V) -> Result<Value<'tx, 'tcell, V>, Error> {
        self.or_insert_with(move || default)
    }

    #[inline]
    pub fn or_insert_with<F: FnOnce() -> V>(
        self,
        default: F,
    ) -> Result<Value<'tx, 'tcell, V>, Error> {
        Ok(match self {
            Entry::Occupied(entry) => Value::Shared(entry.into_view()),
            Entry::Vacant(entry) => Value::Owned(entry.insert(default())?),
        })
    }

    #[inline]
    pub fn or_default(self) -> Result<Value<'tx, 'tcell, V>, Error>
    where
        V: Default,
    {
        self.or_insert_with(Default::default)
    }

    #[inline]
    pub fn key(&self) -> &K {
        match self {
            Entry::Occupied(entry) => entry.key(),
            Entry::Vacant(entry) => entry.key(),
        }
    }

    #[inline]
    pub fn and_modify<F>(self, f: F) -> Self
    where
        F: FnOnce(View<'tcell, V, &mut RwTx<'tcell>>),
    {
        match self {
            Entry::Occupied(OccupiedEntry {
                node,
                tree,
                tx,
                key,
            }) => {
                f(node.value.view(tx));
                Entry::Occupied(OccupiedEntry {
                    node,
                    tree,
                    tx,
                    key,
                })
            }
            this => this,
        }
    }
}

pub enum Value<'tx, 'tcell, V> {
    Owned(&'tx mut V),
    Shared(View<'tcell, V, &'tx mut RwTx<'tcell>>),
}

impl<'tx, 'tcell, V: Send + 'static> Value<'tx, 'tcell, V> {
    pub fn set(&mut self, value: V) -> Result<(), SetError<V>> {
        match self {
            Value::Owned(dest) => Ok(**dest = value),
            Value::Shared(view) => view.set(value),
        }
    }
}
