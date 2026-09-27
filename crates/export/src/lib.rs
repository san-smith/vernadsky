//! Derived suitability features of the Vernadsky planetary geography
//! generator.
//!
//! This crate derives the settlement-suitability facts of a partitioned
//! world: the starting natural potential of every land territory and
//! the neutral geographic facts — coastal exposure, river mouths, and
//! mountainous terrain — that consumers interpret as ports, estuaries,
//! and passes.
//!
//! The boundary of the generator holds here: the crate produces neutral
//! facts and scores, never game labels or bonuses. A "port" is the
//! consumer's reading of a coastal territory; an "estuary" is the
//! consumer's reading of a river mouth reaching the sea.
//!
//! # Enrichment
//!
//! [`enrich`] fills `TerritoryRecord::natural_potential` for every land
//! territory: the mean per-cell score (temperature comfort × humidity ×
//! lowland factor) scaled to permille. The formula and its constants are
//! documented on [`natural_potential`](crate::suitability) — see the
//! suitability module. The pass is idempotent and
//! consumes no randomness.
//!
//! # Example
//!
//! ```
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     use vernadsky_biome::generate as biomes;
//!     use vernadsky_climate::{generate as climate, ClimateConfig};
//!     use vernadsky_core::{seeded_world, RngStreams};
//!     use vernadsky_terrain::{generate as terrain, TerrainConfig};
//!     use vernadsky_export::{enrich, facts};
//!
//!     let mut world = seeded_world(42);
//!     let terrain_params = TerrainConfig::default().to_params()?;
//!     terrain(&mut world, &terrain_params, &RngStreams::new(42))?;
//!     let params = ClimateConfig::default().to_params()?;
//!     climate(&mut world, &params, &RngStreams::new(42))?;
//!     biomes(&mut world, &RngStreams::new(42))?;
//!     vernadsky_territory::generate(&mut world, 8, 2)?;
//!     enrich(&mut world)?;
//!
//!     for fact in facts(&world)? {
//!         assert!(fact.natural_potential.0 >= 0);
//!     }
//!     Ok(())
//! }
//! ```

pub mod suitability;

use std::fmt;

use vernadsky_core::quant::QuantError;
use vernadsky_core::schema::GeographicWorld;

pub use suitability::{TerritoryFacts, facts};

/// Errors of the derived-feature stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportError {
    /// The grid is empty or its cell storage does not match its declared
    /// size; the stage refuses to run on an inconsistent world.
    InvalidGrid,
    /// The world carries no climate fields; the potential consumes
    /// temperature and humidity.
    MissingClimate,
    /// The world carries no biome section; the mountain fact consumes
    /// the biome assignment.
    MissingBiomes,
    /// A computed value could not be quantized onto its lattice.
    Quantization(QuantError),
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::InvalidGrid => {
                write!(f, "export stage requires a consistent, non-empty grid")
            }
            ExportError::MissingClimate => {
                write!(f, "export stage requires a filled climate section")
            }
            ExportError::MissingBiomes => {
                write!(f, "export stage requires a filled biome section")
            }
            ExportError::Quantization(source) => write!(f, "export stage: {source}"),
        }
    }
}

impl std::error::Error for ExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ExportError::Quantization(source) => Some(source),
            _ => None,
        }
    }
}

/// Fills `natural_potential` of every land territory in `world`.
///
/// The value is the mean per-cell suitability score of the territory
/// scaled to permille (0…1000); water territories and unpartitioned
/// worlds are left untouched. Idempotent: a second call reproduces the
/// same values. See the crate documentation for the model.
pub fn enrich(world: &mut GeographicWorld) -> Result<(), ExportError> {
    let facts = suitability::facts(world)?;
    let scores = facts.iter().map(|fact| fact.natural_potential);
    for (territory, potential) in world.territories.iter_mut().zip(scores) {
        if !territory.is_water {
            territory.natural_potential = potential;
        }
    }
    Ok(())
}
