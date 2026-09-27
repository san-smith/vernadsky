//! The biome stage: per-cell classification into registry biomes.
//!
//! The classification is a pure, integer-only walk over ordered rule
//! tables; the tables are the extension points of the stage (adding,
//! splitting, or retiring a biome touches only the tables and the
//! registry — never the schema, the format, or consumers).

use std::fmt;

use fastnoise_lite::{FastNoiseLite, NoiseType};
use vernadsky_core::quant::{HeightM, HumidDeciPct, QuantError, TempDeciC};
use vernadsky_core::schema::{BiomeSection, GeographicWorld};
use vernadsky_core::{BiomeId, RngStreams};

/// Stream name of the boundary dither noise.
pub const STREAM_BOUNDARY: &str = "biome.boundary";

/// Noise frequency of the boundary dither (shared with the climate
/// noise scale, so the dither varies on the same wavelength).
const NOISE_FREQUENCY: f32 = 0.005;

/// Dither amplitude in °C: the noise spans roughly ±1, so the
/// temperature thresholds wobble by ±2 °C in lattice steps.
const DITHER_TEMPERATURE_C: f64 = 2.0;
/// Dither amplitude in % of relative humidity (±5 %).
const DITHER_HUMIDITY_PCT: f64 = 5.0;

// Physical thresholds on the Vernadsky lattices; the normalized mapgen
// values they re-derive are noted in the rule tables.
//
// Water: sea ice forms near −2 °C; the model is coarse, so the pack-ice
// bound keeps a margin (mapgen: `ICE_TEMP_LIMIT` ± bands).
const WATER_FREEZING_BELOW_C: i16 = -50; // −5 °C
const WATER_ICY_BELOW_C: i16 = 50; // +5 °C
/// Deep ocean: mapgen documented `DEEP_OCEAN_DEPTH` 0.1 as ">1000 m".
const DEEP_OCEAN_DEEPER_THAN_M: i32 = 1000;

// Mountains: snowline convention, 2500–5000 m; the synthetic fixture
// worlds are far lower, so this tier is covered by unit tests.
const MOUNTAIN_START_M: i32 = 2500;
const MOUNTAIN_PEAK_M: i32 = 3500;
/// Higher peaks stay glaciated at warmer annual temperatures than the
/// lower slopes (mapgen: 0.3 vs 0.25 normalized).
const PEAK_GLACIAL_BELOW_C: i16 = 50; // +5 °C
const SLOPE_GLACIAL_BELOW_C: i16 = 0; // 0 °C

// Climate bands: annual isotherms re-derived for the model's latitude
// profile (mapgen: 0.15 / 0.3 / 0.65 normalized).
const ICE_BELOW_C: i16 = -120; // −12 °C
const COLD_BELOW_C: i16 = 0; // 0 °C
const TEMPERATE_BELOW_C: i16 = 180; // +18 °C
// Humidity thresholds inside the bands (mapgen values 1:1, in tenths
// of a percent).
const DRY_BELOW_PCT: i16 = 200; // 20 %
const DESERT_BELOW_PCT: i16 = 250; // 25 %
const MODERATE_BELOW_PCT: i16 = 400; // 40 %
const SAVANNA_BELOW_PCT: i16 = 550; // 55 %
const WET_BELOW_PCT: i16 = 700; // 70 %

// Registry identifiers used by the rule tables. They are validated
// against the registry by tests (`rules_reference_live_entries`): each
// must exist, be produced (not deprecated), and belong to the tier's
// category.
const FROZEN_OCEAN: BiomeId = BiomeId(4);
const ICY_OCEAN: BiomeId = BiomeId(3);
const DEEP_OCEAN: BiomeId = BiomeId(2);
const OCEAN: BiomeId = BiomeId(1);
const ICE: BiomeId = BiomeId(5);
const TUNDRA: BiomeId = BiomeId(6);
const TAIGA: BiomeId = BiomeId(7);
const TEMPERATE_FOREST: BiomeId = BiomeId(8);
const TROPICAL_RAINFOREST: BiomeId = BiomeId(9);
const GRASSLAND: BiomeId = BiomeId(10);
const SHRUBLAND: BiomeId = BiomeId(11);
const SAVANNA: BiomeId = BiomeId(12);
const DESERT: BiomeId = BiomeId(13);
const SWAMP: BiomeId = BiomeId(14);
const ROCKY_MOUNTAIN: BiomeId = BiomeId(15);
const GLACIAL_MOUNTAIN: BiomeId = BiomeId(16);

/// One ordered climate rule: the first row whose bounds contain the
/// (dithered) cell values wins. `None` means "no bound on this axis".
#[derive(Clone, Copy, Debug)]
struct ClimateRule {
    /// Upper temperature bound, in lattice steps (exclusive).
    max_temperature: Option<i16>,
    /// Upper humidity bound, in lattice steps (exclusive).
    max_humidity: Option<i16>,
    biome: BiomeId,
}

/// The climate table, cold-dry corner first, tropical-humid last; the
/// fallback below the table is [`TROPICAL_RAINFOREST`].
const CLIMATE_RULES: &[ClimateRule] = &[
    ClimateRule {
        max_temperature: Some(ICE_BELOW_C),
        max_humidity: None,
        biome: ICE,
    },
    ClimateRule {
        max_temperature: Some(COLD_BELOW_C),
        max_humidity: Some(MODERATE_BELOW_PCT),
        biome: TUNDRA,
    },
    ClimateRule {
        max_temperature: Some(COLD_BELOW_C),
        max_humidity: None,
        biome: TAIGA,
    },
    ClimateRule {
        max_temperature: Some(TEMPERATE_BELOW_C),
        max_humidity: Some(DRY_BELOW_PCT),
        biome: SHRUBLAND,
    },
    ClimateRule {
        max_temperature: Some(TEMPERATE_BELOW_C),
        max_humidity: Some(MODERATE_BELOW_PCT),
        biome: GRASSLAND,
    },
    ClimateRule {
        max_temperature: Some(TEMPERATE_BELOW_C),
        max_humidity: Some(WET_BELOW_PCT),
        biome: TEMPERATE_FOREST,
    },
    ClimateRule {
        max_temperature: Some(TEMPERATE_BELOW_C),
        max_humidity: None,
        biome: SWAMP,
    },
    ClimateRule {
        max_temperature: None,
        max_humidity: Some(DESERT_BELOW_PCT),
        biome: DESERT,
    },
    ClimateRule {
        max_temperature: None,
        max_humidity: Some(SAVANNA_BELOW_PCT),
        biome: SAVANNA,
    },
];

/// Mountain bands, evaluated from the highest: `(minimal height in
/// meters, glaciated below this temperature, glacial, rocky)`.
const MOUNTAIN_RULES: &[(i32, i16, BiomeId, BiomeId)] = &[
    (
        MOUNTAIN_PEAK_M,
        PEAK_GLACIAL_BELOW_C,
        GLACIAL_MOUNTAIN,
        ROCKY_MOUNTAIN,
    ),
    (
        MOUNTAIN_START_M,
        SLOPE_GLACIAL_BELOW_C,
        GLACIAL_MOUNTAIN,
        ROCKY_MOUNTAIN,
    ),
];

/// Water rules by temperature, evaluated from the coldest: `(upper
/// temperature bound in lattice steps, biome)`; deeper-than-1000 m
/// falls to the deep ocean, the rest is the open ocean.
const WATER_RULES: &[(i16, BiomeId)] = &[
    (WATER_FREEZING_BELOW_C, FROZEN_OCEAN),
    (WATER_ICY_BELOW_C, ICY_OCEAN),
];

/// Errors of the biome stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BiomeError {
    /// The grid is empty or its cell storage does not match its declared
    /// size; the stage refuses to run on an inconsistent world.
    InvalidGrid,
    /// The world carries no climate fields; classification consumes
    /// temperature and humidity.
    MissingClimate,
    /// A computed value could not be quantized onto its lattice.
    Quantization(QuantError),
}

impl fmt::Display for BiomeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BiomeError::InvalidGrid => {
                write!(f, "biome stage requires a consistent, non-empty grid")
            }
            BiomeError::MissingClimate => {
                write!(f, "biome stage requires a filled climate section")
            }
            BiomeError::Quantization(source) => write!(f, "biome stage: {source}"),
        }
    }
}

impl std::error::Error for BiomeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BiomeError::Quantization(source) => Some(source),
            BiomeError::InvalidGrid | BiomeError::MissingClimate => None,
        }
    }
}

/// The climate-relevant state of one cell, in lattice values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellInput {
    /// Whether the cell is covered by water (the schema flag, not a
    /// sea-level comparison).
    pub is_water: bool,
    /// Surface height; negative is depth.
    pub height: HeightM,
    /// Annual-mean temperature.
    pub temperature: TempDeciC,
    /// Relative humidity.
    pub humidity: HumidDeciPct,
}

/// Per-cell dither of the climate thresholds, quantized into lattice
/// steps. Classification stays integer with the dither in the tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dither {
    /// Offset added to every temperature bound.
    pub temperature: TempDeciC,
    /// Offset added to every humidity bound.
    pub humidity: HumidDeciPct,
}

impl Dither {
    /// Zero dither: thresholds apply exactly.
    pub const NONE: Self = Self {
        temperature: TempDeciC(0),
        humidity: HumidDeciPct(0),
    };
}

/// Classifies one cell. Pure and integer-only: the rule bounds and the
/// dither live on the lattices, so identical inputs always agree.
pub fn classify(cell: CellInput, dither: Dither) -> BiomeId {
    if cell.is_water {
        for &(bound, biome) in WATER_RULES {
            if cell.temperature.0 < bound {
                return biome;
            }
        }
        if -cell.height.0 > DEEP_OCEAN_DEEPER_THAN_M {
            return DEEP_OCEAN;
        }
        return OCEAN;
    }

    for &(minimal_height, glacial_below, glacial, rocky) in MOUNTAIN_RULES {
        if cell.height.0 > minimal_height {
            return if cell.temperature.0 < glacial_below {
                glacial
            } else {
                rocky
            };
        }
    }

    for rule in CLIMATE_RULES {
        if let Some(bound) = rule.max_temperature
            && cell.temperature.0 >= bound.saturating_add(dither.temperature.0)
        {
            continue;
        }
        if let Some(bound) = rule.max_humidity
            && cell.humidity.0 >= bound.saturating_add(dither.humidity.0)
        {
            continue;
        }
        return rule.biome;
    }
    TROPICAL_RAINFOREST
}

/// Applies the biome stage to `world` in place.
///
/// Requires a consistent, non-empty grid with a filled climate section
/// (temperature and humidity). Fills the `biomes` section; the world's
/// parameters are untouched — the stage has none. Pure with respect to
/// `(world, streams)`.
pub fn generate(world: &mut GeographicWorld, streams: &RngStreams) -> Result<(), BiomeError> {
    let width = world.grid.width;
    let height = world.grid.height;
    let row = width as usize;
    let cell_count = row * height as usize;
    if width == 0 || height == 0 || world.grid.cells.len() != cell_count {
        return Err(BiomeError::InvalidGrid);
    }
    let temperatures = world
        .climate
        .temperature
        .as_ref()
        .ok_or(BiomeError::MissingClimate)?;
    let humidities = world
        .climate
        .humidity
        .as_ref()
        .ok_or(BiomeError::MissingClimate)?;

    let mut noise_seed_bytes = [0u8; 4];
    streams.stream_bytes(STREAM_BOUNDARY, &mut noise_seed_bytes);
    let mut noise = FastNoiseLite::new();
    noise.set_seed(Some(i32::from_le_bytes(noise_seed_bytes)));
    noise.set_frequency(Some(NOISE_FREQUENCY));
    noise.set_noise_type(Some(NoiseType::OpenSimplex2));

    // The dither samples a cylinder of circumference `width` cells, so
    // it wraps seamlessly along x. FastNoiseLite speaks f32; the sample
    // coordinates are computed there deliberately.
    let width_f32 = width as f32;
    let cylinder_radius = (f64::from(width) / std::f64::consts::TAU) as f32;

    let mut biomes = Vec::with_capacity(cell_count);
    for y in 0..height as usize {
        let y_sample = y as f32 + 0.5;
        for x in 0..row {
            let angle = ((x as f32 + 0.5) / width_f32) * std::f32::consts::TAU;
            let noise_value = f64::from(noise.get_noise_3d(
                cylinder_radius * angle.cos(),
                y_sample,
                cylinder_radius * angle.sin(),
            ));
            let dither = Dither {
                temperature: TempDeciC::try_from_value(noise_value * DITHER_TEMPERATURE_C)
                    .map_err(BiomeError::Quantization)?,
                humidity: HumidDeciPct::try_from_value(noise_value * DITHER_HUMIDITY_PCT)
                    .map_err(BiomeError::Quantization)?,
            };
            let index = y * row + x;
            let cell = &world.grid.cells[index];
            biomes.push(classify(
                CellInput {
                    is_water: cell.is_water,
                    height: cell.height,
                    temperature: temperatures[index],
                    humidity: humidities[index],
                },
                dither,
            ));
        }
    }

    world.biomes = BiomeSection {
        biome: Some(biomes),
    };
    Ok(())
}

#[cfg(test)]
impl CellInput {
    /// Test helper: the same cell at a different height.
    fn swap_height(self, height: i32) -> Self {
        Self {
            height: HeightM(height),
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{BiomeCategory, entry};

    fn land(temperature: i16, humidity: i16) -> CellInput {
        CellInput {
            is_water: false,
            height: HeightM(100),
            temperature: TempDeciC(temperature),
            humidity: HumidDeciPct(humidity),
        }
    }

    fn water(temperature: i16, height: i32) -> CellInput {
        CellInput {
            is_water: true,
            height: HeightM(height),
            temperature: TempDeciC(temperature),
            humidity: HumidDeciPct(1000),
        }
    }

    #[test]
    fn water_rules_cover_freezing_floe_and_depth() {
        // Bounds in lattice steps: −50 steps = −5 °C, +50 steps = +5 °C.
        assert_eq!(classify(water(-100, -10), Dither::NONE), FROZEN_OCEAN);
        assert_eq!(classify(water(-51, -10), Dither::NONE), FROZEN_OCEAN);
        assert_eq!(classify(water(-49, -10), Dither::NONE), ICY_OCEAN);
        assert_eq!(classify(water(49, -10), Dither::NONE), ICY_OCEAN);
        assert_eq!(classify(water(51, -2000), Dither::NONE), DEEP_OCEAN);
        assert_eq!(classify(water(51, -10), Dither::NONE), OCEAN);
    }

    #[test]
    fn mountain_tier_precedes_climate() {
        assert_eq!(
            classify(land(100, 1000).swap_height(4000), Dither::NONE),
            ROCKY_MOUNTAIN
        );
        assert_eq!(
            classify(land(30, 1000).swap_height(4000), Dither::NONE),
            GLACIAL_MOUNTAIN
        );
        assert_eq!(
            classify(land(-10, 1000).swap_height(3000), Dither::NONE),
            GLACIAL_MOUNTAIN
        );
        assert_eq!(
            classify(land(100, 1000).swap_height(3000), Dither::NONE),
            ROCKY_MOUNTAIN
        );
        // Below the mountain band the climate tier decides.
        assert_eq!(
            classify(land(300, 1000).swap_height(100), Dither::NONE),
            TROPICAL_RAINFOREST
        );
    }

    #[test]
    fn every_climate_rule_is_reachable() {
        // Probe values derived from the table itself: one step below
        // each bound. The rules are ordered by ascending temperature,
        // so a probe never falls into an earlier row.
        for rule in CLIMATE_RULES {
            let temperature = rule.max_temperature.unwrap_or(TEMPERATE_BELOW_C + 120) - 1;
            let humidity = rule.max_humidity.unwrap_or(1000) - 1;
            let produced = classify(land(temperature, humidity), Dither::NONE);
            assert_eq!(
                produced,
                rule.biome,
                "rule for {} is shadowed",
                entry(rule.biome).expect("registered").name
            );
        }
        // The fallback outside the table: the hot, wet corner.
        assert_eq!(
            classify(land(TEMPERATE_BELOW_C + 120, 1000), Dither::NONE),
            TROPICAL_RAINFOREST
        );
    }

    #[test]
    fn dither_shifts_boundary_decisions() {
        // Temperature wobble: one step into the cold band, wet cell →
        // Taiga; a cold wobble of −2 °C pushes the band edge above the
        // cell, dropping it into the temperate belt.
        let cold = land(-1, 500);
        assert_eq!(classify(cold, Dither::NONE), TAIGA);
        assert_eq!(
            classify(
                cold,
                Dither {
                    temperature: TempDeciC(-20),
                    humidity: HumidDeciPct(0)
                }
            ),
            TEMPERATE_FOREST
        );
        // Humidity wobble: a cell just past the tundra/taiga moisture
        // bound flips biomes under a +5 % wobble.
        let wet_edge = land(-1, 410);
        assert_eq!(classify(wet_edge, Dither::NONE), TAIGA);
        assert_eq!(
            classify(
                wet_edge,
                Dither {
                    temperature: TempDeciC(0),
                    humidity: HumidDeciPct(20)
                }
            ),
            TUNDRA
        );
    }

    #[test]
    fn rules_reference_live_entries_of_their_tier() {
        for rule in CLIMATE_RULES {
            let entry = entry(rule.biome).expect("climate rule biome is registered");
            assert!(!entry.deprecated, "{} is deprecated", entry.name);
            assert_eq!(entry.category, BiomeCategory::Climate);
        }
        for &(_, _glacial_below, glacial, rocky) in MOUNTAIN_RULES {
            for biome in [glacial, rocky] {
                let entry = entry(biome).expect("mountain rule biome is registered");
                assert!(!entry.deprecated);
                assert_eq!(entry.category, BiomeCategory::Mountain);
            }
        }
        for &(_, biome) in WATER_RULES {
            let entry = entry(biome).expect("water rule biome is registered");
            assert!(!entry.deprecated);
            assert_eq!(entry.category, BiomeCategory::Water);
        }
        for biome in [DEEP_OCEAN, OCEAN] {
            let entry = entry(biome).expect("depth rule biome is registered");
            assert!(!entry.deprecated);
            assert_eq!(entry.category, BiomeCategory::Water);
        }
        assert_eq!(
            entry(TROPICAL_RAINFOREST).expect("registered").category,
            BiomeCategory::Climate
        );
    }

    #[test]
    fn deprecated_biomes_are_never_produced() {
        // Sweep the whole physical input space densely (zero dither):
        // everything the classification produces must be a known,
        // produced registry entry. Vacuously true for the initial set
        // (no deprecations yet), and it guards every future retirement.
        let mut produced = Vec::new();
        for temperature in (-300..=600).step_by(25) {
            for humidity in (0..=1000).step_by(50) {
                for height in [-5000, -1001, -999, -1, 0, 1, 2000, 3000, 5000] {
                    let input = CellInput {
                        is_water: height < 0,
                        height: HeightM(height),
                        temperature: TempDeciC(temperature),
                        humidity: HumidDeciPct(humidity),
                    };
                    produced.push(classify(input, Dither::NONE));
                }
            }
        }
        produced.sort_unstable();
        produced.dedup();
        for id in produced {
            let entry = entry(id).expect("produced biomes must be registered");
            assert!(!entry.deprecated, "{} must not be produced", entry.name);
        }
    }
}
