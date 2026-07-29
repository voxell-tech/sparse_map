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
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct SparseMap<T> {
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    buffer: Vec<Option<T>>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    generations: Vec<u32>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
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
        let generation = self.generations.get(key.index)?;
        if *generation != key.generation {
            return None;
        }
        let item = self.buffer.get_mut(key.index)?;
        if item.is_none() {
            return None;
        }
        self.empty_slots.push(key.index);
        item.take()
    }

    /// Temporarily removes the value at `key` without freeing the
    /// slot.
    ///
    /// The slot stays reserved at its current generation so the key
    /// remains valid. Call [`Self::restore`] to put the value back.
    ///
    /// # Warning
    ///
    /// Not calling [`Self::restore`] after `take` leaks the slot:
    /// the index is never returned to the free list, so it cannot
    /// be reused.
    ///
    /// Returns `None` if the key does not refer to a live value.
    pub fn take(&mut self, key: &Key) -> Option<T> {
        let generation = self.generations.get(key.index)?;
        if *generation != key.generation {
            return None;
        }
        self.buffer.get_mut(key.index)?.take()
    }

    /// Puts a value back into the slot identified by `key`.
    ///
    /// This is the counterpart to [`Self::take`]. Returns
    /// `false` if the slot is already occupied or the key is
    /// no longer valid.
    pub fn restore(&mut self, key: &Key, value: T) -> bool {
        // Check if generation matches.
        let Some(&generation) = self.generations.get(key.index)
        else {
            return false;
        };
        if generation != key.generation {
            return false;
        }

        // Check if slot exists and is `None`.
        let Some(slot) = self.buffer.get_mut(key.index) else {
            return false;
        };
        if slot.is_some() {
            return false;
        }

        *slot = Some(value);
        true
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
        let generation = self.generations.get(key.index)?;
        if *generation != key.generation {
            return None;
        }

        let mut value = self.buffer[key.index].take()?;
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

    /// Iterates shared references to every live value.
    ///
    /// Values are yielded in slot order, which is not guaranteed to
    /// match insertion order once slots have been reused.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.buffer.iter().flatten()
    }

    /// Iterates mutable references to every live value.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.buffer.iter_mut().flatten()
    }

    /// Removes every live value and yields it by value, leaving the
    /// map empty.
    ///
    /// Each drained slot is freed for reuse and its generation
    /// bumped, so any outstanding [`Key`] is invalidated. Dropping
    /// the returned iterator before it is exhausted still removes the
    /// remaining values.
    pub fn drain(&mut self) -> Drain<'_, T> {
        Drain {
            map: self,
            index: 0,
        }
    }

    /// Removes every live value stored in the map.
    ///
    /// Like [`Self::drain`] but discards the values. Each freed slot
    /// has its generation bumped, so keys to the removed values are
    /// invalidated. A slot reserved by [`Self::take`] or
    /// [`Self::scope`] keeps its checked-out value and is left
    /// untouched for the matching [`Self::restore`].
    pub fn clear(&mut self) {
        for (index, slot) in self.buffer.iter_mut().enumerate() {
            // Only free currently-live slots. A slot reserved via
            // take()/scope() holds its value off to the side, so it
            // must stay reserved for restore() to return it.
            if slot.take().is_some() {
                self.generations[index] =
                    self.generations[index].wrapping_add(1);
                self.empty_slots.push(index);
            }
        }
    }
}

/// Draining iterator for a [`SparseMap`], created by
/// [`SparseMap::drain`].
pub struct Drain<'a, T> {
    map: &'a mut SparseMap<T>,
    index: usize,
}

impl<T> Iterator for Drain<'_, T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        while self.index < self.map.buffer.len() {
            let index = self.index;
            self.index += 1;

            if self.map.buffer[index].is_some() {
                // Free the slot and invalidate its key.
                let generation =
                    self.map.generations[index].wrapping_add(1);
                self.map.generations[index] = generation;
                self.map.empty_slots.push(index);
                return self.map.buffer[index].take();
            }
        }

        None
    }
}

impl<T> Drop for Drain<'_, T> {
    fn drop(&mut self) {
        // Remove any values left when the iterator is dropped early.
        for _ in self.by_ref() {}
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
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct Key {
    index: usize,
    generation: u32,
}

impl Key {
    /// A sentinel key that will never refer to a live value.
    pub const PLACEHOLDER: Self = Self {
        index: usize::MAX,
        generation: u32::MAX,
    };

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

    #[test]
    fn take_returns_value() {
        let mut map = SparseMap::new();
        let key = map.insert(42);

        assert_eq!(map.take(&key), Some(42));
    }

    #[test]
    fn take_makes_slot_empty_but_key_generation_preserved() {
        let mut map = SparseMap::new();
        let key = map.insert(42);

        map.take(&key);

        // Slot is empty so get/contains return nothing.
        assert_eq!(map.get(&key), None);
        assert!(!map.contains(&key));
        // Generation is intact so restore can use the same key.
        assert_eq!(map.generations[key.index], key.generation);
    }

    #[test]
    fn take_does_not_free_slot_for_reuse() {
        let mut map = SparseMap::new();
        let key = map.insert(1);
        map.take(&key);

        // Inserting a new value must go to a fresh slot, not key's slot.
        let key2 = map.insert(2);
        assert_ne!(key.index, key2.index);
    }

    #[test]
    fn take_invalid_key_returns_none() {
        let mut map = SparseMap::<i32>::new();
        let fake_key = Key::new(999, 0);
        assert_eq!(map.take(&fake_key), None);
    }

    #[test]
    fn take_already_taken_returns_none() {
        let mut map = SparseMap::new();
        let key = map.insert(7);

        assert_eq!(map.take(&key), Some(7));
        assert_eq!(map.take(&key), None);
    }

    #[test]
    fn restore_puts_value_back() {
        let mut map = SparseMap::new();
        let key = map.insert(10);

        map.take(&key);
        assert!(map.restore(&key, 10));

        assert_eq!(map.get(&key), Some(&10));
        assert!(map.contains(&key));
    }

    #[test]
    fn restore_fails_if_slot_occupied() {
        let mut map = SparseMap::new();
        let key = map.insert(5);

        // Slot is occupied, restore should fail.
        assert!(!map.restore(&key, 99));
        // Original value is untouched.
        assert_eq!(map.get(&key), Some(&5));
    }

    #[test]
    fn restore_fails_on_stale_key() {
        let mut map = SparseMap::new();
        let k1 = map.insert(1);
        map.remove(&k1);
        let _k2 = map.insert(2); // bumps generation on same slot

        // k1 is stale, restore must reject it.
        assert!(!map.restore(&k1, 99));
    }

    #[test]
    fn restore_fails_on_invalid_index() {
        let mut map = SparseMap::<i32>::new();
        let fake_key = Key::new(999, 0);
        assert!(!map.restore(&fake_key, 42));
    }

    #[test]
    fn take_restore_roundtrip_key_stays_valid() {
        let mut map = SparseMap::new();
        let key = map.insert(100);

        map.take(&key);
        assert!(map.restore(&key, 200));
        assert_eq!(map.get(&key), Some(&200));
    }

    #[test]
    fn take_restore_allows_map_mutation_in_between() {
        let mut map = SparseMap::new();
        let key = map.insert(1);

        let value = map.take(&key).unwrap();
        // Insert and remove other entries while value is out.
        let other = map.insert(99);
        map.remove(&other);

        assert!(map.restore(&key, value));
        assert_eq!(map.get(&key), Some(&1));
    }

    #[test]
    fn iter_yields_only_live_values() {
        let mut map = SparseMap::new();
        let _ = map.insert(1);
        let key = map.insert(2);
        let _ = map.insert(3);
        map.remove(&key);

        let mut values = map.iter().copied().collect::<Vec<_>>();
        values.sort();
        assert_eq!(values, [1, 3]);
    }

    #[test]
    fn iter_mut_allows_mutation() {
        let mut map = SparseMap::new();
        let _ = map.insert(1);
        let _ = map.insert(2);

        for value in map.iter_mut() {
            *value *= 10;
        }

        let mut values = map.iter().copied().collect::<Vec<_>>();
        values.sort();
        assert_eq!(values, [10, 20]);
    }

    #[test]
    fn drain_yields_all_and_empties() {
        let mut map = SparseMap::new();
        let _ = map.insert(1);
        let _ = map.insert(2);

        let mut drained = map.drain().collect::<Vec<_>>();
        drained.sort();
        assert_eq!(drained, [1, 2]);
        assert!(map.is_empty());
    }

    #[test]
    fn drain_invalidates_keys_and_reuses_slots() {
        let mut map = SparseMap::new();
        let k1 = map.insert(1);
        map.drain().count();

        assert_eq!(map.get(&k1), None);

        let k2 = map.insert(2);
        assert_eq!(k1.index(), k2.index());
        assert_ne!(k1.generation(), k2.generation());
    }

    #[test]
    fn drain_dropped_early_still_empties() {
        let mut map = SparseMap::new();
        let _ = map.insert(1);
        let _ = map.insert(2);
        let _ = map.insert(3);

        {
            let mut drain = map.drain();
            let _ = drain.next();
        }

        assert!(map.is_empty());
    }

    #[test]
    fn clear_empties_and_invalidates_keys() {
        let mut map = SparseMap::new();
        let key = map.insert(1);

        map.clear();

        assert!(map.is_empty());
        assert_eq!(map.get(&key), None);

        // Reusing the previous slot must carry a fresh generation so
        // the stale key stays invalid.
        let reused = map.insert(4);
        assert_eq!(key.index(), reused.index());
        assert_ne!(key.generation(), reused.generation());
        assert_eq!(map.get(&key), None);
    }

    #[test]
    fn clear_leaves_reserved_slot_restorable() {
        let mut map = SparseMap::new();
        let key = map.insert(1);
        let value = map.take(&key).unwrap();

        // The value is checked out, so clear must not free its slot.
        map.clear();

        assert!(map.restore(&key, value));
        assert_eq!(map.get(&key), Some(&1));
    }

    #[test]
    fn clear_inside_scope_preserves_scoped_value() {
        let mut map = SparseMap::new();
        let key = map.insert(1);
        let other = map.insert(2);

        // The scoped value is checked out while `clear` runs, so it
        // survives; the other live value is removed.
        map.scope(&key, |map, _| map.clear());

        assert_eq!(map.get(&key), Some(&1));
        assert_eq!(map.get(&other), None);
    }
}
