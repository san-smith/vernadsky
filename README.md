# Vernadsky

Reusable Rust generator for physical planetary geography. The planned output covers relief, water, climate, biomes, and stable geographic identifiers. It does not define factions, game provinces, or simulation rules.

This repository is a Cargo workspace. The generator core lives in `crates/core` (`vernadsky-core`) and defines no public API yet; additional crates (`climate`, `hydrology`, `export`) are extracted when there is real content, never as placeholder APIs. Public format and API documentation will live here once the data contract is designed.

Run `cargo test` from this directory once a Rust toolchain is installed.

## License

Dual-licensed under the MIT license or the Apache License 2.0; see `LICENSE-MIT` and `LICENSE-APACHE`.
