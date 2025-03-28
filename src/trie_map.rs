use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::mem;
use std::ops::{Index, IndexMut};

use crate::as_bytes::AsBytes;
use crate::entry::{Entry, OccupiedEntry, VacantEntry};
use crate::iter::{DrainIter, Iter, IterState, Keys, PrefixIter, PrefixKeys, PrefixValues, Values};
use crate::node::{clear_bit, popcount, set_bit, test_bit, TrieNode};
use crate::slice_pool::SlicePool;

/// A `TrieMap` is a key-value data structure that uses a trie (prefix tree) for storage
/// and retrieval of data.
///
/// # Features
///
/// - Fast key lookups with O(k) complexity where k is the key length
/// - Prefix-based operations (matching keys with a common prefix)
/// - Iterator support
/// - Entry API for efficient in-place updates
///
/// # Examples
///
/// ```
/// use triemap::TrieMap;
///
/// // Create a new TrieMap
/// let mut map = TrieMap::new();
///
/// // Insert key-value pairs
/// map.insert("apple", 1);
/// map.insert("banana", 2);
/// map.insert("cherry", 3);
///
/// // Check if a key exists
/// assert!(map.contains_key("apple"));
/// assert!(!map.contains_key("grape"));
///
/// // Get a value
/// assert_eq!(map.get("banana"), Some(&2));
///
/// // Update a value
/// map.insert("apple", 10);
/// assert_eq!(map.get("apple"), Some(&10));
///
/// // Remove a value
/// assert_eq!(map.remove("cherry"), Some(3));
/// assert_eq!(map.get("cherry"), None);
///
/// // Iterate over key-value pairs
/// for (key, value) in map.iter() {
///     println!("{}: {}", String::from_utf8_lossy(&key), value);
/// }
/// ```
pub struct TrieMap<T> {
    pub(crate) data: Vec<Option<T>>,
    pub(crate) free_indices: Vec<usize>,
    pub(crate) root: TrieNode,
    pub(crate) size: usize,
    pub(crate) pool: SlicePool,
}

impl<T, K: AsBytes, V: Into<T>, const N: usize> From<[(K, V); N]> for TrieMap<T> {
    fn from(array: [(K, V); N]) -> Self {
        let mut trie = TrieMap::with_capacity(N);
        for (key, value) in array {
            trie.insert(key, value.into());
        }
        trie
    }
}

impl<T, K: AsBytes, V: Into<T>> From<&[(K, V)]> for TrieMap<T>
where
    K: Clone,
    V: Clone,
{
    fn from(slice: &[(K, V)]) -> Self {
        let mut trie = TrieMap::with_capacity(slice.len());
        for (key, value) in slice {
            let value_owned: V = value.clone();
            trie.insert(key.clone(), value_owned.into());
        }
        trie
    }
}

impl<T: Hash> Hash for TrieMap<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.size.hash(state);

        let mut entries: Vec<_> = self.iter().collect();
        entries.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));

        for (key, value) in entries {
            key.hash(state);
            value.hash(state);
        }
    }
}

impl<T: Clone> Clone for TrieMap<T> {
    fn clone(&self) -> Self {
        TrieMap {
            data: self.data.clone(),
            free_indices: self.free_indices.clone(),
            root: self.root.clone(),
            size: self.size,
            pool: SlicePool::new(),
        }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for TrieMap<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut map_debug = f.debug_map();

        for (key, value) in self.iter() {
            let key_display = match std::str::from_utf8(&key) {
                Ok(s) => s.to_string(),
                Err(_) => format!("{:?}", key),
            };

            map_debug.entry(&key_display, value);
        }

        map_debug.finish()
    }
}

impl<T: PartialEq> PartialEq for TrieMap<T> {
    fn eq(&self, other: &Self) -> bool {
        if self.size != other.size {
            return false;
        }

        let mut self_pairs: Vec<_> = self.iter().collect();
        let mut other_pairs: Vec<_> = other.iter().collect();

        self_pairs.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));
        other_pairs.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));

        self_pairs == other_pairs
    }
}

impl<T: Eq> Eq for TrieMap<T> {}

impl<T, Q: ?Sized> Index<&Q> for TrieMap<T>
where
    Q: AsBytes,
{
    type Output = T;

    fn index(&self, key: &Q) -> &Self::Output {
        self.get(key).expect("no entry found for key")
    }
}

impl<T, Q: ?Sized> IndexMut<&Q> for TrieMap<T>
where
    Q: AsBytes,
{
    fn index_mut(&mut self, key: &Q) -> &mut Self::Output {
        self.get_mut(key).expect("no entry found for key")
    }
}

impl<T, K: AsBytes, V: Into<T>> Extend<(K, V)> for TrieMap<T> {
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
        for (k, v) in iter {
            self.insert(k, v.into());
        }
    }
}

impl<T, K, V> FromIterator<(K, V)> for TrieMap<T>
where
    K: AsBytes,
    V: Into<T>,
{
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut trie = TrieMap::new();
        for (key, value) in iter {
            trie.insert(key, value.into());
        }
        trie
    }
}

impl<T> From<HashMap<String, T>> for TrieMap<T> {
    fn from(map: HashMap<String, T>) -> Self {
        let mut trie = TrieMap::with_capacity(map.len());
        for (k, v) in map {
            trie.insert(k, v);
        }
        trie
    }
}

impl<T> From<BTreeMap<String, T>> for TrieMap<T> {
    fn from(map: BTreeMap<String, T>) -> Self {
        let mut trie = TrieMap::with_capacity(map.len());
        for (k, v) in map {
            trie.insert(k, v);
        }
        trie
    }
}

impl<T: Clone> From<TrieMap<T>> for HashMap<Vec<u8>, T> {
    fn from(trie: TrieMap<T>) -> Self {
        trie.into_iter().collect()
    }
}

impl<T> Default for TrieMap<T> {
    /// Creates a new empty `TrieMap`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let map: TrieMap<i32> = Default::default();
    /// assert!(map.is_empty());
    /// ```
    fn default() -> Self {
        Self::new()
    }
}

impl<T> TrieMap<T> {
    /// Creates a new empty `TrieMap`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let map: TrieMap<i32> = TrieMap::new();
    /// assert!(map.is_empty());
    /// ```
    pub fn new() -> Self {
        TrieMap {
            data: Vec::new(),
            free_indices: Vec::new(),
            root: TrieNode::new(),
            size: 0,
            pool: SlicePool::new(),
        }
    }

    /// Creates a new `TrieMap` with the specified capacity.
    ///
    /// The map will be able to hold at least `capacity` elements without
    /// reallocating.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let map: TrieMap<i32> = TrieMap::with_capacity(10);
    /// assert!(map.is_empty());
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        TrieMap {
            data: Vec::with_capacity(capacity),
            free_indices: Vec::new(),
            root: TrieNode::new(),
            size: 0,
            pool: SlicePool::new(),
        }
    }

    /// Returns the number of elements in the map.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// assert_eq!(map.len(), 0);
    ///
    /// map.insert("a", 1);
    /// assert_eq!(map.len(), 1);
    /// ```
    pub fn len(&self) -> usize {
        self.size
    }

    /// Returns `true` if the map contains no elements.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// assert!(map.is_empty());
    ///
    /// map.insert("a", 1);
    /// assert!(!map.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Removes all elements from the map.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    /// map.insert("b", 2);
    ///
    /// map.clear();
    /// assert!(map.is_empty());
    /// ```
    pub fn clear(&mut self) {
        self.data.clear();
        self.free_indices.clear();
        self.root = TrieNode::new();
        self.size = 0;
    }

    /// Inserts a key-value pair into the map.
    ///
    /// This method inserts a value associated with a key into the map.
    /// If the key already exists, its value is updated.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    /// map.insert("a", 2); // Updates the existing value
    /// assert_eq!(map.get("a"), Some(&2));
    /// assert_eq!(map.len(),1);
    /// ```
    pub fn insert<K: AsBytes>(&mut self, key: K, value: T) {
        let bytes = key.as_bytes();
        let mut current = &mut self.root;

        for &byte in bytes {
            let idx = popcount(&current.is_present, byte) as usize;

            if !test_bit(&current.is_present, byte) {
                let current_size = current.children.len();
                let mut new_children = self.pool.get(current_size + 1);

                for i in 0..idx {
                    mem::swap(&mut new_children[i], &mut current.children[i]);
                    //new_children.push(std::mem::replace(&mut current.children[i], TrieNode::new()));
                }

                new_children[idx] = TrieNode::new();

                for i in idx..current_size {
                    mem::swap(&mut new_children[i + 1], &mut current.children[i]);
                }

                let old_children = mem::replace(&mut current.children, new_children);
                self.pool.put(old_children);

                set_bit(&mut current.is_present, byte);
            }

            current = &mut current.children[idx];
        }

        let idx = if let Some(free_idx) = self.free_indices.pop() {
            // Use a previously freed index
            self.data[free_idx] = Some(value);
            free_idx
        } else {
            // No free indices, add to the end
            self.data.push(Some(value));
            self.data.len() - 1
        };

        let prev_idx = current.data_idx;

        // Update node to point to the new data index
        current.data_idx = Some(idx);

        // If this is a new key, increment size
        if prev_idx.is_none() {
            self.size += 1;
        } else if let Some(prev_idx) = prev_idx {
            // Free the previous index for reuse
            self.data[prev_idx] = None;
            self.free_indices.push(prev_idx);
        }
    }

    /// Returns a reference to the value corresponding to the key.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    /// assert_eq!(map.get("a"), Some(&1));
    /// assert_eq!(map.get("b"), None);
    /// ```
    pub fn get<K: AsBytes>(&self, key: K) -> Option<&T> {
        let bytes = key.as_bytes();
        let mut current = &self.root;

        for &byte in bytes {
            if !test_bit(&current.is_present, byte) {
                return None;
            }

            let idx = popcount(&current.is_present, byte) as usize;
            if idx >= current.children.len() {
                return None;
            }

            current = &current.children[idx];
        }

        current.data_idx.and_then(|idx| self.data[idx].as_ref())
    }

    /// Returns a mutable reference to the value corresponding to the key.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    ///
    /// if let Some(value) = map.get_mut("a") {
    ///     *value = 10;
    /// }
    ///
    /// assert_eq!(map.get("a"), Some(&10));
    /// ```
    pub fn get_mut<K: AsBytes>(&mut self, key: K) -> Option<&mut T> {
        let bytes = key.as_bytes();
        let mut current = &self.root;

        for &byte in bytes {
            if !test_bit(&current.is_present, byte) {
                return None;
            }

            let idx = popcount(&current.is_present, byte) as usize;
            if idx >= current.children.len() {
                return None;
            }

            current = &current.children[idx];
        }

        if let Some(idx) = current.data_idx {
            self.data[idx].as_mut()
        } else {
            None
        }
    }

    /// Returns `true` if the map contains a value for the specified key.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    /// assert!(map.contains_key("a"));
    /// assert!(!map.contains_key("b"));
    /// ```
    pub fn contains_key<K: AsBytes>(&self, key: K) -> bool {
        self.get(key).is_some()
    }

    /// Returns an entry representing a key in the map.
    ///
    /// The entry can be used to insert, remove, or modify the value associated with the key.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::{TrieMap, Entry};
    /// let mut map = TrieMap::new();
    ///
    /// // Insert a value if the key doesn't exist
    /// map.entry("a").or_insert(1);
    ///
    /// // Update a value if the key exists
    /// if let Entry::Occupied(mut occupied) = map.entry("a") {
    ///     *occupied.get_mut() += 10;
    /// }
    ///
    /// assert_eq!(map.get("a"), Some(&11));
    /// ```
    pub fn entry<K: AsBytes>(&mut self, key: K) -> Entry<'_, T> {
        let key_bytes = key.as_bytes().to_vec();

        let mut current = &self.root;
        let mut found = true;

        for &byte in &key_bytes {
            if !test_bit(&current.is_present, byte) {
                found = false;
                break;
            }

            let idx = popcount(&current.is_present, byte) as usize;
            if idx >= current.children.len() {
                found = false;
                break;
            }

            current = &current.children[idx];
        }

        if found && current.data_idx.is_some() {
            let data_idx = current.data_idx.unwrap();

            if data_idx < self.data.len() && self.data[data_idx].is_some() {
                return Entry::Occupied(OccupiedEntry {
                    trie: self,
                    key: key_bytes,
                    data_idx,
                });
            }
        }

        Entry::Vacant(VacantEntry {
            trie: self,
            key: key_bytes,
        })
    }

    /// Removes a key from the map, returning the value at the key if the key was previously in the map.
    /// This does not remove the nodes used to register the key
    /// Use `remove_and_prune` method for removing intermittent nodes as well.
    /// Use `prune` method to remove *all* nodes that leads to a thombstone
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    ///
    /// assert_eq!(map.remove("a"), Some(1));
    /// assert_eq!(map.remove("a"), None);
    /// ```
    pub fn remove<K: AsBytes>(&mut self, key: K) -> Option<T> {
        let bytes = key.as_bytes();

        self.remove_internal(bytes)
    }

    fn remove_internal(&mut self, bytes: &[u8]) -> Option<T> {
        let mut current = &mut self.root;
        let mut found = true;

        for &byte in bytes {
            if !test_bit(&current.is_present, byte) {
                found = false;
                break;
            }
            let idx = popcount(&current.is_present, byte) as usize;
            if idx >= current.children.len() {
                found = false;
                break;
            }
            current = &mut current.children[idx];
        }

        if found && current.data_idx.is_some() {
            let data_idx = current.data_idx.unwrap();

            if data_idx < self.data.len() && self.data[data_idx].is_some() {
                let value = self.data[data_idx].take();
                current.data_idx = None;
                self.free_indices.push(data_idx);
                self.size -= 1;
                return value;
            }
        }

        None
    }

    /// Removes a key from the map, returning the value at the key if the key was previously in the map.
    /// This method also removes the nodes used to register the key
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    ///
    /// assert_eq!(map.remove("a"), Some(1));
    /// assert_eq!(map.remove("a"), None);
    /// ```
    pub fn remove_and_prune<K: AsBytes>(&mut self, key: K) -> Option<T> {
        let bytes = key.as_bytes();

        self.remove_and_prune_internal(bytes)
    }

    fn remove_and_prune_internal(&mut self, bytes: &[u8]) -> Option<T> {
        let mut path = Vec::with_capacity(bytes.len());
        let mut path_indices = Vec::with_capacity(bytes.len());

        let mut current = &self.root;

        for &byte in bytes {
            if !test_bit(&current.is_present, byte) {
                return None;
            }

            let idx = popcount(&current.is_present, byte) as usize;
            if idx >= current.children.len() {
                return None;
            }

            path.push(byte);
            path_indices.push(idx);
            current = &current.children[idx];
        }

        if let Some(idx) = current.data_idx {
            if self.data[idx].is_some() {
                self.size -= 1;

                self.free_indices.push(idx);

                let value = self.data[idx].take();

                let mut delete_child = true;

                for depth in (0..path.len()).rev() {
                    let byte = path[depth];
                    let child_idx = path_indices[depth];

                    let mut current = &mut self.root;

                    for item in path_indices.iter_mut().take(depth) {
                        current = &mut current.children[*item]
                    }

                    let child = &current.children[child_idx];
                    if delete_child && child.data_idx.is_none() && child.children.is_empty() {
                        let current_size = current.children.len();
                        let mut new_children = self.pool.get(current_size - 1);
                        let mut new_idx = 0;

                        for i in 0..current_size {
                            if i != child_idx {
                                mem::swap(&mut new_children[new_idx], &mut current.children[i]);
                                new_idx += 1;
                            }
                        }
                        let old_children = mem::replace(&mut current.children, new_children);
                        self.pool.put(old_children);

                        clear_bit(&mut current.is_present, byte);

                        delete_child = current.data_idx.is_none() && current.children.is_empty();
                    } else {
                        delete_child = false;
                    }
                }

                value
            } else {
                None
            }
        } else {
            None
        }
    }
    pub fn prune(&mut self) -> usize {
        // We need to avoid having two mutable references to self
        // Let's extract the nodes we need separately
        let mut root = std::mem::take(&mut self.root);
        let slice_pool = &mut self.pool;

        let pruned = Self::prune_node_helper(&mut root, slice_pool);

        // Put the root back
        self.root = root;

        pruned
    }

    fn prune_node_helper(node: &mut TrieNode, slice_pool: &mut SlicePool) -> usize {
        let mut pruned_nodes = 0;
        let mut bytes_to_clear = Vec::new();

        for byte in 0..=255u8 {
            if test_bit(&node.is_present, byte) {
                let idx = popcount(&node.is_present, byte) as usize;
                if idx < node.children.len() {
                    // Recursively prune the child node
                    let child_pruned = Self::prune_node_helper(&mut node.children[idx], slice_pool);
                    pruned_nodes += child_pruned;

                    if node.children[idx].data_idx.is_none()
                        && node.children[idx].children.is_empty()
                    {
                        bytes_to_clear.push(byte);
                    }
                }
            }
        }

        if !bytes_to_clear.is_empty() {
            let current_size = node.children.len();
            let new_size = current_size - bytes_to_clear.len();

            if new_size == 0 {
                let old_children = std::mem::replace(&mut node.children, Box::new([]));
                slice_pool.put(old_children);
            } else {
                let mut new_children = slice_pool.get(new_size);
                let mut new_idx = 0;

                for byte in 0..=255u8 {
                    if test_bit(&node.is_present, byte) && !bytes_to_clear.contains(&byte) {
                        let idx = popcount(&node.is_present, byte) as usize;
                        if idx < node.children.len() {
                            std::mem::swap(&mut new_children[new_idx], &mut node.children[idx]);
                            new_idx += 1;
                        }
                    }
                }

                let old_children = std::mem::replace(&mut node.children, new_children);
                slice_pool.put(old_children);
            }

            pruned_nodes += bytes_to_clear.len();
            for byte in bytes_to_clear {
                clear_bit(&mut node.is_present, byte);
            }
        }

        pruned_nodes
    }
    /// Prunes unused nodes from the trie to reclaim memory.
    ///
    /// This method removes all nodes that don't contain values and don't lead to nodes with values.
    /// It's useful to call periodically if you've removed many items from the trie.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("apple", 1);
    /// map.insert("application", 2);
    ///
    /// map.remove("apple");
    /// map.remove("application");
    ///
    /// // The trie structure still contains nodes for "apple" and "application"
    /// // even though the values have been removed
    ///
    /// map.prune();
    /// // Now the unused nodes have been removed
    /// ```

    /// Returns an iterator over the key-value pairs of the map.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    /// map.insert("b", 2);
    ///
    /// for (key, value) in map.iter() {
    ///     println!("{}: {}", String::from_utf8_lossy(&key), value);
    /// }
    /// ```
    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            trie: self,
            stack: vec![IterState {
                node: &self.root,
                byte_index: 0,
                value_emitted: false,
            }],
            current_path: Vec::new(),
            remaining: self.size,
        }
    }

    fn collect_pairs<'a>(
        &'a self,
        node: &TrieNode,
        current_key: &mut Vec<u8>,
        pairs: &mut Vec<(Vec<u8>, &'a T)>,
    ) {
        if let Some(idx) = node.data_idx {
            if let Some(value) = self.data[idx].as_ref() {
                pairs.push((current_key.clone(), value));
            }
        }

        for byte in 0..=255u8 {
            if test_bit(&node.is_present, byte) {
                let idx = popcount(&node.is_present, byte) as usize;

                current_key.push(byte);

                self.collect_pairs(&node.children[idx], current_key, pairs);

                current_key.pop();
            }
        }
    }

    /// Returns an iterator over the keys of the map.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    /// map.insert("b", 2);
    ///
    /// for key in map.keys() {
    ///     println!("Key: {}", String::from_utf8_lossy(&key));
    /// }
    /// ```
    pub fn keys(&self) -> Keys<'_, T> {
        Keys { inner: self.iter() }
    }

    /// Returns an iterator over the values of the map.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    /// map.insert("b", 2);
    ///
    /// for value in map.values() {
    ///     println!("Value: {}", value);
    /// }
    /// ```
    pub fn values(&self) -> Values<'_, T> {
        Values { inner: self.iter() }
    }

    /// Returns a mutable iterator over the key-value pairs of the map.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    /// map.insert("b", 2);
    ///
    /// for (key, value) in map.iter_mut() {
    ///     *value += 10;
    /// }
    ///
    /// assert_eq!(map.get("a"), Some(&11));
    /// assert_eq!(map.get("b"), Some(&12));
    /// ```
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Vec<u8>, &mut T)> + '_ {
        let mut keys_indices = Vec::with_capacity(self.size);
        let mut current_key = Vec::new();
        Self::collect_keys_indices(&self.root, &mut current_key, &mut keys_indices);
        let map: std::collections::HashMap<_, _> =
            keys_indices.into_iter().map(|(x, y)| (y, x)).collect();

        self.data
            .iter_mut()
            .enumerate()
            .filter_map(move |(idx, opt)| opt.as_mut().map(|value| (map[&idx].clone(), value)))
    }

    /// Private helper to collect all keys and their associated data indices
    fn collect_keys_indices(
        node: &TrieNode,
        current_key: &mut Vec<u8>,
        keys_indices: &mut Vec<(Vec<u8>, usize)>,
    ) {
        if let Some(idx) = node.data_idx {
            keys_indices.push((current_key.clone(), idx));
        }

        for byte in 0..=255u8 {
            if test_bit(&node.is_present, byte) {
                let idx = popcount(&node.is_present, byte) as usize;

                if idx < node.children.len() {
                    current_key.push(byte);
                    Self::collect_keys_indices(&node.children[idx], current_key, keys_indices);
                    current_key.pop();
                }
            }
        }
    }

    /// Returns a mutable iterator over the values of the map.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    /// map.insert("b", 2);
    ///
    /// for value in map.values_mut() {
    ///     *value *= 2;
    /// }
    ///
    /// assert_eq!(map.get("a"), Some(&2));
    /// assert_eq!(map.get("b"), Some(&4));
    /// ```
    pub fn values_mut(&mut self) -> std::iter::Flatten<std::slice::IterMut<'_, Option<T>>> {
        self.data.iter_mut().flatten()
    }

    /// Returns an iterator over all key-value pairs with keys that start with the given prefix.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("apple", 1);
    /// map.insert("application", 2);
    /// map.insert("banana", 3);
    ///
    /// let mut iter = map.prefix_iter("app");
    /// assert_eq!(iter.next().unwrap().1, &1);
    /// assert_eq!(iter.next().unwrap().1, &2);
    /// assert!(iter.next().is_none());
    /// ```
    pub fn prefix_iter<K: crate::AsBytes>(&self, prefix: K) -> PrefixIter<'_, T> {
        let prefix_bytes = prefix.as_bytes();
        let mut current_node = &self.root;
        let mut current_path = Vec::with_capacity(prefix_bytes.len());
        let mut valid_prefix = true;

        // Navigate to the node corresponding to the prefix
        for &byte in prefix_bytes {
            if !test_bit(&current_node.is_present, byte) {
                valid_prefix = false;
                break;
            }

            let idx = crate::node::popcount(&current_node.is_present, byte) as usize;
            if idx >= current_node.children.len() {
                valid_prefix = false;
                break;
            }

            current_path.push(byte);
            current_node = &current_node.children[idx];
        }

        // If the prefix is valid, start the iterator at that node
        if valid_prefix {
            // Count how many items we'll be returning
            let mut count = 0;
            let mut temp_path = current_path.clone();
            Self::count_items_recursive(current_node, &mut temp_path, &mut count);

            PrefixIter {
                trie: self,
                stack: vec![IterState {
                    node: current_node,
                    byte_index: 0,
                    value_emitted: false,
                }],
                current_path,
                remaining: count,
                prefix: prefix_bytes.to_vec(),
            }
        } else {
            // Empty iterator if prefix not found
            PrefixIter {
                trie: self,
                stack: Vec::new(),
                current_path: Vec::new(),
                remaining: 0,
                prefix: prefix_bytes.to_vec(),
            }
        }
    }
    fn count_items_recursive(node: &crate::node::TrieNode, _path: &mut Vec<u8>, count: &mut usize) {
        if node.data_idx.is_some() {
            *count += 1;
        }

        for byte in 0..=255u8 {
            if test_bit(&node.is_present, byte) {
                let idx = crate::node::popcount(&node.is_present, byte) as usize;
                if idx < node.children.len() {
                    _path.push(byte);
                    Self::count_items_recursive(&node.children[idx], _path, count);
                    _path.pop();
                }
            }
        }
    }
    /// Returns an iterator over all keys that start with the given prefix.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("apple", 1);
    /// map.insert("application", 2);
    /// map.insert("banana", 3);
    ///
    /// let mut keys = map.prefix_keys("app").collect::<Vec<_>>();
    /// keys.sort();
    ///
    /// assert_eq!(keys.len(), 2);
    /// assert_eq!(String::from_utf8(keys[0].clone()).unwrap(), "apple");
    /// assert_eq!(String::from_utf8(keys[1].clone()).unwrap(), "application");
    /// ```
    pub fn prefix_keys<K: AsBytes>(&self, prefix: K) -> PrefixKeys<'_, T> {
        PrefixKeys {
            inner: self.prefix_iter(prefix),
        }
    }

    /// Returns an iterator over all values whose keys start with the given prefix.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("apple", 1);
    /// map.insert("application", 2);
    /// map.insert("banana", 3);
    ///
    /// let mut values = map.prefix_values("app").collect::<Vec<_>>();
    /// values.sort();
    ///
    /// assert_eq!(values, vec![&1, &2]);
    /// ```
    pub fn prefix_values<K: AsBytes>(&self, prefix: K) -> PrefixValues<'_, T> {
        PrefixValues {
            inner: self.prefix_iter(prefix),
        }
    }

    /// Finds a node matching the given prefix
    fn find_node(&self, bytes: &[u8]) -> Option<&TrieNode> {
        let mut current = &self.root;

        for &byte in bytes {
            if !test_bit(&current.is_present, byte) {
                return None;
            }

            let idx = popcount(&current.is_present, byte) as usize;
            if idx >= current.children.len() {
                return None;
            }

            current = &current.children[idx];
        }

        Some(current)
    }

    /// Collects all prefix matches from a node
    fn collect_prefix_matches<'a>(
        &'a self,
        node: &TrieNode,
        prefix: &mut Vec<u8>,
        result: &mut Vec<(Vec<u8>, &'a T)>,
    ) {
        if let Some(idx) = node.data_idx {
            if let Some(value) = self.data[idx].as_ref() {
                result.push((prefix.clone(), value));
            }
        }

        for byte in 0..=255u8 {
            if test_bit(&node.is_present, byte) {
                let idx = popcount(&node.is_present, byte) as usize;
                if idx < node.children.len() {
                    prefix.push(byte);
                    self.collect_prefix_matches(&node.children[idx], prefix, result);
                    prefix.pop();
                }
            }
        }
    }

    /// Returns `true` if the map contains any keys starting with the given prefix.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("apple", 1);
    /// map.insert("application", 2);
    ///
    /// assert!(map.starts_with("app"));
    /// assert!(!map.starts_with("ban"));
    /// ```
    pub fn starts_with<K: AsBytes>(&self, prefix: K) -> bool {
        let bytes = prefix.as_bytes();

        if let Some(node) = self.find_node(bytes) {
            node.data_idx.is_some() && self.data[node.data_idx.unwrap()].is_some()
                || self.has_any_value(node)
        } else {
            false
        }
    }

    /// Determines if a node contains any values in its subtree
    fn has_any_value(&self, node: &TrieNode) -> bool {
        if let Some(idx) = node.data_idx {
            if self.data[idx].is_some() {
                return true;
            }
        }

        for byte in 0..=255u8 {
            if test_bit(&node.is_present, byte) {
                let idx = popcount(&node.is_present, byte) as usize;
                if idx < node.children.len() && self.has_any_value(&node.children[idx]) {
                    return true;
                }
            }
        }

        false
    }

    /// Returns all key-value pairs for keys that start with the given prefix.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("apple", 1);
    /// map.insert("application", 2);
    /// map.insert("banana", 3);
    ///
    /// let matches = map.get_prefix_matches("app");
    /// assert_eq!(matches.len(), 2);
    /// ```
    pub fn get_prefix_matches<K: AsBytes>(&self, prefix: K) -> Vec<(Vec<u8>, &'_ T)> {
        let bytes = prefix.as_bytes();
        let mut result = Vec::new();

        if let Some(node) = self.find_node(bytes) {
            let mut prefix_vec = bytes.to_vec();
            self.collect_prefix_matches(node, &mut prefix_vec, &mut result);
        }

        result
    }

    /// Removes all entries where the key starts with the given prefix.
    ///
    /// Returns the removed key-value pairs.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("apple", 1);
    /// map.insert("application", 2);
    /// map.insert("banana", 3);
    ///
    /// let removed = map.remove_prefix_matches("app");
    /// assert_eq!(removed.len(), 2);
    /// assert_eq!(map.len(), 1);
    /// ```
    pub fn remove_prefix_matches<K: AsBytes>(&mut self, prefix: K) -> Vec<(Vec<u8>, T)> {
        let bytes = prefix.as_bytes();
        let mut result = Vec::new();

        let keys_to_remove = if let Some(node) = self.find_node(bytes) {
            let mut keys = Vec::new();
            let mut prefix_vec = bytes.to_vec();
            self.collect_keys_with_prefix(node, &mut prefix_vec, &mut keys);
            keys
        } else {
            return result;
        };

        for key in keys_to_remove {
            if let Some(value) = self.remove_internal(&key) {
                result.push((key, value));
            }
        }

        result
    }

    fn collect_keys_with_prefix(
        &self,
        node: &TrieNode,
        prefix: &mut Vec<u8>,
        keys: &mut Vec<Vec<u8>>,
    ) {
        if let Some(idx) = node.data_idx {
            if self.data[idx].is_some() {
                keys.push(prefix.clone());
            }
        }

        for byte in 0..=255u8 {
            if test_bit(&node.is_present, byte) {
                let idx = popcount(&node.is_present, byte) as usize;
                if idx < node.children.len() {
                    prefix.push(byte);

                    self.collect_keys_with_prefix(&node.children[idx], prefix, keys);

                    prefix.pop();
                }
            }
        }
    }

    /// Removes all key-value pairs from the map, returning them as an iterator.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    /// map.insert("b", 2);
    ///
    /// let drained: Vec<_> = map.drain().collect();
    /// assert_eq!(drained.len(), 2);
    /// assert_eq!(map.len(), 0);
    /// ```
    pub fn drain(&mut self) -> DrainIter<T> {
        let mut keys = Vec::with_capacity(self.size);
        let mut current_key = Vec::new();

        self.collect_keys(&self.root, &mut current_key, &mut keys);

        DrainIter {
            trie_map: self,
            keys,
            position: 0,
        }
    }

    fn collect_keys(&self, node: &TrieNode, current_key: &mut Vec<u8>, keys: &mut Vec<Vec<u8>>) {
        if let Some(idx) = node.data_idx {
            if self.data[idx].is_some() {
                keys.push(current_key.clone());
            }
        }

        for byte in 0..=255u8 {
            if test_bit(&node.is_present, byte) {
                let idx = popcount(&node.is_present, byte) as usize;

                current_key.push(byte);
                self.collect_keys(&node.children[idx], current_key, keys);
                current_key.pop();
            }
        }
    }

    /// Returns all keys that start with the given prefix.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("apple", 1);
    /// map.insert("application", 2);
    /// map.insert("banana", 3);
    ///
    /// let keys = map.keys_starting_with("app");
    /// assert_eq!(keys.len(), 2);
    /// ```
    pub fn keys_starting_with<K: AsBytes>(&self, prefix: K) -> Vec<Vec<u8>> {
        let bytes = prefix.as_bytes();
        let mut result = Vec::new();

        if let Some(node) = self.find_node(bytes) {
            let mut prefix_vec = bytes.to_vec();
            self.collect_keys_with_prefix(node, &mut prefix_vec, &mut result);
        }

        result
    }

    /// Gets an entry for a key reference.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::{TrieMap, Entry};
    /// let mut map = TrieMap::new();
    /// let key = "test_key".to_string();
    ///
    /// // First, insert a value
    /// map.entry_ref(&key).or_insert(1);
    /// assert_eq!(map.get(&key), Some(&1));
    ///
    /// // Then, update it
    /// if let Entry::Occupied(mut occupied) = map.entry_ref(&key) {
    ///     *occupied.get_mut() = 2;
    /// }
    /// assert_eq!(map.get(&key), Some(&2));
    /// ```
    pub fn entry_ref<'a, K: AsBytes + ?Sized>(&'a mut self, key: &'a K) -> Entry<'a, T> {
        let key_bytes = key.as_bytes().to_vec();

        let mut current = &self.root;
        let mut found = true;

        for &byte in key.as_bytes() {
            if !test_bit(&current.is_present, byte) {
                found = false;
                break;
            }

            let idx = popcount(&current.is_present, byte) as usize;
            if idx >= current.children.len() {
                found = false;
                break;
            }

            current = &current.children[idx];
        }

        if found && current.data_idx.is_some() {
            let data_idx = current.data_idx.unwrap();
            if data_idx < self.data.len() && self.data[data_idx].is_some() {
                return Entry::Occupied(OccupiedEntry {
                    trie: self,
                    key: key_bytes,
                    data_idx,
                });
            }
        }

        Entry::Vacant(VacantEntry {
            trie: self,
            key: key_bytes,
        })
    }

    /// Retains only the elements specified by the predicate.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    /// map.insert("b", 2);
    /// map.insert("c", 3);
    /// map.insert("d", 4);
    ///
    /// // Keep only entries with even values
    /// map.retain(|_, v| *v % 2 == 0);
    ///
    /// assert_eq!(map.len(), 2);
    /// assert!(!map.contains_key("a"));
    /// assert!(map.contains_key("b"));
    /// assert!(!map.contains_key("c"));
    /// assert!(map.contains_key("d"));
    /// ```
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&[u8], &mut T) -> bool,
    {
        let keys: Vec<Vec<u8>> = self.keys().collect();

        let keys_to_remove = keys
            .iter()
            .filter_map(|k| {
                if let Some(value) = self.get_mut(k) {
                    if !f(k, value) {
                        return Some(k.clone());
                    }
                }
                None
            })
            .collect::<Vec<_>>();

        for key in keys_to_remove {
            self.remove(&key);
        }
    }

    /// Converts the map into an iterator over keys.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    /// map.insert("b", 2);
    ///
    /// let keys: Vec<_> = map.into_keys().collect();
    /// assert_eq!(keys.len(), 2);
    /// ```
    pub fn into_keys(self) -> impl Iterator<Item = Vec<u8>> {
        self.into_iter().map(|(key, _)| key)
    }

    /// Converts the map into an iterator over values.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    /// map.insert("b", 2);
    ///
    /// let values: Vec<_> = map.into_values().collect();
    /// assert_eq!(values.len(), 2);
    /// ```
    pub fn into_values(self) -> impl Iterator<Item = T> {
        self.into_iter().map(|(_, value)| value)
    }

    /// Shrinks the capacity of the map as much as possible.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::with_capacity(100);
    /// map.insert("a", 1);
    /// map.insert("b", 2);
    ///
    /// map.shrink_to_fit();
    /// ```
    pub fn shrink_to_fit(&mut self) {
        self.data.shrink_to_fit();
        self.free_indices.shrink_to_fit();
    }

    /// Returns the number of elements the map can hold without reallocating.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let map: TrieMap<i32> = TrieMap::with_capacity(100);
    /// assert!(map.capacity() >= 100);
    /// ```
    pub fn capacity(&self) -> usize {
        self.data.capacity()
    }

    /// Reserves capacity for at least `additional` more elements.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map : TrieMap<()> = TrieMap::new();
    /// map.reserve(100);
    /// ```
    pub fn reserve(&mut self, additional: usize) {
        self.data.reserve(additional);
    }

    /// Tries to insert a key-value pair into the map.
    ///
    /// If the map did not have this key present, the value is inserted and `Ok(&mut T)` is returned.
    ///
    /// If the map did have this key present, the value is not updated, and `Err(T)` is returned
    /// containing the value passed.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    ///
    /// // First insertion succeeds
    /// assert!(map.try_insert("a", 1).is_ok());
    ///
    /// // Second insertion fails
    /// assert!(map.try_insert("a", 2).is_err());
    ///
    /// assert_eq!(map.get("a"), Some(&1));
    /// ```
    pub fn try_insert<K: AsBytes>(&mut self, key: K, value: T) -> Result<&mut T, T> {
        match self.entry(key) {
            Entry::Vacant(entry) => Ok(entry.insert(value)),
            Entry::Occupied(_) => Err(value),
        }
    }

    /// Returns the key-value pair corresponding to the supplied key.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    ///
    /// assert!(map.get_key_value("a").is_some());
    /// let (key, &value) = map.get_key_value("a").unwrap();
    /// assert_eq!(String::from_utf8_lossy(&key), "a");
    /// assert_eq!(value, 1);
    /// ```
    pub fn get_key_value<K: AsBytes + Clone>(&self, key: K) -> Option<(Vec<u8>, &T)> {
        let k2 = key.clone();
        let bytes = key.as_bytes();
        self.get(k2).map(|value| (bytes.to_vec(), value))
    }

    /// Gets the given key's corresponding value if it exists, otherwise inserts a default value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    ///
    /// // First call inserts the default value
    /// *map.get_or_insert_default("a") = 1;
    /// assert_eq!(map.get("a"), Some(&1));
    ///
    /// // Second call doesn't change the value
    /// *map.get_or_insert_default("a") = 2;
    /// assert_eq!(map.get("a"), Some(&2));
    /// ```
    pub fn get_or_insert_default<K: AsBytes>(&mut self, key: K) -> &mut T
    where
        T: Default,
    {
        if self.contains_key(&key) {
            self.get_mut(key).unwrap()
        } else {
            let value = T::default();
            self.insert(&key, value);
            self.get_mut(key).unwrap()
        }
    }

    /// Gets the given key's corresponding value if it exists, otherwise inserts a value using the default function.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    ///
    /// // First call inserts the generated value
    /// *map.get_or_insert_with("a", || 42) = 1;
    /// assert_eq!(map.get("a"), Some(&1));
    ///
    /// // Second call doesn't change the value
    /// let called = std::cell::Cell::new(false);
    /// *map.get_or_insert_with("a", || {
    ///     called.set(true);
    ///     100
    /// });
    /// assert_eq!(map.get("a"), Some(&1));
    /// assert_eq!(called.get(), false);
    /// ```
    pub fn get_or_insert_with<K: AsBytes, F>(&mut self, key: K, f: F) -> &mut T
    where
        F: FnOnce() -> T,
    {
        if self.contains_key(&key) {
            self.get_mut(key).unwrap()
        } else {
            let value = f();
            self.insert(&key, value);
            self.get_mut(key).unwrap()
        }
    }

    /// Updates a value if the key exists.
    ///
    /// Returns `None` if the key exists and the value was updated, or `None` if the key doesn't exist.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    ///
    /// map.update("a", |v| *v *= 2);
    /// assert_eq!(map.get("a"), Some(&2));
    ///
    /// map.update("b", |v| *v *= 2);
    /// assert_eq!(map.get("b"), None);
    /// ```
    pub fn update<K: AsBytes, F>(&mut self, key: K, f: F) -> Option<T>
    where
        F: FnOnce(&mut T),
    {
        if let Some(value) = self.get_mut(key) {
            f(value);
            None
        } else {
            None
        }
    }

    /// Updates a value if the key exists, otherwise inserts a new value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map = TrieMap::new();
    /// map.insert("a", 1);
    ///
    /// map.update_or_insert("a", |v| *v *= 2, || 0);
    /// assert_eq!(map.get("a"), Some(&2));
    ///
    /// map.update_or_insert("b", |v| *v *= 2, || 3);
    /// assert_eq!(map.get("b"), Some(&3));
    /// ```
    pub fn update_or_insert<K: AsBytes, F, G>(&mut self, key: K, update: F, insert: G) -> &mut T
    where
        F: FnOnce(&mut T),
        G: FnOnce() -> T,
    {
        match self.entry(key) {
            Entry::Occupied(mut entry) => {
                update(entry.get_mut());
                entry.into_mut()
            }
            Entry::Vacant(entry) => entry.insert(insert()),
        }
    }

    /// Creates a new map with the given key-value pair added.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let map1 = TrieMap::new();
    /// let map2 = map1.inserted("a", 1);
    ///
    /// assert_eq!(map1.len(), 0);
    /// assert_eq!(map2.len(), 1);
    /// assert_eq!(map2.get("a"), Some(&1));
    /// ```
    pub fn inserted<K: AsBytes>(&self, key: K, value: T) -> Self
    where
        T: Clone,
    {
        let mut new_map = self.clone();
        new_map.insert(key, value);
        new_map
    }

    /// Creates a new map with the given key removed.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map1 = TrieMap::new();
    /// map1.insert("a", 1);
    /// map1.insert("b", 2);
    ///
    /// let map2 = map1.removed("a");
    ///
    /// assert_eq!(map1.len(), 2);
    /// assert_eq!(map2.len(), 1);
    /// assert!(!map2.contains_key("a"));
    /// assert!(map2.contains_key("b"));
    /// ```
    pub fn removed<K: AsBytes>(&self, key: K) -> Self
    where
        T: Clone,
    {
        let mut new_map = self.clone();
        new_map.remove(key);
        new_map
    }

    /// Creates a new map without any entries that match the given prefix.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map1 = TrieMap::new();
    /// map1.insert("apple", 1);
    /// map1.insert("application", 2);
    /// map1.insert("banana", 3);
    ///
    /// let map2 = map1.without_prefix("app");
    ///
    /// assert_eq!(map1.len(), 3);
    /// assert_eq!(map2.len(), 1);
    /// assert!(!map2.contains_key("apple"));
    /// assert!(!map2.contains_key("application"));
    /// assert!(map2.contains_key("banana"));
    /// ```
    pub fn without_prefix<K: AsBytes>(&self, prefix: K) -> Self
    where
        T: Clone,
    {
        let mut new_map = self.clone();
        new_map.remove_prefix_matches(prefix);
        new_map
    }

    /// Creates a new map with only entries that match the given prefix.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map1 = TrieMap::new();
    /// map1.insert("apple", 1);
    /// map1.insert("application", 2);
    /// map1.insert("banana", 3);
    ///
    /// let map2 = map1.with_prefix_only("app");
    ///
    /// assert_eq!(map1.len(), 3);
    /// assert_eq!(map2.len(), 2);
    /// assert!(map2.contains_key("apple"));
    /// assert!(map2.contains_key("application"));
    /// assert!(!map2.contains_key("banana"));
    /// ```
    pub fn with_prefix_only<K: AsBytes>(&self, prefix: K) -> Self
    where
        T: Clone,
    {
        let mut new_map = TrieMap::new();

        if let Some(matches) = self.find_node(prefix.as_bytes()) {
            let mut prefix_vec = prefix.as_bytes().to_vec();
            let mut pairs = Vec::new();

            self.collect_prefix_matches(matches, &mut prefix_vec, &mut pairs);

            for (key, value) in pairs {
                new_map.insert(key, value.clone());
            }
        }

        new_map
    }

    /// Returns an iterator over entries from both maps, preferring values from this map
    /// when keys exist in both maps.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// use triemap::AsBytes;
    /// let mut map1 = TrieMap::new();
    /// map1.insert("a", 1);
    /// map1.insert("b", 2);
    ///
    /// let mut map2 = TrieMap::new();
    /// map2.insert("b", 20);
    /// map2.insert("c", 30);
    ///
    /// let union: Vec<_> = map1.union(&map2).collect();
    /// assert_eq!(union.len(), 3);
    /// assert!(union.contains(&("a".as_bytes_vec(), &1)));
    /// assert!(union.contains(&("b".as_bytes_vec(), &2))); // Value from map1 is used
    /// assert!(union.contains(&("c".as_bytes_vec(), &30)));
    /// ```
    pub fn union<'a>(
        &'a self,
        other: &'a TrieMap<T>,
    ) -> impl Iterator<Item = (Vec<u8>, &'a T)> + 'a {
        // Start with all entries from this map
        self.iter()
            // Then add entries from other map that don't exist in this map
            .chain(other.iter().filter(move |(key, _)| !self.contains_key(key)))
    }

    /// Returns an iterator over the entries whose keys are present in both maps.
    ///
    /// The values from this map are used for the result.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// use triemap::AsBytes;
    /// let mut map1 = TrieMap::new();
    /// map1.insert("a", 1);
    /// map1.insert("b", 2);
    /// map1.insert("c", 3);
    ///
    /// let mut map2 = TrieMap::new();
    /// map2.insert("b", 20);
    /// map2.insert("c", 30);
    /// map2.insert("d", 40);
    ///
    /// let intersection: Vec<_> = map1.intersect(&map2).collect();
    /// assert_eq!(intersection.len(), 2);
    /// assert!(intersection.contains(&("b".as_bytes_vec(), &2))); // Values from map1
    /// assert!(intersection.contains(&("c".as_bytes_vec(), &3)));
    /// ```
    pub fn intersect<'a>(
        &'a self,
        other: &'a TrieMap<T>,
    ) -> impl Iterator<Item = (Vec<u8>, &'a T)> + 'a {
        self.iter().filter(move |(key, _)| other.contains_key(key))
    }

    /// Returns an iterator over the entries whose keys are in this map but not in the other map.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// use triemap::AsBytes;
    /// let mut map1 = TrieMap::new();
    /// map1.insert("a", 1);
    /// map1.insert("b", 2);
    /// map1.insert("c", 3);
    ///
    /// let mut map2 = TrieMap::new();
    /// map2.insert("b", 20);
    /// map2.insert("d", 40);
    ///
    /// let difference: Vec<_> = map1.difference(&map2).collect();
    /// assert_eq!(difference.len(), 2);
    /// assert!(difference.contains(&("a".as_bytes_vec(), &1)));
    /// assert!(difference.contains(&("c".as_bytes_vec(), &3)));
    /// ```
    pub fn difference<'a>(
        &'a self,
        other: &'a TrieMap<T>,
    ) -> impl Iterator<Item = (Vec<u8>, &'a T)> + 'a {
        self.iter().filter(move |(key, _)| !other.contains_key(key))
    }

    /// Returns an iterator over entries whose keys are in exactly one of the maps.
    ///
    /// For keys that only exist in this map, values from this map are used.
    /// For keys that only exist in the other map, values from the other map are used.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map1 = TrieMap::new();
    /// map1.insert("a", 1);
    /// map1.insert("b", 2);
    ///
    /// let mut map2 = TrieMap::new();
    /// map2.insert("b", 20);
    /// map2.insert("c", 30);
    ///
    /// let symmetric_difference: Vec<_> = map1.symmetric_difference(&map2).collect();
    /// assert_eq!(symmetric_difference.len(), 2);
    /// // Note: order of elements may vary
    /// assert!(symmetric_difference.iter().any(|(key, _)| key == "a".as_bytes()));
    /// assert!(symmetric_difference.iter().any(|(key, _)| key == "c".as_bytes()));
    /// assert!(!symmetric_difference.iter().any(|(key, _)| key == "b".as_bytes()));
    /// ```
    pub fn symmetric_difference<'a>(
        &'a self,
        other: &'a TrieMap<T>,
    ) -> impl Iterator<Item = (Vec<u8>, &'a T)> + 'a {
        self.difference(other).chain(other.difference(self))
    }

    /// Determines whether this map is a subset of another map.
    ///
    /// Returns `true` if all keys in this map are also in the other map.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map1 = TrieMap::new();
    /// map1.insert("a", 1);
    /// map1.insert("b", 2);
    ///
    /// let mut map2 = TrieMap::new();
    /// map2.insert("a", 10);
    /// map2.insert("b", 20);
    /// map2.insert("c", 30);
    ///
    /// assert!(map1.is_subset_of(&map2));
    /// assert!(!map2.is_subset_of(&map1));
    /// ```
    pub fn is_subset_of(&self, other: &TrieMap<T>) -> bool {
        self.iter().all(|(key, _)| other.contains_key(&key))
    }

    /// Determines whether this map is a proper subset of another map.
    ///
    /// Returns `true` if all keys in this map are in the other map,
    /// and the other map has at least one key not in this map.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map1 = TrieMap::new();
    /// map1.insert("a", 1);
    /// map1.insert("b", 2);
    ///
    /// let mut map2 = TrieMap::new();
    /// map2.insert("a", 10);
    /// map2.insert("b", 20);
    /// map2.insert("c", 30);
    ///
    /// assert!(map1.is_proper_subset_of(&map2));
    /// assert!(!map2.is_proper_subset_of(&map1));
    ///
    /// let mut map3 = TrieMap::new();
    /// map3.insert("a", 1);
    /// map3.insert("b", 2);
    ///
    /// assert!(!map1.is_proper_subset_of(&map3));
    /// ```
    pub fn is_proper_subset_of(&self, other: &TrieMap<T>) -> bool {
        self.len() < other.len() && self.is_subset_of(other)
    }

    /// Merges another map into this one.
    ///
    /// If a key exists in both maps, the value from the other map is used.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map1 = TrieMap::new();
    /// map1.insert("a", 1);
    /// map1.insert("b", 2);
    ///
    /// let mut map2 = TrieMap::new();
    /// map2.insert("b", 3);
    /// map2.insert("c", 4);
    ///
    /// map1.merge(&map2);
    ///
    /// assert_eq!(map1.get("a"), Some(&1));
    /// assert_eq!(map1.get("b"), Some(&3));
    /// assert_eq!(map1.get("c"), Some(&4));
    /// ```
    pub fn merge(&mut self, other: &TrieMap<T>)
    where
        T: Clone,
    {
        for (key, value) in other.iter() {
            self.insert(key.clone(), value.clone());
        }
    }

    /// Merges another map into this one using a custom function to resolve conflicts.
    ///
    /// If a key exists in both maps, the function is called with the key, this map's value, and
    /// the other map's value, and the result is used as the new value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use triemap::TrieMap;
    /// let mut map1 = TrieMap::new();
    /// map1.insert("a", 1);
    /// map1.insert("b", 2);
    ///
    /// let mut map2 = TrieMap::new();
    /// map2.insert("b", 3);
    /// map2.insert("c", 4);
    ///
    /// map1.merge_with(&map2, |_, v1, v2| v1 + v2);
    ///
    /// assert_eq!(map1.get("a"), Some(&1));
    /// assert_eq!(map1.get("b"), Some(&5)); // 2 + 3 = 5
    /// assert_eq!(map1.get("c"), Some(&4));
    /// ```
    pub fn merge_with<F>(&mut self, other: &TrieMap<T>, mut f: F)
    where
        F: FnMut(&[u8], &T, &T) -> T,
        T: Clone,
    {
        for (key, value) in other.iter() {
            if let Some(existing) = self.get(&key) {
                let merged_value = f(&key, existing, value);
                self.insert(key.clone(), merged_value);
            } else {
                self.insert(key.clone(), value.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests;
