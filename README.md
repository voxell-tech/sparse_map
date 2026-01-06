# Sparse Map

[![License](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](https://github.com/voxell-tech/sparse_map#license)
[![Crates.io](https://img.shields.io/crates/v/sparse_map.svg)](https://crates.io/crates/sparse_map)
[![Downloads](https://img.shields.io/crates/d/sparse_map.svg)](https://crates.io/crates/sparse_map)
[![Docs](https://docs.rs/sparse_map/badge.svg)](https://docs.rs/sparse_map/latest/sparse_map/)
[![CI](https://github.com/voxell-tech/sparse_map/workflows/CI/badge.svg)](https://github.com/voxell-tech/sparse_map/actions)
[![Discord](https://img.shields.io/discord/442334985471655946.svg?label=&logo=discord&logoColor=ffffff&color=7389D8&labelColor=6A7EC2)](https://discord.gg/Mhnyp6VYEQ)

A sparse map with stable generational keys. It is designed for
scenarios with frequent insertions and removals.

The crate is `no_std`-friendly and uses generational indices to
prevent use-after-free and slot reuse bugs.

## Example

```rust
use sparse_map::SparseMap;

let mut map = SparseMap::new();
let k1 = map.insert(42);

let k2 = map.scope(&k1, |map, value| {
    let k = map.insert(*value);
    *value += 2;

    k
})
.expect("Key should exist.");

assert_eq!(map.get(&k1), Some(&44));
assert_eq!(map.get(&k2), Some(&42));

map.remove(&k1);
assert_eq!(map.get(&k1), None);
```

## Join the community!

You can join us on the [Voxell discord server](https://discord.gg/Mhnyp6VYEQ).

## License

`sparse_map` is dual-licensed under either:

- MIT License ([LICENSE-MIT](LICENSE-MIT) or [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))

This means you can select the license you prefer!
This dual-licensing approach is the de-facto standard in the Rust ecosystem and there are [very good reasons](https://github.com/bevyengine/bevy/issues/2373) to include both.

