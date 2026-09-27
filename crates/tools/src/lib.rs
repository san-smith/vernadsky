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
use vernadsky_core::{GeographicWorld, RngStreams, seeded_world};

/// Builds a synthetic world for `seed`, optionally applying the climate
/// stage (the default configuration unless overridden).
///
/// This mirrors the current generation pipeline; the resulting world is
/// the same one the export files carry.
pub fn build_world(seed: u64, climate: Option<&ClimateParams>) -> anyhow::Result<GeographicWorld> {
    let mut world = seeded_world(seed);
    if let Some(params) = climate {
        vernadsky_climate::generate(&mut world, params, &RngStreams::new(seed))
            .context("climate stage failed")?;
    }
    Ok(world)
}

/// Parses climate parameters from a TOML document.
pub fn climate_params_from_toml(source: &str) -> anyhow::Result<ClimateParams> {
    vernadsky_climate::params::from_toml_str(source).context("invalid climate config")
}
