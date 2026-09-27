//! Climate generation stage of the Vernadsky planetary geography
//! generator: per-cell annual-mean temperature, relative humidity, and
//! annual precipitation.
//!
//! The stage fills the `climate` section of a [`vernadsky_core::GeographicWorld`] that
//! already carries a grid with heights and water flags. It is a port of
//! the climate pass of our earlier mapgen prototype; the numeric model
//! is re-derived in physical units (°C, %, mm/year) for the Vernadsky
//! lattices, and every hand-off to the world is quantized through
//! `vernadsky_core::quant`.
//!
//! # Model
//!
//! Rows map to latitude: the `lat_factor` of a row runs from `0` at the
//! equator row to `1` at the pole rows. Temperature per cell:
//!
//! ```text
//! lat_factor = |(y + 0.5) / height − 0.5| · 2
//! T_base     = T_EQUATOR + (T_POLE − T_EQUATOR) · lat_factor ^ latitude_exponent
//! T          = T_base
//!            + NOISE_SHARE · (T_EQUATOR − T_POLE) · noise(cell)
//!            + temperature_offset · (1 + lat_factor · polar_amplification)
//!            − LAPSE_RATE · max(0, height_m)                     (land cells)
//!            − OCEAN_DAMPING_MAX · min(1, depth_m / OCEAN_DAMPING_DEPTH)  (water cells)
//! T          ← clamp(T, −90 °C, +60 °C), quantized to `TempDeciC`
//! ```
//!
//! The noise is 3D `OpenSimplex2` (FastNoiseLite) sampled on a cylinder
//! along the x axis, so the field wraps seamlessly from the western to
//! the eastern edge of the grid.
//!
//! Humidity and precipitation come from a per-row moisture march along a
//! simplified trade-wind circulation (westerlies outside the
//! `0.3..0.7` latitude band, trade winds inside it):
//!
//! ```text
//! ocean cells:  moisture ← min(1, moisture + evaporation); humidity = 1
//! land cells:   rise = max(0, height_norm − height_norm_of_previous_cell_in_wind_direction)
//!               precipitation = moisture · (0.02 + 8 · rise), bounded by the moisture
//!               moisture ← moisture − precipitation
//!               humidity = precipitation · 20 + humidity_offset        (normalized units)
//! humidity ← box-smooth(humidity, radius 3), clamp to [0, 1], quantized to `HumidDeciPct`
//! precipitation ← quantized to mm/year via `PRECIP_MM_SCALE`
//! ```
//!
//! # Constants and sources
//!
//! | Constant | Value | Source |
//! | --- | --- | --- |
//! | `T_EQUATOR` | 27 °C | approximate annual-mean surface temperature near the equator |
//! | `T_POLE` | −25 °C | approximate annual-mean surface temperature at the poles before offset amplification |
//! | `NOISE_SHARE` | 0.2 | port of the mapgen 80/20 latitude/noise blend, as a fraction of the equator–pole range |
//! | lapse rate | 6.5 °C/km | ICAO Standard Atmosphere tropospheric lapse rate |
//! | ocean damping | up to 5 °C over the first 1000 m of depth | port of the mapgen water-cooling term |
//! | precipitation scale | 3000 mm/year | order of magnitude of annual precipitation on windward coasts |
//! | circulation, evaporation, orographic factors | port values | `mapgen/src/climate.rs` (our earlier prototype), structure kept, units re-derived |
//!
//! The heights of the world are physical meters; slopes enter the
//! orographic term normalized by a 4000 m reference (high-mountain
//! relief). Fixture worlds built by `vernadsky_core::synthetic` are much
//! lower and therefore produce mostly baseline precipitation — by
//! construction, not by accident.
//!
//! # Randomness
//!
//! The only consumer of chance is the temperature noise: its seed is
//! the leading bytes of the stage-owned [`STREAM_TEMPERATURE`] stream
//! (`rng-streams-v1`). Introducing this stage does not shift the output
//! of any other stream, and regenerating the world reproduces the field
//! byte for byte.
//!
//! # Example
//!
//! ```
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     use vernadsky_climate::{generate, ClimateConfig};
//!     use vernadsky_core::{seeded_world, RngStreams};
//!
//!     let mut world = seeded_world(42);
//!     let params = ClimateConfig::default().to_params()?;
//!     generate(&mut world, &params, &RngStreams::new(42))?;
//!
//!     let temperature = world.climate.temperature.expect("filled by the stage");
//!     assert_eq!(temperature.len(), world.grid.cells.len());
//!     assert_eq!(world.params.params_version, 1);
//!     Ok(())
//! }
//! ```

pub mod params;
pub mod stage;

pub use params::{ClimateConfig, ParamsError};
pub use stage::{
    ClimateError, SMOOTHING_RADIUS, STREAM_TEMPERATURE, TEMP_MAX_C, TEMP_MIN_C, generate,
};
