//! The climate stage: temperature, humidity, and annual precipitation
//! over an existing [`vernadsky_core::GeographicWorld`].
//!
//! The formulas, constants, and their sources are documented at the
//! crate level. Computation happens in `f64` (the FastNoiseLite API
//! speaks `f32` and its sample coordinates are narrowed deliberately);
//! every value handed to the world is quantized through
//! `vernadsky_core::quant`, so the stage output is byte-identical across
//! runs on the reference platform.

use std::fmt;

use fastnoise_lite::{FastNoiseLite, NoiseType};
use vernadsky_core::RngStreams;
use vernadsky_core::quant::{HumidDeciPct, PrecipMmYr, QuantError, TempDeciC};
use vernadsky_core::schema::{ClimateParams, ClimateSection, GenerationParams, GeographicWorld};

/// Stream name of the climate noise. The seed of the FastNoiseLite
/// generator is derived from the leading bytes of this stream.
pub const STREAM_TEMPERATURE: &str = "climate.temperature";

/// Noise frequency of the temperature field (port value of mapgen).
const NOISE_FREQUENCY: f32 = 0.005;

/// Annual-mean surface temperature near the equator, in °C.
pub(crate) const T_EQUATOR_C: f64 = 27.0;
/// Annual-mean surface temperature at the poles before offset
/// amplification, in °C.
pub(crate) const T_POLE_C: f64 = -25.0;
/// Share of the equator–pole range contributed by the noise (port of
/// the mapgen 80/20 latitude/noise blend).
pub(crate) const NOISE_SHARE: f64 = 0.2;
/// Physical bounds of the temperature field, in °C: the extremes of
/// observed surface climate. Clamping here is part of the model.
pub const TEMP_MIN_C: f64 = -90.0;
pub const TEMP_MAX_C: f64 = 60.0;
/// ICAO Standard Atmosphere lapse rate, in °C per meter.
pub(crate) const LAPSE_RATE_C_PER_M: f64 = 0.0065;
/// Deepest-ocean temperature damping, in °C, reached at
/// [`OCEAN_DAMPING_DEPTH_M`] (port of the mapgen water-cooling term).
pub(crate) const OCEAN_DAMPING_MAX_C: f64 = 5.0;
pub(crate) const OCEAN_DAMPING_DEPTH_M: f64 = 1000.0;
/// Inner and outer bound of the trade-wind latitude band (port values).
pub(crate) const TRADE_WIND_INNER: f64 = 0.3;
pub(crate) const TRADE_WIND_OUTER: f64 = 0.7;
/// Evaporation gain over ocean cells per march step (port values).
pub(crate) const EVAPORATION_BASE: f64 = 0.15;
pub(crate) const EVAPORATION_OFFSET_FACTOR: f64 = 0.1;
pub(crate) const EVAPORATION_MIN: f64 = 0.05;
/// Orographic precipitation factors: baseline drizzle and the
/// precipitation gained per unit of normalized windward rise (port
/// values).
pub(crate) const OROGRAPHIC_BASE: f64 = 0.02;
pub(crate) const OROGRAPHIC_SLOPE_FACTOR: f64 = 8.0;
/// Conversion from normalized precipitation to humidity units (port
/// value).
pub(crate) const HUMIDITY_PRECIP_FACTOR: f64 = 20.0;
/// Annual precipitation in mm/year of fully saturated orographic
/// precipitation.
pub(crate) const PRECIP_MM_SCALE: f64 = 3000.0;
/// Height in meters that counts as a "full" windward rise; high-mountain
/// relief. Fixture worlds are far lower and produce mostly baseline
/// precipitation.
pub(crate) const SLOPE_HEIGHT_REFERENCE_M: f64 = 4000.0;
/// Radius of the humidity box smoother (port value).
pub const SMOOTHING_RADIUS: usize = 3;

/// Errors of the climate stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClimateError {
    /// The grid is empty or its cell storage does not match its declared
    /// size; the stage refuses to run on an inconsistent world.
    InvalidGrid,
    /// A computed value could not be quantized onto its lattice.
    Quantization(QuantError),
}

impl fmt::Display for ClimateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClimateError::InvalidGrid => {
                write!(f, "climate stage requires a consistent, non-empty grid")
            }
            ClimateError::Quantization(source) => write!(f, "climate stage: {source}"),
        }
    }
}

impl std::error::Error for ClimateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClimateError::Quantization(source) => Some(source),
            ClimateError::InvalidGrid => None,
        }
    }
}

/// Latitude factor of a cell row: `0` at the equator row, `1` at the
/// pole rows. Row centers are used so top and bottom rows are symmetric.
pub(crate) fn latitude_factor(y: usize, height: u32) -> f64 {
    ((y as f64 + 0.5) / f64::from(height) - 0.5).abs() * 2.0
}

/// Wind direction of a latitude band: `+1` (eastward march) inside the
/// trade-wind band, `-1` (westward) outside it. The band bounds are
/// strict, matching the port.
pub(crate) fn wind_direction(lat_factor: f64) -> i32 {
    if lat_factor > TRADE_WIND_INNER && lat_factor < TRADE_WIND_OUTER {
        1
    } else {
        -1
    }
}

/// Evaporation gain over ocean cells for a normalized humidity offset.
pub(crate) fn evaporation(humidity_offset: f64) -> f64 {
    (EVAPORATION_BASE + EVAPORATION_OFFSET_FACTOR * humidity_offset).max(EVAPORATION_MIN)
}

/// Applies the climate stage to `world` in place.
///
/// Fills the `climate` section and installs `params` into
/// `world.params` (params version 1). The world must carry a consistent,
/// non-empty grid with heights and water flags; see the crate docs for
/// the model. Pure with respect to `(world grid, params, streams)`:
/// identical inputs produce identical sections.
pub fn generate(
    world: &mut GeographicWorld,
    params: &ClimateParams,
    streams: &RngStreams,
) -> Result<(), ClimateError> {
    let width = world.grid.width;
    let height = world.grid.height;
    let row = width as usize;
    let cell_count = row * height as usize;
    if width == 0 || height == 0 || world.grid.cells.len() != cell_count {
        return Err(ClimateError::InvalidGrid);
    }

    let temperature_offset_c = params.temperature_offset.to_value();
    let amplification = params.polar_amplification.to_value();
    let exponent = params.latitude_exponent.to_value();
    let humidity_offset = params.humidity_offset.to_value();

    let mut noise_seed_bytes = [0u8; 4];
    streams.stream_bytes(STREAM_TEMPERATURE, &mut noise_seed_bytes);
    let mut noise = FastNoiseLite::new();
    noise.set_seed(Some(i32::from_le_bytes(noise_seed_bytes)));
    noise.set_frequency(Some(NOISE_FREQUENCY));
    noise.set_noise_type(Some(NoiseType::OpenSimplex2));

    // The noise samples a cylinder of circumference `width` cells, so the
    // field wraps seamlessly along x. FastNoiseLite speaks f32; the
    // sample coordinates are computed there deliberately.
    let width_f32 = width as f32;
    let cylinder_radius = f64::from(width) / std::f64::consts::TAU;
    let cylinder_radius = cylinder_radius as f32;

    let heights_m: Vec<f64> = world
        .grid
        .cells
        .iter()
        .map(|cell| cell.height.to_value())
        .collect();
    let is_water: Vec<bool> = world.grid.cells.iter().map(|cell| cell.is_water).collect();

    // --- Temperature ---
    let mut temperature = Vec::with_capacity(cell_count);
    for y in 0..height as usize {
        let lat_factor = latitude_factor(y, height);
        let offset_amplification = 1.0 + lat_factor * amplification;
        let y_sample = y as f32 + 0.5;
        for x in 0..row {
            let angle = ((x as f32 + 0.5) / width_f32) * std::f32::consts::TAU;
            let noise_value = f64::from(noise.get_noise_3d(
                cylinder_radius * angle.cos(),
                y_sample,
                cylinder_radius * angle.sin(),
            ));
            let index = y * row + x;
            let base = T_EQUATOR_C + (T_POLE_C - T_EQUATOR_C) * lat_factor.powf(exponent);
            let mut value = base + NOISE_SHARE * (T_EQUATOR_C - T_POLE_C) * noise_value;
            value += temperature_offset_c * offset_amplification;
            let cell_height = heights_m[index];
            if is_water[index] {
                let depth_m = (-cell_height).max(0.0);
                value -= OCEAN_DAMPING_MAX_C * (depth_m / OCEAN_DAMPING_DEPTH_M).min(1.0);
            } else {
                value -= LAPSE_RATE_C_PER_M * cell_height.max(0.0);
            }
            let clamped = value.clamp(TEMP_MIN_C, TEMP_MAX_C);
            temperature
                .push(TempDeciC::try_from_value(clamped).map_err(ClimateError::Quantization)?);
        }
    }

    // --- Moisture march: humidity and precipitation ---
    let ocean_evaporation = evaporation(humidity_offset);
    let mut humidity_raw = vec![0.0f64; cell_count];
    let mut precip_norm = vec![0.0f64; cell_count];
    for y in 0..height as usize {
        let direction = wind_direction(latitude_factor(y, height));
        let mut moisture = 0.0f64;
        let mut previous_height_norm: Option<f64> = None;
        for step in 0..row {
            let x = if direction > 0 { step } else { row - 1 - step };
            let index = y * row + x;
            let height_norm = heights_m[index] / SLOPE_HEIGHT_REFERENCE_M;
            if is_water[index] {
                moisture = (moisture + ocean_evaporation).min(1.0);
                // The ocean carries all the moisture it can; forced
                // before smoothing so only coastal cells blend away.
                humidity_raw[index] = 1.0;
            } else {
                let rise =
                    previous_height_norm.map_or(0.0, |previous| (height_norm - previous).max(0.0));
                let precipitation =
                    (moisture * (OROGRAPHIC_BASE + OROGRAPHIC_SLOPE_FACTOR * rise)).min(moisture);
                moisture -= precipitation;
                precip_norm[index] = precipitation;
                humidity_raw[index] =
                    (precipitation * HUMIDITY_PRECIP_FACTOR + humidity_offset).max(0.0);
            }
            previous_height_norm = Some(height_norm);
        }
    }
    let humidity_smoothed = box_smooth(&humidity_raw, width, height, SMOOTHING_RADIUS);

    // --- Quantize and hand over ---
    let mut humidity = Vec::with_capacity(cell_count);
    let mut precipitation = Vec::with_capacity(cell_count);
    for index in 0..cell_count {
        let relative = humidity_smoothed[index].clamp(0.0, 1.0);
        humidity.push(
            HumidDeciPct::try_from_value(relative * 100.0).map_err(ClimateError::Quantization)?,
        );
        let millimeters = (precip_norm[index] * PRECIP_MM_SCALE).max(0.0);
        precipitation
            .push(PrecipMmYr::try_from_value(millimeters).map_err(ClimateError::Quantization)?);
    }

    // The climate block is installed without clobbering the terrain
    // block: stages compose, and the params version tracks the present
    // blocks (2 when terrain ran first, 1 otherwise).
    let terrain = world.params.terrain.clone();
    let hydrology = world.params.hydrology;
    world.params = GenerationParams {
        params_version: if terrain.is_some() {
            4
        } else if hydrology.is_some() {
            3
        } else {
            1
        },
        seed: world.params.seed,
        climate: Some(params.clone()),
        terrain,
        hydrology,
    };
    world.climate = ClimateSection {
        temperature: Some(temperature),
        humidity: Some(humidity),
        precipitation: Some(precipitation),
    };
    Ok(())
}

/// Separable box blur with clamped edges: constant fields stay constant,
/// and the pass order (horizontal, then vertical) is part of the
/// determinism contract.
pub(crate) fn box_smooth(values: &[f64], width: u32, height: u32, radius: usize) -> Vec<f64> {
    let w = width as usize;
    let h = height as usize;
    let mut horizontal = vec![0.0f64; values.len()];
    for y in 0..h {
        for x in 0..w {
            let lo = x.saturating_sub(radius);
            let hi = (x + radius).min(w - 1);
            let mut sum = 0.0;
            for k in lo..=hi {
                sum += values[y * w + k];
            }
            horizontal[y * w + x] = sum / ((hi - lo + 1) as f64);
        }
    }
    let mut out = vec![0.0f64; values.len()];
    for y in 0..h {
        for x in 0..w {
            let lo = y.saturating_sub(radius);
            let hi = (y + radius).min(h - 1);
            let mut sum = 0.0;
            for k in lo..=hi {
                sum += horizontal[k * w + x];
            }
            out[y * w + x] = sum / ((hi - lo + 1) as f64);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latitude_factor_spans_the_bands() {
        assert!((latitude_factor(0, 32) - 0.968_75).abs() < 1e-12);
        let middle = (16.5f64 / 32.0 - 0.5).abs() * 2.0;
        assert!((latitude_factor(16, 32) - middle).abs() < 1e-12);
        assert!(latitude_factor(0, 32) > latitude_factor(16, 32));
    }

    #[test]
    fn trade_winds_blow_only_inside_the_band() {
        assert_eq!(wind_direction(0.0), -1);
        assert_eq!(wind_direction(0.3), -1, "band bounds are strict");
        assert_eq!(wind_direction(0.5), 1);
        assert_eq!(wind_direction(0.7), -1, "band bounds are strict");
        assert_eq!(wind_direction(1.0), -1);
    }

    #[test]
    fn evaporation_follows_the_offset() {
        assert!((evaporation(0.0) - 0.15).abs() < 1e-12);
        assert!((evaporation(0.4) - 0.19).abs() < 1e-12);
        assert!((evaporation(-0.4) - 0.11).abs() < 1e-12);
        // The floor is a port-value guard for offsets beyond the domain.
        assert!((evaporation(-1.0) - 0.05).abs() < 1e-12, "floor applies");
    }

    #[test]
    fn smoothing_keeps_constant_fields_constant() {
        let values = vec![0.7f64; 6 * 4];
        let smoothed = box_smooth(&values, 6, 4, 3);
        assert!(smoothed.iter().all(|value| (value - 0.7).abs() < 1e-12));
    }

    #[test]
    fn smoothing_pulls_a_spike_toward_the_neighbors() {
        let mut values = vec![0.0f64; 7];
        values[3] = 1.0;
        // Degenerate height of one row: only the horizontal pass acts.
        let smoothed = box_smooth(&values, 7, 1, 1);
        assert_eq!(smoothed[0], 0.0, "clamped edge keeps its weight");
        assert_eq!(smoothed[1], 0.0, "window [0..=1] does not see the spike");
        assert!((smoothed[2] - 1.0 / 3.0).abs() < 1e-12);
        assert!(smoothed[3] < 1.0 && smoothed[3] > 0.0);
    }
}
