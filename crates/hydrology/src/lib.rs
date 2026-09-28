//! Water body and river stage of the Vernadsky planetary geography
//! generator.
//!
//! The stage fills the `water_bodies` and `rivers` sections of a
//! [`GeographicWorld`] whose grid carries water flags and whose biome
//! section is filled:
//!
//! - **Water bodies**: 4-neighbor flood fill over the water flags, with
//!   the map seamless along the x axis and the poles disconnected. A
//!   component touching a polar row is the world ocean; enclosed
//!   components split by area into seas and lakes. Identifiers derive
//!   from the component centroid through the stable identifier strategy
//!   (ADR-0003).
//! - **Rivers**: flow accumulation — cells sorted by height (descending,
//!   canonical tie-break by cell index), each passing its flow to the
//!   lowest of its 8 neighbors; cells draining at least the drainage
//!   threshold form the river network. The threshold is
//!   resolution-independent: the maximum of the absolute floor
//!   (`8.0`, the mapgen port value) and [`HydrologyParams`]'s
//!   `river_land_share` of the world's land cells — the share keeps
//!   the network consistent across grid resolutions, the floor keeps
//!   tiny worlds from dissolving it.
//!   River paths run from each network source down the flow graph to
//!   the sea; the mouth is the last land cell before the water.
//!   Identifiers anchor at the mouth. Converging rivers share the
//!   downstream cells.
//!
//! Ice cells produce no flow, and deserts evaporate half of the flow
//! passing through; both are resolved by stable name from the biome
//! registry. The stage consumes no randomness at all: identical worlds
//! produce identical water topology, and no existing stream shifts when
//! this stage comes online.
//!
//! Known v0 limits (documented, post-MVP refinements): rivers ending in
//! land pits never reach water and are dropped rather than forming
//! endorheic lakes; river surface stays land in the `is_water` flag.
//!
//! # Example
//!
//! ```
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     use vernadsky_climate::{generate as climate, ClimateConfig};
//!     use vernadsky_core::{seeded_world, RngStreams};
//!     use vernadsky_hydrology::{generate, HydrologyConfig};
//!
//!     let mut world = seeded_world(42);
//!     let params = ClimateConfig::default().to_params()?;
//!     climate(&mut world, &params, &RngStreams::new(42))?;
//!     vernadsky_biome::generate(&mut world, &RngStreams::new(42))?;
//!     let hydrology = HydrologyConfig::default().to_params()?;
//!     generate(&mut world, &hydrology)?;
//!
//!     assert!(!world.water_bodies.is_empty());
//!     Ok(())
//! }
//! ```

pub mod params;
pub mod rivers;
pub mod water;

use std::fmt;

use vernadsky_core::idgen::IdAssigner;
use vernadsky_core::schema::{
    GeographicWorld, HydrologyParams, NO_INDEX, RiverRecord, WaterBodyRecord,
};
use vernadsky_core::{Anchor, BiomeId, CellId};

pub use params::{HydrologyConfig, ParamsError};

/// Errors of the hydrology stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HydrologyError {
    /// The grid is empty or its cell storage does not match its declared
    /// size; the stage refuses to run on an inconsistent world.
    InvalidGrid,
    /// The world carries no biome section; ice and desert handling
    /// consumes it.
    MissingBiomes,
    /// The biome registry does not define a biome the stage relies on;
    /// the registry and the stage disagree.
    UnknownRegistryBiome(&'static str),
    /// Stable identifier generation failed.
    IdGeneration(vernadsky_core::IdGenError),
}

impl fmt::Display for HydrologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HydrologyError::InvalidGrid => {
                write!(f, "hydrology stage requires a consistent, non-empty grid")
            }
            HydrologyError::MissingBiomes => {
                write!(f, "hydrology stage requires a filled biome section")
            }
            HydrologyError::UnknownRegistryBiome(name) => {
                write!(f, "biome registry does not define {name:?}")
            }
            HydrologyError::IdGeneration(source) => write!(f, "hydrology stage: {source}"),
        }
    }
}

impl std::error::Error for HydrologyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HydrologyError::IdGeneration(source) => Some(source),
            _ => None,
        }
    }
}

/// Applies the hydrology stage to `world` in place.
///
/// Replaces the water body classification, the river network, and the
/// per-cell water body references. Pure with respect to `world`: no
/// randomness, no wall-clock, no environment.
pub fn generate(
    world: &mut GeographicWorld,
    params: &HydrologyParams,
) -> Result<(), HydrologyError> {
    let width = world.grid.width;
    let height = world.grid.height;
    let cell_count = width as usize * height as usize;
    if width == 0 || height == 0 || world.grid.cells.len() != cell_count {
        return Err(HydrologyError::InvalidGrid);
    }
    let biomes = world
        .biomes
        .biome
        .as_ref()
        .ok_or(HydrologyError::MissingBiomes)?;
    let ice = resolve_biome("Ice")?;
    let desert = resolve_biome("Desert")?;

    // --- Water bodies: classify, then assign stable identifiers in the
    // canonical anchor order. ---
    let components = water::components(&world.grid.cells, width, height);
    let mut anchored: Vec<(Anchor, &water::Component)> = components
        .iter()
        .map(|component| (component.anchor(), component))
        .collect();
    anchored.sort_by_key(|(anchor, _)| (anchor.y, anchor.x));

    let mut assigner = IdAssigner::new();
    let mut water_body_of_cell = vec![NO_INDEX; cell_count];
    let mut water_bodies = Vec::with_capacity(anchored.len());
    for (position, (anchor, component)) in anchored.iter().enumerate() {
        let id = assigner
            .water_body(anchor.x, anchor.y)
            .map_err(HydrologyError::IdGeneration)?;
        water_bodies.push(WaterBodyRecord {
            id,
            kind: component.kind(water::SEA_MIN_CELLS),
            anchor: anchor.clone(),
        });
        for &cell in &component.cells {
            water_body_of_cell[cell] = position as u32;
        }
    }

    // --- Rivers: accumulate flow, extract source-to-mouth paths. ---
    //
    // The river threshold scales with each land component's own size,
    // so every landmass — continent or island — carries a network
    // proportional to itself.
    let land_components = water::surface_components(&world.grid.cells, width, height, false);
    let mut cell_threshold = vec![0.0f64; cell_count];
    for cells in &land_components {
        let threshold = rivers::river_threshold(params.river_land_share, cells.len());
        for &index in cells {
            cell_threshold[index] = threshold;
        }
    }
    let river_config = rivers::RiverConfig { ice, desert };
    let river_paths = rivers::network(
        &world.grid.cells,
        biomes,
        width,
        height,
        river_config,
        &cell_threshold,
        &mut assigner,
    )?;

    // --- Hand over ---
    for cell in &mut world.grid.cells {
        cell.water_body = NO_INDEX;
    }
    for (index, cell) in world.grid.cells.iter_mut().enumerate() {
        cell.water_body = water_body_of_cell[index];
    }
    world.water_bodies = water_bodies;
    world.params.hydrology = Some(*params);
    world.params.params_version = 3;
    world.rivers = river_paths
        .into_iter()
        .map(|path| RiverRecord {
            id: path.id,
            mouth: path.mouth,
            path: path
                .cells
                .into_iter()
                .map(|index| CellId(index as u64))
                .collect(),
        })
        .collect();
    Ok(())
}

fn resolve_biome(name: &'static str) -> Result<BiomeId, HydrologyError> {
    vernadsky_biome::by_name(name).ok_or(HydrologyError::UnknownRegistryBiome(name))
}
