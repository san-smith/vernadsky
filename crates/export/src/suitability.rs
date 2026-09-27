//! Suitability scoring and the neutral geographic facts of territories.
//!
//! ## Natural potential (agricultural score, 0…1000 ‰)
//!
//! Per land cell, the product of three factors in `[0, 1]`:
//!
//! - **Temperature comfort** — a trapezoid over the annual mean:
//!   zero at −5 °C and below, one in the 10…25 °C belt, zero again at
//!   +45 °C and above (frost and heat limit agriculture).
//! - **Moisture** — linear from 0 at 0 % relative humidity to 1 at
//!   70 % (plateau above; the hydrology model caps the humidity field
//!   near saturation anyway).
//! - **Lowland** — `1 − min(|m|, 3000) / 3000`: lowlands suit crops,
//!   highlands do not (the same 3000 m reference the biome stage uses
//!   for its mountain tier).
//!
//! A territory's score is the mean over its land cells, scaled to
//! permille on the [`NatPotential`] lattice. The water surface is not
//! scored: water territories keep the zero default until a consumer
//! defines marine suitability.

use vernadsky_core::quant::{HeightM, HumidDeciPct, NatPotential, TempDeciC};
use vernadsky_core::schema::{GeographicWorld, NO_INDEX, WaterBodyKind};
use vernadsky_core::{Anchor, BiomeId};

use crate::ExportError;

/// Temperature comfort: zero below −5 °C, one in the 10…25 °C belt,
/// zero above +45 °C.
fn temperature_comfort(deci_celsius: i16) -> f64 {
    let t = f64::from(deci_celsius) / 10.0;
    if t <= -5.0 || t >= 45.0 {
        0.0
    } else if t < 10.0 {
        (t + 5.0) / 15.0
    } else if t <= 25.0 {
        1.0
    } else {
        (45.0 - t) / 20.0
    }
}

/// Moisture: linear to the 70 % saturation mark.
fn moisture_factor(deci_percent: i16) -> f64 {
    (f64::from(deci_percent) / 10.0 / 70.0).clamp(0.0, 1.0)
}

/// Lowland factor: one at sea level, zero at 3000 m and above.
fn lowland_factor(height: HeightM) -> f64 {
    (1.0 - height.to_value().abs() / 3000.0).clamp(0.0, 1.0)
}

/// The suitability score of one land cell, in `[0, 1]`.
fn cell_score(temperature: TempDeciC, humidity: HumidDeciPct, height: HeightM) -> f64 {
    temperature_comfort(temperature.0) * moisture_factor(humidity.0) * lowland_factor(height)
}

/// The neutral geographic facts of one territory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerritoryFacts {
    /// The territory's position in `world.territories`.
    pub territory: u32,
    /// The starting natural potential (permille); water territories
    /// carry the zero default.
    pub natural_potential: NatPotential,
    /// The territory touches water: the geographic basis for a harbor.
    pub is_coastal: bool,
    /// The territory carries mountain-biome cells: the geographic basis
    /// for a pass.
    pub is_mountainous: bool,
    /// Mouths of this territory's rivers that reach an ocean or a sea:
    /// the geographic basis for an estuary.
    pub river_mouths: Vec<Anchor>,
}

/// Derives the facts and the natural potential of every territory.
///
/// Requires a consistent grid with climate and biome sections filled
/// and the territory partition applied; a world without partitioned
/// territories yields an empty result. Pure with respect to `world`.
pub fn facts(world: &GeographicWorld) -> Result<Vec<TerritoryFacts>, ExportError> {
    let cells = &world.grid.cells;
    let width = world.grid.width;
    let height = world.grid.height;
    let row = width as usize;
    let cell_count = row * height as usize;
    if width == 0 || height == 0 || world.grid.cells.len() != cell_count {
        return Err(ExportError::InvalidGrid);
    }
    let temperatures = world
        .climate
        .temperature
        .as_ref()
        .ok_or(ExportError::MissingClimate)?;
    let humidities = world
        .climate
        .humidity
        .as_ref()
        .ok_or(ExportError::MissingClimate)?;
    let biomes = world
        .biomes
        .biome
        .as_ref()
        .ok_or(ExportError::MissingBiomes)?;

    let glacial = vernadsky_biome::by_name("GlacialMountain").ok_or(ExportError::MissingBiomes)?;
    let rocky = vernadsky_biome::by_name("RockyMountain").ok_or(ExportError::MissingBiomes)?;
    let is_mountain_biome = |biome: BiomeId| biome == glacial || biome == rocky;

    // Members of each land territory, from the grid references.
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); world.territories.len()];
    for (index, cell) in world.grid.cells.iter().enumerate() {
        if !cell.is_water && cell.territory != NO_INDEX {
            members[cell.territory as usize].push(index);
        }
    }

    // River mouths: the kind of the water body the mouth touches. The
    // mouth itself is a land cell, so its neighbors carry the reference.
    let mouths: Vec<(Anchor, WaterBodyKind)> = world
        .rivers
        .iter()
        .filter_map(|river| {
            let index = river.mouth.y as usize * row + river.mouth.x as usize;
            let y = index / row;
            let x = index % row;
            NEIGHBORS.iter().find_map(|&(dy, dx)| {
                let ny = y as i64 + dy;
                if ny < 0 || ny >= height as i64 {
                    return None;
                }
                let nx = (x as i64 + dx).rem_euclid(row as i64) as usize;
                let neighbor = ny as usize * row + nx;
                let water_body = world.grid.cells[neighbor].water_body;
                if water_body == NO_INDEX {
                    return None;
                }
                world
                    .water_bodies
                    .get(water_body as usize)
                    .map(|body| (river.mouth.clone(), body.kind))
            })
        })
        .collect();

    let mut result = Vec::with_capacity(world.territories.len());
    for (position, territory) in world.territories.iter().enumerate() {
        let member_cells = &members[position];
        let is_coastal = if territory.is_water {
            false
        } else {
            member_cells.iter().any(|&index| {
                let y = index / row;
                let x = index % row;
                NEIGHBORS.iter().any(|&(dy, dx)| {
                    let ny = y as i64 + dy;
                    if ny < 0 || ny >= height as i64 {
                        return false;
                    }
                    let nx = (x as i64 + dx).rem_euclid(row as i64) as usize;
                    world.grid.cells[ny as usize * row + nx].is_water
                })
            })
        };
        let natural_potential = if territory.is_water {
            NatPotential(0)
        } else {
            let mean = member_cells
                .iter()
                .map(|&index| {
                    cell_score(temperatures[index], humidities[index], cells[index].height)
                })
                .sum::<f64>()
                / member_cells.len().max(1) as f64;
            NatPotential::try_from_value(mean.clamp(0.0, 1.0)).map_err(ExportError::Quantization)?
        };
        let mountainous = member_cells
            .iter()
            .any(|&index| is_mountain_biome(biomes[index]));
        let river_mouths = mouths
            .iter()
            .filter(|(mouth, kind)| {
                let index = mouth.y as usize * row + mouth.x as usize;
                world.grid.cells[index].territory == position as u32
                    && matches!(kind, WaterBodyKind::Ocean | WaterBodyKind::Sea)
            })
            .map(|(mouth, _)| mouth.clone())
            .collect();
        result.push(TerritoryFacts {
            territory: position as u32,
            natural_potential,
            is_coastal,
            is_mountainous: mountainous,
            river_mouths,
        });
    }
    Ok(result)
}

/// The four orthogonal neighbor offsets, in fixed scan order.
const NEIGHBORS: [(i64, i64); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperature_comfort_is_a_trapezoid() {
        assert_eq!(temperature_comfort(-100), 0.0, "deep frost");
        assert_eq!(temperature_comfort(-50), 0.0, "at the cold bound");
        assert!(
            (temperature_comfort(-25) - 1.0 / 6.0).abs() < 1e-9,
            "-2.5 °C: a third of the ramp"
        );
        assert_eq!(temperature_comfort(150), 1.0, "the comfort belt");
        assert_eq!(temperature_comfort(250), 1.0);
        assert!((temperature_comfort(350) - 0.5).abs() < 1e-9);
        assert_eq!(temperature_comfort(450), 0.0, "at the heat bound");
    }

    #[test]
    fn moisture_saturates_at_seventy_percent() {
        assert_eq!(moisture_factor(0), 0.0);
        assert!((moisture_factor(350) - 0.5).abs() < 1e-9);
        assert_eq!(moisture_factor(700), 1.0);
        assert_eq!(moisture_factor(1000), 1.0, "the plateau holds");
    }

    #[test]
    fn lowland_falls_with_altitude() {
        assert_eq!(lowland_factor(HeightM(0)), 1.0);
        assert!((lowland_factor(HeightM(1500)) - 0.5).abs() < 1e-9);
        assert_eq!(lowland_factor(HeightM(3000)), 0.0);
        assert_eq!(lowland_factor(HeightM(-1500)), 0.5, "depths count too");
    }
}
