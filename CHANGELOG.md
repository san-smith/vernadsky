# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Climate generation stage in a new `vernadsky-climate` crate: per-cell annual-mean temperature, relative humidity, and precipitation, ported from the mapgen prototype and re-derived in physical units (°C, %, mm/year) for the Vernadsky lattices. FastNoiseLite temperature noise is seeded from the stage-owned `climate.temperature` stream; formulas, constants, and their sources are documented in the crate rustdoc; a stage-local golden manifest pins the exports for the core seed set.
- Climate generation parameters in the export contract (params version 1): `GenerationParams` carries stage parameters as canonical integer lattices (`ClimateParams`), the format writer derives the params version from their presence, and the reader accepts versions 0 and 1 while rejecting unknown ones. Version 0 exports remain byte-identical and readable.
- `RngStreams::stream_bytes` in `vernadsky-core`: leading bytes of a named stream without importing the RNG stack.
- Golden seed manifest in `vernadsky-core` tests: a fixed seed set (including boundary values) with pinned content hashes of full exports, a hash sensitivity canary, and a documented regeneration procedure. The synthetic world constructor is now seed-parameterized through its `synthetic.*` RNG streams, making it the reference consumer of the stream strategy.
- Named RNG streams in `vernadsky-core` (`rng-streams-v1`): ChaCha8 streams derived from the world seed and a stage-owned stream name, independent and re-entrant, with locality across stream sets. First runtime dependency: `rand_chacha`.
- `GeographicWorld` schema v0 in `vernadsky-core`: the frozen data contract with typed lattice values and stable identifiers (`content-hash-v1`), a canonical little-endian binary export format with content hashing and a validating reader, and a deterministic synthetic world constructor backing golden round-trip tests.
- Bootstrap crate scaffold: package metadata, dual MIT or Apache-2.0 license, and a committed `Cargo.lock` for reproducible builds.
- CI on GitHub Actions: rustfmt, clippy, rustdoc, and tests on a pinned toolchain, plus a guard against references to private repositories.
- `AGENTS.md` describing scope, core invariants, and dependency policy.

### Changed

- Restructured the repository into a Cargo workspace with a virtual root manifest: the bootstrap crate moved to `crates/core` as `vernadsky-core`, inheriting version, edition, license, MSRV, and publish policy from workspace metadata, with `unsafe_code = "forbid"` applied workspace-wide. No public API existed before, so nothing breaks.
- Adopted Rust edition 2024.
