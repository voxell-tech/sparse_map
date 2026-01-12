#![doc = include_str!("../README.md")]
#![no_std]

extern crate alloc;

use core::fmt::{Display, Formatter};

use alloc::vec::Vec;

/// A sparse, generational map keyed by [`Key`].
///
/// `SparseMap` provides stable keys with generation to prevent
/// use-after-free bugs. Internally, it reuses vacant slots while
/// incrementing a generation counter to invalidate stale keys.
///
/// ## Guarantees
///
/// - Insertion is **O(1)**.
/// - Removal is **O(1)**.
/// - Lookup is **O(1)**.
/// - Keys are invalidated once their value is removed.
#[derive(Debug)]
pub struct SparseMap<T> {
    buffer: Vec<Option<T>>,
    generations: Vec<u32>,
    empty_slots: Vec<usize>,
}

impl<T> SparseMap<T> {
    /// Creates a new empty sparse map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a value into the map and returns a unique [`Key`].
    ///
    /// Vacant slots are reused when possible. If a slot is reused,
    /// its generation counter is incremented to invalidate old keys.
    #[must_use = "The returned key is the only way to reference back the inserted value!"]
    pub fn insert(&mut self, value: T) -> Key {
        self.insert_with_key(|_, _| value)
    }

    /// Similar to [`Self::insert()`] but provides a [`Key`] before
    /// inserting the value.
    pub fn insert_with_key<F>(&mut self, create: F) -> Key
    where
        F: FnOnce(&mut Self, Key) -> T,
    {
        if let Some(index) = self.empty_slots.pop() {
            // Increment the generation counter.
            let mut generation = self.generations[index];
            generation = generation.wrapping_add(1);
            self.generations[index] = generation;

            let key = Key::new(index, generation);

            let item = create(self, key);
            self.buffer[index] = Some(item);

            key
        } else {
            let index = self.buffer.len();
            self.generations.insert(index, 0);

            let key = Key::new(index, 0);

            let item = create(self, key);
            self.buffer.insert(index, Some(item));

            key
        }
    }

    /// Removes a value associated with the given key.
    ///
    /// Returns `None` if the key is invalid or already removed.
    /// The slot is marked for reuse.
    pub fn remove(&mut self, key: &Key) -> Option<T> {
        let item = self.buffer.get_mut(key.index)?;
        self.empty_slots.push(key.index);
        item.take()
    }

    /// Returns an immutable reference to the value for the given
    /// key if present.
    pub fn get(&self, key: &Key) -> Option<&T> {
        let item = self.buffer.get(key.index)?;
        let generation = self.generations.get(key.index)?;
        if *generation == key.generation {
            return item.as_ref();
        }

        None
    }

    /// Returns a mutable reference to the value for the given key
    /// if present.
    pub fn get_mut(&mut self, key: &Key) -> Option<&mut T> {
        let item = self.buffer.get_mut(key.index)?;
        let generation = self.generations.get(key.index)?;
        if *generation == key.generation {
            return item.as_mut();
        }

        None
    }

    /// Temporarily takes ownership of the value associated with `key`,
    /// passes it to `f`, and then places it back into the map.
    ///
    /// This enables safe, scoped mutation of a value while still allowing
    /// `f` to mutate the map itself (e.g. insert or remove other entries)
    /// without violating Rust’s aliasing rules.
    ///
    /// Returns `None` if `key` does not currently refer to a live value.
    ///
    /// # Guarantees
    /// - The value is removed from the map for the duration of `f`.
    /// - The value is restored to the same slot after `f` returns.
    /// - The key remains valid if and only if it was valid before
    ///   the call.
    ///
    /// # Example
    ///
    /// ```
    /// use sparse_map::SparseMap;
    ///
    /// let mut map = SparseMap::new();
    /// let key = map.insert(42);
    ///
    /// map.scope(&key, |map, value| {
    ///     *value += 2;
    ///     map.insert(24);
    /// });
    ///
    /// assert_eq!(map.get(&key), Some(&44));
    /// ```
    pub fn scope<F, R>(&mut self, key: &Key, f: F) -> Option<R>
    where
        F: FnOnce(&mut Self, &mut T) -> R,
    {
        if !self.contains(key) {
            return None;
        }

        // SAFETY: We already checked that the key contains a value.
        let mut value = self.buffer[key.index].take().unwrap();
        let result = f(self, &mut value);
        self.buffer[key.index] = Some(value);

        Some(result)
    }

    /// Returns `true` if the key currently refers to a live value.
    pub fn contains(&self, key: &Key) -> bool {
        self.buffer
            .get(key.index)
            .zip(self.generations.get(key.index))
            .is_some_and(|(item, generation)| {
                item.is_some() && *generation == key.generation
            })
    }

    /// Returns the number of live values stored in the map.
    ///
    /// This is **O(1)** and does not require scanning the buffer.
    pub fn len(&self) -> usize {
        self.buffer.len() - self.empty_slots.len()
    }

    /// Returns `true` if the map contains no live values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T> Default for SparseMap<T> {
    fn default() -> Self {
        Self {
            buffer: Vec::new(),
            generations: Vec::new(),
            empty_slots: Vec::new(),
        }
    }
}

/// A stable, generational key into a [`SparseMap`].
///
/// A `Key` identifies a slot by **index** and **generation**.
/// This prevents stale keys from accessing values after a slot has
/// been removed and reused.
///
/// # Semantics
/// - `index` selects a slot in the map’s internal buffer.
/// - `generation` must match the slot’s current generation for the
///   key to be valid.
///
/// If a value is removed, the slot’s generation is incremented.
/// Any previously issued `Key` for that slot becomes invalid.
#[derive(
    Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord,
)]
pub struct Key {
    index: usize,
    generation: u32,
}

impl Key {
    fn new(index: usize, generation: u32) -> Self {
        Self { index, generation }
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }
}

impl Display for Key {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!(
            "#{}v{}",
            self.index, self.generation
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_get() {
        let mut map = SparseMap::new();

        let key = map.insert(42);
        assert_eq!(map.get(&key), Some(&42));
        assert!(map.contains(&key));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn insert_and_insert_with_key_behave_equivalently() {
        let mut map = SparseMap::new();

        let k1 = map.insert(1);
        let k2 = map.insert_with_key(|_, _| 2);

        assert_eq!(k1.index, 0);
        assert_eq!(k2.index, 1);

        assert_eq!(map.buffer[k1.index], Some(1));
        assert_eq!(map.buffer[k2.index], Some(2));
    }

    #[test]
    fn remove_invalidates_key() {
        let mut map = SparseMap::new();

        let key = map.insert(10);
        let removed = map.remove(&key);

        assert_eq!(removed, Some(10));
        assert_eq!(map.get(&key), None);
        assert!(!map.contains(&key));
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn insert_reuse_bumps_generation() {
        let mut map = SparseMap::new();

        let k1 = map.insert(1);
        map.remove(&k1);

        let k2 = map.insert(2);

        assert_eq!(k1.index, k2.index);
        assert_ne!(k1.generation, k2.generation);

        assert_eq!(map.get(&k1), None);
        assert_eq!(map.get(&k2), Some(&2));
    }

    #[test]
    fn insert_with_key_reuse_bumps_generation() {
        let mut map = SparseMap::new();

        let k1 = map.insert_with_key(|_, _| 1);
        map.remove(&k1);

        let k2 = map.insert_with_key(|_, _| 2);

        assert_eq!(k1.index, k2.index);
        assert_ne!(k1.generation, k2.generation);

        assert_eq!(map.get(&k1), None);
        assert_eq!(map.get(&k2), Some(&2));
    }

    #[test]
    fn get_mut_works() {
        let mut map = SparseMap::new();

        let key = map.insert(5);
        *map.get_mut(&key).unwrap() = 99;

        assert_eq!(map.get(&key), Some(&99));
    }

    #[test]
    fn removing_twice_is_safe() {
        let mut map = SparseMap::new();

        let key = map.insert(7);
        assert_eq!(map.remove(&key), Some(7));
        assert_eq!(map.remove(&key), None);
    }

    #[test]
    fn invalid_index_returns_none() {
        let mut map = SparseMap::<usize>::new();

        let fake_key = Key::new(999, 0);
        assert_eq!(map.get(&fake_key), None);
        assert_eq!(map.get_mut(&fake_key), None);
        assert!(!map.contains(&fake_key));
    }
}
