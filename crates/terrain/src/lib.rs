//! Heightmap and erosion stage of the Vernadsky planetary geography
//! generator: the base relief pass that turns a
//! [`vernadsky_core::skeleton_world`] into a physical world.
//!
//! The stage fills the cell heights (in meters, sea level `0`) and the
//! water flags; the climate, biome, and hydrology stages consume them
//! downstream. It is a port of the heightmap pass of our earlier mapgen
//! prototype, re-expressed in physical units for the Vernadsky
//! lattices, with every float→lattice conversion going through
//! `vernadsky_core::quant`.
//!
//! # Pipeline
//!
//! 1. **Base noise**: 3D fBm OpenSimplex2 (FastNoiseLite) sampled on a
//!    cylinder for x-seamlessness, seeded from the [`STREAM_HEIGHTMAP`]
//!    stream, remapped to `[0, 1]`.
//! 2. **Island effect**: a second noise (stream [`STREAM_ISLANDS`])
//!    lifts low-lying cells, scattering islands across the oceans.
//! 3. **Smoothing**: two-pass box blur with clamped edges.
//! 4. **Power correction**: `h ← h^power` with the port exponent `1.0`.
//! 5. **Normalization** to `[0, 1]` by min/max.
//! 6. **Thermal erosion** (when the erosion profile is on): talus-angle
//!    material slide, deterministic, double-buffered, port transfer
//!    factor `0.3`.
//! 7. **Hydraulic erosion** (when the erosion profile is on): water
//!    drops drawn from the [`STREAM_EROSION`] stream walk downhill up
//!    to 30 steps with inertial speed (`speed ← speed·0.9 + Δh`),
//!    eroding `min(speed·power, h/2)` and depositing at low speed.
//! 8. **Land-ratio targeting**: the offset over `[0, 1]` whose land
//!    share (cells above `0.5`) is closest to the target `0.3` — the
//!    Earth-like convention.
//! 9. **Physical hand-off**: `meters = (n − 0.5) · 12 000` (sea level
//!    `0.5 ⇔ 0 m`), quantized to `HeightM`; `is_water ⇔ meters ≤ 0`.
//!
//! The stage consumes randomness from exactly three named streams;
//! coming online shifts no other stream, and identical worlds with
//! identical parameters reproduce byte for byte.
//!
//! # Parameters
//!
//! [`TerrainParams`] carries the optional erosion profile (see
//! [`ErosionParams`] in `vernadsky_core`): the port defaults are one
//! percent of the map area as water drops, erosion power `0.02`, and a
//! talus angle of `120 m`. The stage installs its parameters into the
//! export (params version 2, the block-presence layout) and a TOML
//! configuration is available through the `params` module
//! ([`TerrainConfig`]).
//!
//! [`TerrainConfig`]: crate::params::TerrainConfig
//!
//! # Example
//!
//! ```
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     use vernadsky_core::{skeleton_world, RngStreams};
//!     use vernadsky_terrain::{generate, TerrainConfig};
//!
//!     let mut world = skeleton_world(48, 32, 42);
//!     let params = TerrainConfig::default().to_params()?;
//!     generate(&mut world, &params, &RngStreams::new(42))?;
//!
//!     let land = world.grid.cells.iter().filter(|cell| !cell.is_water).count();
//!     assert!(land > 0, "the targeted land ratio keeps land on the map");
//!     assert_eq!(world.params.params_version, 4);
//!     Ok(())
//! }
//! ```

pub mod params;

pub use params::{ParamsError, TerrainConfig, from_toml_str};

use std::fmt;

use fastnoise_lite::{FastNoiseLite, FractalType, NoiseType};
use rand_chacha::rand_core::RngCore;
use vernadsky_core::RngStreams;
use vernadsky_core::quant::{HeightM, QuantError};
use vernadsky_core::schema::{
    CellRecord, ErosionParams, GenerationParams, GeographicWorld, NO_INDEX, TerrainParams,
};

/// Stream name of the base height noise.
pub const STREAM_HEIGHTMAP: &str = "heightmap";
/// Stream name of the island-effect noise.
pub const STREAM_ISLANDS: &str = "terrain.islands";
/// Stream name of the hydraulic erosion drops.
pub const STREAM_EROSION: &str = "terrain.erosion";

/// The island noise spans this multiple of the base feature count
/// across the map (the mapgen port ratio `0.015 / 0.005`).
const ISLAND_FREQUENCY_RATIO: f32 = 3.0;
/// The octave budget of the fBm stack: high-resolution maps spend it on
/// progressively finer detail down to the configured detail floor.
const MAX_OCTAVES: i32 = 8;
/// The detail floor default of [`TerrainConfig`]:
/// features below this wavelength in cells would read as speckle at
/// any resolution (the mapgen port's finest octave).
pub const DEFAULT_MIN_FEATURE_CELLS: u32 = 8;
/// Island lift strength (mapgen `island_density` mid value).
const ISLAND_DENSITY: f64 = 0.5;
/// Smoothing radius of the two-pass box blur.
const SMOOTH_RADIUS: usize = 2;
/// Power-correction exponent (the port's neutral value; `1.0` keeps the
/// noise distribution, the step itself stays in the pipeline for the
/// future tuning surface).
const ELEVATION_POWER: f64 = 1.0;
/// Thermal erosion passes (port value).
const THERMAL_ITERATIONS: usize = 3;
/// Fraction of the excess slope moved per thermal pass (port value).
const THERMAL_TRANSFER: f64 = 0.3;
/// Hydraulic droplet lifetime, in cells (port value).
const DROP_STEPS: usize = 30;
/// Droplet speed inertia (port value).
const DROP_INERTIA: f64 = 0.9;
/// Speed below which a droplet deposits part of its load (port value).
const DROP_DEPOSIT_SPEED: f64 = 0.1;
/// Fraction of the load deposited per low-speed step (port value).
const DROP_DEPOSIT_FRACTION: f64 = 0.1;
/// Target land share after the offset search (the Earth-like
/// convention of the port).
const TARGET_LAND_RATIO: f64 = 0.3;
/// Full physical height scale: normalized 0.5 maps to sea level (0 m),
/// the extremes to ±6000 m.
const HEIGHT_SCALE_M: f64 = 12_000.0;

/// Errors of the terrain stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainError {
    /// The grid is empty or its cell storage does not match its declared
    /// size; the stage refuses to run on an inconsistent world.
    InvalidGrid,
    /// A computed value could not be quantized onto its lattice.
    Quantization(QuantError),
}

impl fmt::Display for TerrainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TerrainError::InvalidGrid => {
                write!(f, "terrain stage requires a consistent, non-empty grid")
            }
            TerrainError::Quantization(source) => write!(f, "terrain stage: {source}"),
        }
    }
}

impl std::error::Error for TerrainError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TerrainError::Quantization(source) => Some(source),
            TerrainError::InvalidGrid => None,
        }
    }
}

/// Applies the terrain stage to `world` in place.
///
/// Fills the cell heights and water flags, resets the per-cell water
/// body references (the hydrology stage owns that topology), and
/// installs `params` into `world.params` (params version 2, preserving
/// an existing climate block). Pure with respect to
/// `(world grid, params, streams)`.
pub fn generate(
    world: &mut GeographicWorld,
    params: &TerrainParams,
    streams: &RngStreams,
) -> Result<(), TerrainError> {
    let width = world.grid.width;
    let height = world.grid.height;
    let row = width as usize;
    let cell_count = row * height as usize;
    if width == 0 || height == 0 || world.grid.cells.len() != cell_count {
        return Err(TerrainError::InvalidGrid);
    }

    // --- 1. Base fBm noise on the cylinder ---
    //
    // The cylinder already normalizes the x axis; the frequency does
    // the same for the feature count: exactly `features_across` base
    // features span the circumference at every grid resolution, so the
    // world's macro structure does not depend on the map size.
    let features = params.features_across.to_value() as f32;
    let frequency = features / width as f32;
    let base_wavelength = width as f32 / features;
    let octaves = effective_octaves(MAX_OCTAVES, base_wavelength, params.min_feature_cells);
    let noise = seeded_noise(streams, STREAM_HEIGHTMAP, frequency, octaves);
    let mut heights: Vec<f64> = Vec::with_capacity(cell_count);
    for y in 0..height as usize {
        let y_sample = y as f32 + 0.5;
        for x in 0..row {
            heights.push(normalized(noise_sample(
                &noise,
                width,
                x,
                y_sample,
                cylinder_radius(width),
            )));
        }
    }

    // --- 2. Island effect: lift the lowlands with a second noise ---
    let islands = seeded_noise(
        streams,
        STREAM_ISLANDS,
        frequency * ISLAND_FREQUENCY_RATIO,
        1,
    );
    for (index, height) in heights.iter_mut().enumerate() {
        let y = index / row;
        let sample = normalized(noise_sample(
            &islands,
            width,
            index % row,
            y as f32 + 0.5,
            cylinder_radius(width),
        ));
        let lowland = (0.5 - *height).max(0.0);
        *height += sample * ISLAND_DENSITY * lowland;
    }

    // --- 3. Smoothing ---
    box_smooth(&mut heights, row, height as usize, SMOOTH_RADIUS);

    // --- 4. Power correction ---
    for value in &mut heights {
        *value = value.clamp(0.0, 1.0).powf(ELEVATION_POWER);
    }

    // --- 5. Normalization by min/max ---
    let min = heights.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = heights.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if max > min {
        for value in &mut heights {
            *value = (*value - min) / (max - min);
        }
    }

    // --- 6/7. Erosion (optional profile) ---
    if let Some(erosion) = &params.erosion {
        apply_thermal_erosion(&mut heights, row, height as usize, erosion);
        apply_hydraulic_erosion(
            &mut heights,
            row,
            height as usize,
            streams,
            erosion,
            cell_count,
        );
    }

    // --- 8. Land-ratio targeting ---
    apply_land_ratio_target(&mut heights, TARGET_LAND_RATIO);

    // --- 9. Physical hand-off ---
    let mut cells = Vec::with_capacity(cell_count);
    for value in &heights {
        let meters = (value.clamp(0.0, 1.0) - 0.5) * HEIGHT_SCALE_M;
        let height = HeightM::try_from_value(meters).map_err(TerrainError::Quantization)?;
        // The quantized lattice is the truth: the water flag follows it,
        // so the flag and the exported height never disagree.
        let is_water = height.to_value() <= 0.0;
        cells.push(CellRecord {
            height,
            is_water,
            territory: NO_INDEX,
            water_body: NO_INDEX,
        });
    }
    world.grid.cells = cells;
    // The per-cell water body references belong to the hydrology stage;
    // a rerun over a staged world must not keep stale topology.
    for cell in &mut world.grid.cells {
        cell.water_body = NO_INDEX;
    }

    // The terrain block is installed without clobbering the climate
    // and hydrology blocks; the params version tracks the terrain
    // layout (4, the map-space fields).
    let climate = world.params.climate.clone();
    let hydrology = world.params.hydrology;
    world.params = GenerationParams {
        params_version: 4,
        seed: world.params.seed,
        climate,
        terrain: Some(params.clone()),
        hydrology,
    };
    Ok(())
}

/// Creates the stage noise for one named stream.
fn seeded_noise(
    streams: &RngStreams,
    name: &'static str,
    frequency: f32,
    octaves: i32,
) -> FastNoiseLite {
    let mut seed_bytes = [0u8; 4];
    streams.stream_bytes(name, &mut seed_bytes);
    let mut noise = FastNoiseLite::new();
    noise.set_seed(Some(i32::from_le_bytes(seed_bytes)));
    noise.set_noise_type(Some(NoiseType::OpenSimplex2));
    noise.set_fractal_type(Some(FractalType::FBm));
    noise.set_fractal_octaves(Some(octaves));
    noise.set_frequency(Some(frequency));
    noise
}

fn cylinder_radius(width: u32) -> f32 {
    (f64::from(width) / std::f64::consts::TAU) as f32
}

/// The octave count the detail floor allows: the fBm stack stops before
/// its wavelengths drop below `min_feature_cells`, so features smaller
/// than the floor never exist — at any grid resolution. High-resolution
/// maps spend the budget on finer detail; small ones simply use fewer
/// octaves.
fn effective_octaves(max_octaves: i32, base_wavelength_cells: f32, min_feature_cells: u32) -> i32 {
    if min_feature_cells == 0 || base_wavelength_cells <= 0.0 {
        return max_octaves.max(1);
    }
    let ratio = base_wavelength_cells / min_feature_cells as f32;
    let affordable = 1.0 + ratio.log2();
    affordable.floor().max(1.0).min(max_octaves.max(1) as f32) as i32
}

/// Samples the 3D noise at a cell center on the x-seamless cylinder.
fn noise_sample(noise: &FastNoiseLite, width: u32, x: usize, y: f32, radius: f32) -> f32 {
    let angle = ((x as f32 + 0.5) / width as f32) * std::f32::consts::TAU;
    noise.get_noise_3d(radius * angle.cos(), y, radius * angle.sin())
}

/// Remaps the noise range (roughly `[-1, 1]`) onto `[0, 1]`.
fn normalized(sample: f32) -> f64 {
    (f64::from(sample) + 1.0) / 2.0
}

/// Two-pass box blur with clamped edges (the pass order — horizontal,
/// then vertical — is part of the determinism contract).
fn box_smooth(values: &mut [f64], width: usize, height: usize, radius: usize) {
    let mut scratch = vec![0.0f64; values.len()];
    for y in 0..height {
        for x in 0..width {
            let lo = x.saturating_sub(radius);
            let hi = (x + radius).min(width - 1);
            let mut sum = 0.0;
            for k in lo..=hi {
                sum += values[y * width + k];
            }
            scratch[y * width + x] = sum / ((hi - lo + 1) as f64);
        }
    }
    for y in 0..height {
        for x in 0..width {
            let lo = y.saturating_sub(radius);
            let hi = (y + radius).min(height - 1);
            let mut sum = 0.0;
            for k in lo..=hi {
                sum += scratch[k * width + x];
            }
            values[y * width + x] = sum / ((hi - lo + 1) as f64);
        }
    }
}

/// Thermal erosion: material above the talus angle slides to the lowest
/// neighbor, a fraction of the excess per pass (double-buffered, the
/// neighbor walk is x-seamless and latitude-clamped).
fn apply_thermal_erosion(
    heights: &mut [f64],
    width: usize,
    height: usize,
    erosion: &ErosionParams,
) {
    let talus = (erosion.talus.to_value() / HEIGHT_SCALE_M).max(0.0);
    let directions = [(0i64, 1i64), (1, 0), (0, -1), (-1, 0)];
    for _ in 0..THERMAL_ITERATIONS {
        let mut scratch = heights.to_vec();
        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let current = heights[index];
                let mut max_diff = 0.0f64;
                let mut target = index;
                for &(dy, dx) in &directions {
                    let ny = y as i64 + dy;
                    if ny < 0 || ny >= height as i64 {
                        continue;
                    }
                    let nx = (x as i64 + dx).rem_euclid(width as i64) as usize;
                    let neighbor = ny as usize * width + nx;
                    let diff = current - heights[neighbor];
                    if diff > max_diff {
                        max_diff = diff;
                        target = neighbor;
                    }
                }
                if max_diff > talus {
                    let moved = (max_diff - talus) * THERMAL_TRANSFER;
                    scratch[index] -= moved;
                    scratch[target] += moved;
                }
            }
        }
        heights.copy_from_slice(&scratch);
    }
}

/// Hydraulic erosion: droplets from the stage stream walk downhill,
/// eroding at speed and depositing when slow.
fn apply_hydraulic_erosion(
    heights: &mut [f64],
    width: usize,
    height: usize,
    streams: &RngStreams,
    erosion: &ErosionParams,
    cell_count: usize,
) {
    let power = erosion.power.to_value();
    let drops =
        ((cell_count as f64) * erosion.droplets_per_hundred_cells.to_value() / 100.0 / 100.0)
            .round() as usize;
    let mut rng = streams.stream(STREAM_EROSION);
    for _ in 0..drops {
        let mut x = (rng.next_u64() % width as u64) as usize;
        let mut y = (rng.next_u64() % height as u64) as usize;
        let mut sediment = 0.0f64;
        let mut speed = 0.0f64;
        for _ in 0..DROP_STEPS {
            let index = y * width + x;
            let mut lowest = heights[index];
            let mut next = (x, y);
            for &(dy, dx) in &DIRECTIONS_HYDRAULIC {
                let ny = y as i64 + dy;
                if ny < 0 || ny >= height as i64 {
                    continue;
                }
                let nx = (x as i64 + dx).rem_euclid(width as i64) as usize;
                let neighbor = ny as usize * width + nx;
                if heights[neighbor] < lowest {
                    lowest = heights[neighbor];
                    next = (nx, ny as usize);
                }
            }
            if next == (x, y) {
                break; // a pit: the droplet settles here
            }
            let height_diff = heights[index] - lowest;
            speed = speed * DROP_INERTIA + height_diff;
            let eroded = (speed * power).min(heights[index] * 0.5);
            heights[index] -= eroded;
            sediment += eroded;
            if speed < DROP_DEPOSIT_SPEED && sediment > 0.0 {
                let deposit = sediment * DROP_DEPOSIT_FRACTION;
                heights[index] += deposit;
                sediment -= deposit;
            }
            x = next.0;
            y = next.1;
        }
        if sediment > 0.0 {
            let index = y * width + x;
            heights[index] += sediment;
        }
    }
}

/// The four orthogonal offsets of the erosion passes.
const DIRECTIONS_HYDRAULIC: [(i64, i64); 4] = [(0, 1), (1, 0), (0, -1), (-1, 0)];

/// Picks the offset whose land share (cells above the sea level mark)
/// is closest to the target, and applies it.
fn apply_land_ratio_target(heights: &mut [f64], target_land_ratio: f64) {
    let count = heights.len() as f64;
    let mut best_offset = 0.0;
    let mut best_diff = f64::INFINITY;
    for step in 0..100 {
        let offset = step as f64 / 100.0 - 0.5;
        let land = heights
            .iter()
            .filter(|&&value| (value + offset).clamp(0.0, 1.0) > 0.5)
            .count() as f64;
        let diff = (land / count - target_land_ratio).abs();
        if diff < best_diff {
            best_diff = diff;
            best_offset = offset;
        }
    }
    for value in &mut heights[..] {
        *value = (*value + best_offset).clamp(0.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octave_budget_respects_the_detail_floor() {
        // A 2048-wide map at 2.0 features: base wavelength 1024 cells
        // affords detail down to the 8-cell floor and the budget caps.
        assert_eq!(effective_octaves(8, 1024.0, 8), 8);
        // A 200-wide map: 50-cell base wavelength affords ~3.6 -> 3.
        assert_eq!(effective_octaves(8, 50.0, 8), 3);
        // A fixture-sized map affords two octaves (wavelengths 24, 12).
        assert_eq!(effective_octaves(8, 24.0, 8), 2);
    }

    #[test]
    fn octave_budget_stays_within_the_maximum() {
        assert_eq!(effective_octaves(5, 4096.0, 1), 5);
        assert_eq!(effective_octaves(5, 4096.0, 0), 5, "degenerate floor");
        assert_eq!(effective_octaves(5, 0.0, 8), 5, "degenerate wavelength");
    }
}
