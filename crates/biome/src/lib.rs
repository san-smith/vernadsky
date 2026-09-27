//! Biome classification stage of the Vernadsky planetary geography
//! generator.
//!
//! The stage fills the `biomes` section of a [`vernadsky_core::GeographicWorld`] whose
//! climate section is already filled: every cell receives a
//! [`vernadsky_core::BiomeId`] resolved through the versioned biome registry.
//!
//! # The biome set is evolvable
//!
//! The initial set of sixteen biomes is a port of our earlier mapgen
//! prototype — a convenient starting configuration, **not** a fixed
//! contract. The set evolves inside the registry:
//!
//! - **Adding** a biome: a new entry with a fresh, never-before-used
//!   identifier.
//! - **Removing or splitting** a biome (e.g. Desert splitting into
//!   semi-desert and dry desert): the identifier is *deprecated* — the
//!   classification stops producing it and the identifier is never
//!   reassigned, but the registry entry stays so that existing exports
//!   remain interpretable.
//! - Consumers must not enumerate biomes: the schema, the export
//!   format, and downstream tools treat `BiomeId` values as opaque.
//!   Unknown identifiers are valid — a file written by a newer
//!   generator stays readable for an older consumer, which renders
//!   unknown values with a fallback.
//!
//! Registry version [`BIOME_REGISTRY_VERSION`] changes only when the
//! meaning of an existing identifier is redefined — which the policy
//! above is designed to avoid entirely.
//!
//! # Model
//!
//! Classification walks three priority tiers of ordered rule tables
//! ([`classify`] documents the exact tables):
//!
//! 1. **Water** cells (the schema's `is_water` flag, not a sea-level
//!    comparison): freezing by water temperature, then depth.
//! 2. **Mountains** (land): by height first, temperature second —
//!    highlands take priority over climate, realistic for alpine
//!    terrain.
//! 3. **Climate** (land): ordered temperature/humidity rules of the
//!    Whittaker family, with every threshold compared on integer
//!    lattice steps (ADR-0004 §5.5).
//!
//! All thresholds are physical values on the Vernadsky lattices (°C,
//! %, m); the correspondence to the normalized mapgen values is noted
//! in the rule table comments.
//!
//! # Boundary dither
//!
//! Hard thresholds on smooth fields draw geometric bands, so the
//! climate rules take a per-cell dither: 3D OpenSimplex2 noise
//! (FastNoiseLite) sampled on a cylinder for x-seamlessness, seeded
//! from the stage-owned [`STREAM_BOUNDARY`] stream. The noise is
//! quantized into lattice steps before entering the comparisons, so
//! every decision stays integer. The amplitude (±2 °C, ±5 % humidity)
//! is much smaller than the mapgen prototype's, because the climate
//! fields already carry cell-level variation.
//!
//! # Example
//!
//! ```
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     use vernadsky_biome::generate;
//!     use vernadsky_climate::{generate as climate, ClimateConfig};
//!     use vernadsky_core::{seeded_world, RngStreams};
//!
//!     let mut world = seeded_world(42);
//!     let params = ClimateConfig::default().to_params()?;
//!     climate(&mut world, &params, &RngStreams::new(42))?;
//!     generate(&mut world, &RngStreams::new(42))?;
//!
//!     let biomes = world.biomes.biome.expect("filled by the stage");
//!     assert_eq!(biomes.len(), world.grid.cells.len());
//!     Ok(())
//! }
//! ```

pub mod registry;
pub mod stage;

pub use registry::{BIOME_REGISTRY_VERSION, BiomeCategory, BiomeEntry, by_name, entry};
pub use stage::{BiomeError, CellInput, Dither, STREAM_BOUNDARY, classify, generate};
