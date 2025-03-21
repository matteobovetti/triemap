# TrieMap

A Rust implementation of a map data structure backed by a trie (prefix tree).

## Features

- Fast key lookups with O(k) complexity where k is the key length
- Prefix-based operations (matching keys with a common prefix)
- Intuitive API similar to Rust's standard collections
- Full iterator support
- Entry API for in-place updates

## Documentation

Full API documentation is available at: https://ekinimo.github.io/triemap

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
triemap = { git = "https://github.com/ekinimo/triemap" }
```

### Basic Operations

```rust
use triemap::TrieMap;

// Create a new map
let mut map = TrieMap::new();

// Insert key-value pairs
map.insert("apple", 1);
map.insert("banana", 2);
map.insert("cherry", 3);

// Check if a key exists
assert!(map.contains_key("apple"));
assert!(!map.contains_key("grape"));

// Get a value
assert_eq!(map.get("banana"), Some(&2));

// Update a value
map.insert("apple", 10);
assert_eq!(map.get("apple"), Some(&10));

// Remove a value
assert_eq!(map.remove("cherry"), Some(3));
assert_eq!(map.get("cherry"), None);
```

### Prefix Operations

One of TrieMap's strengths is working with key prefixes:

```rust
use triemap::TrieMap;

let mut map = TrieMap::new();
map.insert("apple", 1);
map.insert("application", 2);
map.insert("banana", 3);

// Check if any keys start with a prefix
assert!(map.starts_with("app"));

// Get all key-value pairs with a certain prefix
let matches = map.get_prefix_matches("app");
assert_eq!(matches.len(), 2);

// Iterate over keys with a prefix
for key in map.prefix_keys("app") {
    println!("Key: {}", String::from_utf8_lossy(&key));
}

// Remove all keys with a prefix
let removed = map.remove_prefix_matches("app");
assert_eq!(removed.len(), 2);
```

### Entry API

```rust
use triemap::{TrieMap, Entry};

let mut map = TrieMap::new();

// Insert a value if the key doesn't exist
map.entry("a").or_insert(1);

// Update a value if the key exists
match map.entry("a") {
    Entry::Occupied(mut entry) => {
        *entry.get_mut() += 10;
    }
    Entry::Vacant(_) => {}
}

// Or more concisely:
*map.entry("a").or_insert(0) += 5;
```

### Iterators

```rust
use triemap::TrieMap;

let mut map = TrieMap::new();
map.insert("a", 1);
map.insert("b", 2);
map.insert("c", 3);

// Iterate over key-value pairs
for (key, value) in &map {
    println!("{}: {}", String::from_utf8_lossy(&key), value);
}

// Iterate over keys
for key in map.keys() {
    println!("Key: {}", String::from_utf8_lossy(&key));
}

// Iterate over values
for value in map.values() {
    println!("Value: {}", value);
}

// Mutable iteration
for value in map.values_mut() {
    *value *= 2;
}
```

### Set Operations

```rust
use triemap::TrieMap;

let mut map1 = TrieMap::new();
map1.insert("a", 1);
map1.insert("b", 2);

let mut map2 = TrieMap::new();
map2.insert("b", 20);
map2.insert("c", 30);

// Intersection
let intersection = map1.intersect_ref(&map2);
assert_eq!(intersection.len(), 1);
assert_eq!(intersection.get("b"), Some(&2));

// Difference
let difference = map1.difference_ref(&map2);
assert_eq!(difference.len(), 1);
assert_eq!(difference.get("a"), Some(&1));

// Union
let union = map1.clone().union(map2.clone());
assert_eq!(union.len(), 3);
```

## Contributing

Contributions are welcome! Here are some ways you can contribute:

- Improve documentation
- Add new features
- Fix bugs
- Optimize performance
- Add more tests

Please feel free to submit issues and pull requests.

## License

This project is licensed under the MIT License - see the LICENSE file for details.
