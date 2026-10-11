# Vernadsky

Vernadsky is a reusable Rust generator for physical planetary geography. Its output covers relief, water, climate, biomes, and stable geographic identifiers, exported in a versioned schema. It is a standalone library: it knows nothing about factions, economies, game provinces, simulation rules, or any simulation framework.

Status: pre-release (0.1.0, not published). The `GeographicWorld` schema and the canonical binary export format are frozen contracts documented in the `vernadsky-core` rustdoc; the format reader stays backward-compatible with earlier schema and parameter versions.

## Workspace

| Crate                                     | Content                                                                                                                                                      |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| [`vernadsky-core`](crates/core)           | The `GeographicWorld` schema, integer lattices for physical quantities, typed stable identifiers, named RNG streams, and the canonical binary export format. |
| [`vernadsky-terrain`](crates/terrain)     | Heightmap stage: fBm noise on an x-seamless cylinder, island effect, smoothing, normalization to a land-ratio target, optional erosion; relief in meters.    |
| [`vernadsky-climate`](crates/climate)     | Climate stage: annual-mean temperature, relative humidity, and precipitation in physical units (°C, %, mm/year).                                             |
| [`vernadsky-biome`](crates/biome)         | Biome classification stage: an ordered rule table over water, mountains, and climate, with a versioned biome registry (`biomes-v1`).                         |
| [`vernadsky-hydrology`](crates/hydrology) | Water stage: ocean, seas, and lakes from flood fill, plus a river network from flow accumulation.                                                            |
| [`vernadsky-territory`](crates/territory) | Territory partition stage: land and water territories, grouped into continent and sea-basin regions.                                                         |
| [`vernadsky-export`](crates/export)       | Derived natural potential per land territory and neutral geographic facts (coastal exposure, river mouths, mountains) as deterministic queries.              |
| [`vernadsky-tools`](crates/tools)         | Debug CLI: PNG layer rendering, export dumping, export validation. Not part of the generator contract.                                                       |

The full pipeline runs terrain → climate → biomes → hydrology → territory partition → export enrichment over a shared `GeographicWorld`. Generation is a pure function of the seed, the stage parameters, and stage-owned named RNG streams: identical inputs produce identical output on every run and platform. Territory and cell identifiers are part of the public contract; changing how they are derived is a breaking schema change.

## Building and testing

The toolchain is pinned in `rust-toolchain.toml` (1.92.0). Run from the repository root:

```bash
cargo build
cargo test
cargo fmt --check
cargo clippy -- -D warnings
```

Golden tests pin content hashes of full exports for a fixed seed set; a failure means accidental nondeterminism or schema drift.

## Debug tools

```bash
cargo run -p vernadsky-tools -- render --seed 42 --zoom 4 --out map
cargo run -p vernadsky-tools -- dump --seed 42 --out world.gwb
cargo run -p vernadsky-tools -- validate --input world.gwb
```

`render` rasterizes the world's layers (`height`, `water`, `rivers`, `temperature`, `humidity`, `precipitation`, `biome`, `territory`, `regions`, `potential`) into PNG maps, one file per layer; `dump` builds a world from a seed and writes it as a canonical export file; `validate` checks an export file and prints a summary. Worlds can also be loaded from an export file instead of a seed (`render --input world.gwb`), and each generation stage accepts TOML overrides through `--terrain-config`, `--config` (climate), and `--hydrology-config`.

## Scope and conventions

Settlement suitability, political geography, and any gameplay semantics belong to consumers; physical geography stops at natural potential. `AGENTS.md` documents the scope, the core invariants (seeded reproducibility, deterministic computation, stable identifiers, versioned export), the dependency policy, and the contribution conventions.

## License

Dual-licensed under the MIT license or the Apache License 2.0; see `LICENSE-MIT` and `LICENSE-APACHE`.
