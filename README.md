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

`render` rasterizes the world's layers (`height`, `water`, `rivers`, `temperature`, `humidity`, `precipitation`, `biome`, `territory`, `regions`, `potential`) into PNG maps, one file per layer; `dump` builds a world from a seed and writes it as a canonical export file; `validate` checks an export file and prints a summary, including the land share and the land components. Worlds can also be loaded from an export file instead of a seed (`render --input world.gwb`), and each generation stage accepts TOML overrides through `--terrain-config`, `--config` (climate), and `--hydrology-config`. `--water-cells N` replaces `--water-count` with a target area per water territory.

### Shaping a world

The terrain stage exposes a recipe surface through which a consumer shapes world types without touching the stages:

| `TerrainConfig` field | Default | Domain     | Effect                                                                                                          |
| --------------------- | ------- | ---------- | --------------------------------------------------------------------------------------------------------------- |
| `land_ratio`          | `0.3`   | 0.05–0.95  | Share of land cells: the sea level is placed to meet it (within 1.5 percentage points at any grid).              |
| `features_across`     | `1.5`   | 0.5–8      | Base features around the map: few give one landmass, many give scattered continents; resolution-independent.     |
| `island_density`      | `0.5`   | 0–3        | Island lift over the lowlands: subtle up to 1, island chains out of the deep ocean around 2, broken continents toward 3. |
| `land_hypsometry`     | `1.0`   | 1–4        | Exponent of the land elevation curve: 1 spreads the land evenly up to 6000 m, about 3 crowds it into lowlands under a few ranges (Earth's hypsometric curve); the coast stays. |
| `warp_strength`       | `0.0`   | 0–0.3      | Domain warp reach, in map widths: bends the coastlines into peninsulas, gulfs, and island chains around solid cores instead of rounded blobs (about 0.06 reads as natural; strong reaches smear the land along the flow). |
| `warp_features`       | `6.0`   | 1–12       | Features of the warp field around the map: few bend whole continents, many fray the coasts. |
| `ridge_strength`      | `0.0`   | 0–1        | Height of the ridged ranges: long, narrow mountain chains over the land and the shallows. |
| `ridge_features`      | `5.0`   | 1–12       | Features of the ranges around the map. |

The climate stage (`temperature_offset_c`, `polar_amplification`, `humidity_offset`) moves temperature and moisture, and with them the biomes and the natural potential. Water territories can follow a target area (`WaterSplit::CellsPerTerritory` in `vernadsky-territory`, `build_world_with` in `vernadsky-tools`), which keeps them commensurate with the land territories at any land share and resolution and spreads their seeds evenly over the sea (Z-order sampling relaxed toward the centroids), so the zones come out compact and of even size; `land_components` reports the land structure for consumers that judge connectivity by their own thresholds.

Stage changes are inspected at release resolutions: the structure test runs the pipeline at 512² and 1024² over the recipe surface and prints the calibration table.

```bash
cargo test --release -p vernadsky-tools --test recipe_structure -- --ignored --nocapture
```

## Scope and conventions

Settlement suitability, political geography, and any gameplay semantics belong to consumers; physical geography stops at natural potential. `AGENTS.md` documents the scope, the core invariants (seeded reproducibility, deterministic computation, stable identifiers, versioned export), the dependency policy, and the contribution conventions.

## License

Dual-licensed under the MIT license or the Apache License 2.0; see `LICENSE-MIT` and `LICENSE-APACHE`.
