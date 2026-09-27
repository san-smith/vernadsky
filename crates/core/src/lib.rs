//! Vernadsky core: seeded planetary geography generation.
//!
//! This crate owns the `GeographicWorld` schema (version 1) and its
//! canonical binary export format. Generation stages are being ported
//! incrementally; the schema they fill is already the frozen contract.
//!
//! # Layout
//!
//! - [`quant`]: integer lattices for physical quantities; the only place
//!   where floats are converted to exported values.
//! - [`id`]: typed identifier spaces (cells, territories, regions, water
//!   bodies, rivers, biomes) with reserved zero values.
//! - [`idgen`]: the `content-hash-v1` derivation strategy — FNV-1a 64 over
//!   a canonical little-endian anchor encoding, deterministic collision
//!   probing.
//! - [`schema`]: the `GeographicWorld` data contract.
//! - [`mod@format`]: canonical binary serialization, decoding, and validation.
//! - [`synthetic`]: a deterministic minimal world constructor used by the
//!   golden round-trip tests.
//!
//! # Invariants
//!
//! - Generation is a pure function of a seed plus parameters: identical
//!   inputs produce identical output on every run and every supported
//!   platform. The generation path uses no wall-clock time, thread
//!   scheduling, or external entropy.
//! - Results never depend on hash-map iteration order; ordered structures
//!   and stable identifiers are used instead.
//! - Physical quantities cross stage boundaries and the export only as
//!   lattice values ([`quant`]); identifiers are derived only through
//!   [`idgen`].
//! - The crate contains no `unsafe` code; this is enforced by the workspace
//!   lints, not by convention.
//!
//! # Round-trip example
//!
//! ```
//! use vernadsky_core::{synthetic, GeographicWorld};
//!
//! let world = synthetic::minimal_world();
//! let bytes = world.to_bytes();
//!
//! let decoded = GeographicWorld::from_bytes(&bytes).expect("valid export");
//! assert_eq!(decoded, world);
//! assert_eq!(decoded.to_bytes(), bytes);
//! ```

pub mod format;
pub mod id;
pub mod idgen;
pub mod quant;
pub mod rng;
pub mod schema;
pub mod synthetic;

pub use id::{BiomeId, CellId, RegionId, RiverId, TerritoryId, WaterBodyId};
pub use idgen::{ID_STRATEGY, IdAssigner, IdGenError};
pub use quant::{CentiScalar, LATTICE_REGISTRY_VERSION, QuantError};
pub use rng::{RNG_STRATEGY, RngStreams};
pub use schema::{
    Anchor, BiomeSection, CellRecord, ClimateParams, ClimateSection, Connectivity, ErosionParams,
    GenerationParams, GeographicWorld, GridSection, NO_INDEX, RegionRecord, RiverRecord,
    SCHEMA_VERSION, TerrainParams, TerritoryRecord, WaterBodyKind, WaterBodyRecord,
};
pub use synthetic::{minimal_world, seeded_world, skeleton_world};
