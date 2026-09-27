//! Debug visualization and export helpers for Vernadsky worlds.
//!
//! This is the library half of the `vernadsky-tools` debug CLI. It
//! builds worlds from seeds through the public generator API, rasterizes
//! the per-cell layers of a [`GeographicWorld`] into RGB buffers, and
//! writes canonical export files. It is a diagnostic instrument: the
//! rendered images are debug-grade, and nothing here is part of the
//! generator contract.
//!
//! All layer values are consumed through the typed lattices of
//! `vernadsky-core`; no generator internals are accessed.

pub mod render;

use anyhow::Context;

use vernadsky_core::schema::ClimateParams;
use vernadsky_core::{GeographicWorld, RngStreams};

/// Grid dimensions of the tool's worlds (the fixture pipeline size).
pub const FIXTURE_WIDTH: u32 = 48;
pub const FIXTURE_HEIGHT: u32 = 32;
/// Land territory count of the tool's worlds.
pub const FIXTURE_TERRITORY_COUNT: usize = 12;
/// Water territory count of the tool's worlds.
pub const FIXTURE_WATER_TERRITORY_COUNT: usize = 8;

/// Builds a real-pipeline world for `seed`: skeleton grid, terrain with
/// erosion, then — when `climate` is `Some` — the climate, biome,
/// hydrology, and territory partition stages.
///
/// This mirrors the current generation pipeline; the resulting world is
/// the same one the export files carry.
pub fn build_world(seed: u64, climate: Option<&ClimateParams>) -> anyhow::Result<GeographicWorld> {
    let mut world = vernadsky_core::skeleton_world(FIXTURE_WIDTH, FIXTURE_HEIGHT, seed);
    let terrain_params = vernadsky_terrain::TerrainConfig::default()
        .to_params()
        .context("default terrain config")?;
    vernadsky_terrain::generate(&mut world, &terrain_params, &RngStreams::new(seed))
        .context("terrain stage failed")?;
    if let Some(params) = climate {
        vernadsky_climate::generate(&mut world, params, &RngStreams::new(seed))
            .context("climate stage failed")?;
        vernadsky_biome::generate(&mut world, &RngStreams::new(seed))
            .context("biome stage failed")?;
        vernadsky_hydrology::generate(&mut world).context("hydrology stage failed")?;
        vernadsky_territory::generate(
            &mut world,
            FIXTURE_TERRITORY_COUNT,
            FIXTURE_WATER_TERRITORY_COUNT,
        )
        .context("territory stage failed")?;
    }
    Ok(world)
}

/// Parses climate parameters from a TOML document.
pub fn climate_params_from_toml(source: &str) -> anyhow::Result<ClimateParams> {
    vernadsky_climate::params::from_toml_str(source).context("invalid climate config")
}
