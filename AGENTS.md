# AGENTS.md — Vernadsky

Vernadsky is a reusable Rust generator for physical planetary geography. Its output covers relief, water, climate, biomes, and stable geographic identifiers, exported in a versioned schema. It is a standalone library: it knows nothing about factions, economies, game provinces, simulation rules, or any simulation framework.

## Status

Cargo workspace with `crates/core` (`vernadsky-core`), `crates/terrain` (`vernadsky-terrain`), `crates/climate` (`vernadsky-climate`), `crates/biome` (`vernadsky-biome`), `crates/hydrology` (`vernadsky-hydrology`), and `crates/tools` (`vernadsky-tools`, the debug CLI); further crates (`hydrology`, `export`) are extracted when there is real content, never as placeholder APIs. Schema `GeographicWorld` v0 with the canonical binary export format lives in `vernadsky-core`; see the crate rustdoc. Keep the root buildable and testable at every commit.

Terminology: the generator's base partition unit is a **Territory** — a neutral geographic unit without gameplay semantics. Game concepts such as Province, State, or SeaZone are consumer-side constructs built from territories and cells.

## Commands

Run from the repository root:

```bash
cargo build
cargo test
cargo fmt --check
cargo clippy -- -D warnings
```

## Core invariants

- **Seeded reproducibility.** Generation is a pure function of seed plus parameters: same inputs must produce identical output on every run and every supported platform. No wall-clock time, thread scheduling, external entropy, or uninitialized memory in the generation path.
- **Deterministic computation.** Same rules as for any numeric simulation code: no `HashMap`/`HashSet` iteration order influencing results (use `BTreeMap`, sorted structures, or stable IDs), no platform-dependent floating-point without a documented policy on rounding and precision.
- **Stable identifiers.** Territory and cell identifiers are part of the public contract; consumers correlate data across exports. Changing how IDs are derived is a breaking schema change and needs a version bump and a migration note.
- **Versioned export.** Exported data carries a schema version and the generation parameters needed to reproduce it. Never change the meaning of an existing schema version in place.

## Scope and dependencies

- This repository builds and tests standalone. Do not add dependencies on sibling checkouts; external dependencies come from crates.io or pinned Git revisions only, and no committed `[patch]` sections.
- Licenses: dual `MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`). Keep `publish = false` until publication is explicitly decided.
- Commit `Cargo.lock` and update it together with dependency changes; it does not constrain library consumers but keeps CI builds reproducible.
- Settlement suitability, political geography, and any gameplay semantics belong to consumers. Physical geography stops at natural potential; if a feature needs factions or game rules, the boundary is wrong — stop and push it out.

## Conventions

- Contributions and issue discussion in English.
- Public data structures and the export format get rustdoc with round-trip examples before stabilization; undocumented public items are a defect.
- Golden tests are mandatory for the generator: a fixed set of seeds whose full outputs are hashed and compared on every run, so accidental nondeterminism or schema drift fails CI.
