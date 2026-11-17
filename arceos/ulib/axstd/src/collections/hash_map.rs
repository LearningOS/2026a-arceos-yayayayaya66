//! A hash table for environments without an operating-system entropy source.
//!
//! The default hasher is deterministic FNV-1a. It is suitable for trusted keys,
//! but does not provide the randomized collision resistance of std's RandomState.

use alloc::vec::Vec;
use core::borrow::Borrow;
use core::fmt;
use core::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};
use core::iter::FusedIterator;
use core::ops::Index;

#[derive(Clone)]
enum Bucket<K, V> {
    Empty,
    Deleted,
    Full(u64, K, V),
}

/// The deterministic default hasher used by this collection.
#[derive(Clone)]
pub struct DefaultHasher(u64);

impl Default for DefaultHasher {
    fn default() -> Self {
        Self(0xcbf29ce484222325)
    }
}

impl Hasher for DefaultHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
    }
}

/// A hash map using linear probing and a maximum load factor of 75%.
///
/// Keys must have compatible Eq and Hash implementations. Changing a key's hash
/// or equality while it is in the map is a logic error, as with std::HashMap.
#[derive(Clone)]
pub struct HashMap<K, V, S = BuildHasherDefault<DefaultHasher>> {
    buckets: Vec<Bucket<K, V>>,
    len: usize,
    deleted: usize,
    hash_builder: S,
}

impl<K, V> HashMap<K, V> {
    /// Creates an empty map without allocating.
    pub fn new() -> Self {
        Self::with_hasher(BuildHasherDefault::default())
    }

    /// Creates a map that can hold at least capacity entries without growing.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_and_hasher(capacity, BuildHasherDefault::default())
    }
}

impl<K, V, S> HashMap<K, V, S> {
    pub const fn with_hasher(hash_builder: S) -> Self {
        Self {
            buckets: Vec::new(),
            len: 0,
            deleted: 0,
            hash_builder,
        }
    }

    pub fn with_capacity_and_hasher(capacity: usize, hash_builder: S) -> Self {
        let mut map = Self::with_hasher(hash_builder);
        map.reserve(capacity);
        map
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn capacity(&self) -> usize {
        self.buckets.len() / 4 * 3
    }

    pub fn hasher(&self) -> &S {
        &self.hash_builder
    }

    /// Removes all entries, retaining the allocated table.
    pub fn clear(&mut self) {
        for bucket in &mut self.buckets {
            *bucket = Bucket::Empty;
        }
        self.len = 0;
        self.deleted = 0;
    }

    /// Reserves space for at least additional more entries.
    pub fn reserve(&mut self, additional: usize) {
        let required = self
            .len
            .checked_add(additional)
            .expect("HashMap capacity overflow");
        if required > self.capacity() {
            let mut size = self.buckets.len().max(8);
            while size / 4 * 3 < required {
                size = size.checked_mul(2).expect("HashMap capacity overflow");
            }
            self.rehash(size);
        } else if self.deleted > self.buckets.len() / 4 {
            self.rehash(self.buckets.len());
        }
    }

    fn rehash(&mut self, size: usize) {
        let mut buckets = Vec::with_capacity(size);
        buckets.resize_with(size, || Bucket::Empty);
        let old = core::mem::replace(&mut self.buckets, buckets);
        for bucket in old {
            if let Bucket::Full(hash, key, value) = bucket {
                let mut index = hash as usize & (size - 1);
                while !matches!(self.buckets[index], Bucket::Empty) {
                    index = (index + 1) & (size - 1);
                }
                self.buckets[index] = Bucket::Full(hash, key, value);
            }
        }
        self.deleted = 0;
    }

    pub fn iter(&self) -> Iter<'_, K, V> {
        Iter {
            inner: self.buckets.iter(),
            remaining: self.len,
        }
    }

    pub fn iter_mut(&mut self) -> IterMut<'_, K, V> {
        IterMut {
            inner: self.buckets.iter_mut(),
            remaining: self.len,
        }
    }

    pub fn keys(&self) -> impl ExactSizeIterator<Item = &K> + FusedIterator {
        self.iter().map(|(key, _)| key)
    }

    pub fn values(&self) -> impl ExactSizeIterator<Item = &V> + FusedIterator {
        self.iter().map(|(_, value)| value)
    }

    pub fn values_mut(&mut self) -> impl ExactSizeIterator<Item = &mut V> + FusedIterator {
        self.iter_mut().map(|(_, value)| value)
    }
}

impl<K, V, S: BuildHasher> HashMap<K, V, S> {
    fn hash<Q: Hash + ?Sized>(&self, key: &Q) -> u64 {
        let mut hasher = self.hash_builder.build_hasher();
        key.hash(&mut hasher);
        hasher.finish()
    }

    fn find<Q>(&self, hash: u64, key: &Q) -> Option<usize>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        if self.buckets.is_empty() {
            return None;
        }
        let mask = self.buckets.len() - 1;
        let mut index = hash as usize & mask;
        for _ in 0..self.buckets.len() {
            match &self.buckets[index] {
                Bucket::Empty => return None,
                Bucket::Full(stored, candidate, _)
                    if *stored == hash && candidate.borrow() == key =>
                {
                    return Some(index);
                }
                _ => index = (index + 1) & mask,
            }
        }
        None
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.get_key_value(key).map(|(_, value)| value)
    }

    pub fn get_key_value<Q>(&self, key: &Q) -> Option<(&K, &V)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let index = self.find(self.hash(key), key)?;
        match &self.buckets[index] {
            Bucket::Full(_, key, value) => Some((key, value)),
            _ => unreachable!(),
        }
    }

    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let index = self.find(self.hash(key), key)?;
        match &mut self.buckets[index] {
            Bucket::Full(_, _, value) => Some(value),
            _ => unreachable!(),
        }
    }

    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.get(key).is_some()
    }

    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.remove_entry(key).map(|(_, value)| value)
    }

    pub fn remove_entry<Q>(&mut self, key: &Q) -> Option<(K, V)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let index = self.find(self.hash(key), key)?;
        let old = core::mem::replace(&mut self.buckets[index], Bucket::Deleted);
        self.len -= 1;
        self.deleted += 1;
        match old {
            Bucket::Full(_, key, value) => Some((key, value)),
            _ => unreachable!(),
        }
    }
}

impl<K: Hash + Eq, V, S: BuildHasher> HashMap<K, V, S> {
    /// Inserts a value, returning the previous value without replacing its key.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let hash = self.hash(&key);
        if let Some(index) = self.find(hash, &key) {
            if let Bucket::Full(_, _, stored) = &mut self.buckets[index] {
                return Some(core::mem::replace(stored, value));
            }
        }
        self.reserve(1);
        let mask = self.buckets.len() - 1;
        let mut index = hash as usize & mask;
        while matches!(self.buckets[index], Bucket::Full(..)) {
            index = (index + 1) & mask;
        }
        if matches!(self.buckets[index], Bucket::Deleted) {
            self.deleted -= 1;
        }
        self.buckets[index] = Bucket::Full(hash, key, value);
        self.len += 1;
        None
    }
}

impl<K, V, S: Default> Default for HashMap<K, V, S> {
    fn default() -> Self {
        Self::with_hasher(S::default())
    }
}

impl<K: fmt::Debug, V: fmt::Debug, S> fmt::Debug for HashMap<K, V, S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_map().entries(self.iter()).finish()
    }
}

impl<K, V, S, Q> Index<&Q> for HashMap<K, V, S>
where
    K: Borrow<Q>,
    Q: Hash + Eq + ?Sized,
    S: BuildHasher,
{
    type Output = V;

    fn index(&self, key: &Q) -> &V {
        self.get(key).expect("no entry found for key")
    }
}

impl<K: Hash + Eq, V, S: BuildHasher> Extend<(K, V)> for HashMap<K, V, S> {
    fn extend<T: IntoIterator<Item = (K, V)>>(&mut self, iter: T) {
        for (key, value) in iter {
            self.insert(key, value);
        }
    }
}

impl<K: Hash + Eq, V, S: BuildHasher + Default> FromIterator<(K, V)> for HashMap<K, V, S> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        let mut map = Self::default();
        map.extend(iter);
        map
    }
}

/// Iterator over immutable key-value pairs.
pub struct Iter<'a, K, V> {
    inner: core::slice::Iter<'a, Bucket<K, V>>,
    remaining: usize,
}

impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        for bucket in self.inner.by_ref() {
            if let Bucket::Full(_, key, value) = bucket {
                self.remaining -= 1;
                return Some((key, value));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V> ExactSizeIterator for Iter<'_, K, V> {}
impl<K, V> FusedIterator for Iter<'_, K, V> {}

/// Iterator over immutable keys and mutable values.
pub struct IterMut<'a, K, V> {
    inner: core::slice::IterMut<'a, Bucket<K, V>>,
    remaining: usize,
}

impl<'a, K, V> Iterator for IterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> {
        for bucket in self.inner.by_ref() {
            if let Bucket::Full(_, key, value) = bucket {
                self.remaining -= 1;
                return Some((key, value));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V> ExactSizeIterator for IterMut<'_, K, V> {}
impl<K, V> FusedIterator for IterMut<'_, K, V> {}

/// Iterator consuming a map's entries.
pub struct IntoIter<K, V> {
    inner: alloc::vec::IntoIter<Bucket<K, V>>,
    remaining: usize,
}

impl<K, V> Iterator for IntoIter<K, V> {
    type Item = (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        for bucket in self.inner.by_ref() {
            if let Bucket::Full(_, key, value) = bucket {
                self.remaining -= 1;
                return Some((key, value));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V> ExactSizeIterator for IntoIter<K, V> {}
impl<K, V> FusedIterator for IntoIter<K, V> {}

impl<K, V, S> IntoIterator for HashMap<K, V, S> {
    type Item = (K, V);
    type IntoIter = IntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            inner: self.buckets.into_iter(),
            remaining: self.len,
        }
    }
}

impl<'a, K, V, S> IntoIterator for &'a HashMap<K, V, S> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, K, V, S> IntoIterator for &'a mut HashMap<K, V, S> {
    type Item = (&'a K, &'a mut V);
    type IntoIter = IterMut<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::string::String;

    #[test]
    fn growth_borrowed_lookup_and_iteration() {
        let mut map = HashMap::new();
        for value in 0..50_000 {
            assert_eq!(map.insert(format!("key_{value}"), value), None);
        }
        assert_eq!(map.len(), 50_000);
        assert_eq!(map.iter().len(), 50_000);
        for (key, value) in &map {
            assert_eq!(key, &format!("key_{value}"));
            assert_eq!(map.get(key.as_str()), Some(value));
        }
        assert_eq!(map.insert(String::from("key_10"), 99), Some(10));
        assert_eq!(map.len(), 50_000);
        assert_eq!(map["key_10"], 99);
        *map.get_mut("key_10").unwrap() = 10;
        assert_eq!(map.remove("key_10"), Some(10));
        assert_eq!(map.remove("key_10"), None);
        assert!(!map.contains_key("key_10"));
        assert_eq!(map.into_iter().count(), 49_999);
    }

    #[derive(Default)]
    struct CollisionHasher;

    impl Hasher for CollisionHasher {
        fn finish(&self) -> u64 {
            7
        }
        fn write(&mut self, _: &[u8]) {}
    }

    #[test]
    fn collisions_deletion_and_reuse() {
        let mut map = HashMap::with_hasher(BuildHasherDefault::<CollisionHasher>::default());
        for key in 0..400 {
            map.insert(key, key * 2);
        }
        for key in (0..400).step_by(2) {
            assert_eq!(map.remove(&key), Some(key * 2));
        }
        for key in (1..400).step_by(2) {
            assert_eq!(map.get(&key), Some(&(key * 2)));
        }
        for key in 400..800 {
            map.insert(key, key * 2);
        }
        assert_eq!(map.len(), 600);
        for (_, value) in &mut map {
            *value += 1;
        }
        assert_eq!(map.get(&799), Some(&1599));
        let capacity = map.capacity();
        map.clear();
        assert!(map.is_empty());
        assert_eq!(map.capacity(), capacity);
        map.insert(1, 2);
        assert_eq!(map.get(&1), Some(&2));
    }

    #[test]
    fn preserves_existing_key_and_drops_values() {
        use alloc::rc::Rc;
        use core::cell::Cell;

        #[derive(Debug)]
        struct Key(u32, &'static str);
        impl PartialEq for Key {
            fn eq(&self, rhs: &Self) -> bool {
                self.0 == rhs.0
            }
        }
        impl Eq for Key {}
        impl Hash for Key {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self.0.hash(state);
            }
        }
        struct Value(Rc<Cell<usize>>);
        impl Drop for Value {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        let drops = Rc::new(Cell::new(0));
        let mut map = HashMap::new();
        map.insert(Key(1, "original"), Value(drops.clone()));
        drop(map.insert(Key(1, "replacement"), Value(drops.clone())));
        assert_eq!(map.keys().next().unwrap().1, "original");
        assert_eq!(drops.get(), 1);
        map.clear();
        assert_eq!(drops.get(), 2);

        let map: HashMap<_, _> = [(1, 2), (1, 3), (2, 4)].into_iter().collect();
        assert_eq!(map.len(), 2);
        assert_eq!(map[&1], 3);
    }
}
