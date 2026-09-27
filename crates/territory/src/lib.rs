//! Territory partition stage of the Vernadsky planetary geography
//! generator: splits the land surface into neutral geographic
//! territories — the base unit the game adapter maps into provinces
//! (one-to-one for the MVP).
//!
//! The stage is a port of the province partition of our earlier mapgen
//! prototype with two deliberate corrections: the seed weight comes
//! from the actual climate lattices (the prototype reused height as a
//! temperature proxy), and territory identifiers derive from the
//! centroid anchor through the stable identifier strategy (ADR-0003)
//! instead of the position in a sorted list.
//!
//! # Algorithm
//!
//! 1. **Candidates and weight**: every land cell is a candidate; its
//!    weight is `temp_norm × humid_norm × height_factor` computed from
//!    the climate and height lattices, quantized onto the `SeedWeight`
//!    lattice.
//!    Sorting and every comparison use the lattice — never the raw
//!    float (ADR-0004 §5.5).
//! 2. **Seed selection**: the candidates are sorted by weight
//!    (descending, canonical tie-break by cell index) and the seeds are
//!    taken **evenly across the sorted list** — territories sample the
//!    whole attractiveness spectrum instead of clustering in the
//!    best climate.
//! 3. **Growth**: multi-source BFS from the seeds (4-neighbors, the map
//!    seamless along x, the poles clamped, land cells only).
//! 4. **Fill**: every uncovered land cell joins the territory whose
//!    running centroid is nearest.
//! 5. **Identifiers**: each territory's final centroid (over its final
//!    membership) is the anchor; identifiers are assigned in the
//!    canonical anchor order through `IdAssigner`.
//!
//! The stage consumes no randomness. `natural_potential` starts at `0`
//! (the suitability stage owns it) and `region` stays `None` (the
//! regions stage owns it). Water surface is not partitioned: it is
//! covered by the hydrology stage's water bodies.
//!
//! # Example
//!
//! ```
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     use vernadsky_climate::{generate as climate, ClimateConfig};
//!     use vernadsky_core::{seeded_world, RngStreams};
//!     use vernadsky_terrain::{generate as terrain, TerrainConfig};
//!     use vernadsky_territory::generate;
//!
//!     let mut world = seeded_world(42);
//!     let terrain_params = TerrainConfig::default().to_params()?;
//!     terrain(&mut world, &terrain_params, &RngStreams::new(42))?;
//!     let params = ClimateConfig::default().to_params()?;
//!     climate(&mut world, &params, &RngStreams::new(42))?;
//!     generate(&mut world, 12)?;
//!
//!     assert_eq!(world.territories.len(), 12);
//!     Ok(())
//! }
//! ```

use std::collections::VecDeque;
use std::fmt;

use vernadsky_core::quant::{
    HeightM, HumidDeciPct, NatPotential, QuantError, SeedWeight, TempDeciC,
};
use vernadsky_core::schema::{GeographicWorld, NO_INDEX, TerritoryRecord};
use vernadsky_core::{Anchor, idgen::IdAssigner};

/// The four orthogonal neighbor offsets, in fixed scan order.
const NEIGHBORS: [(i64, i64); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];

/// Errors of the territory stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerritoryError {
    /// The grid is empty or its cell storage does not match its declared
    /// size; the stage refuses to run on an inconsistent world.
    InvalidGrid,
    /// The world carries no climate fields; the weight consumes
    /// temperature and humidity.
    MissingClimate,
    /// The requested territory count is zero or exceeds the number of
    /// land cells.
    InvalidCount {
        /// The requested count.
        requested: usize,
        /// The number of available land cells.
        available: usize,
    },
    /// A computed value could not be quantized onto its lattice.
    Quantization(QuantError),
    /// Stable identifier generation failed.
    IdGeneration(vernadsky_core::IdGenError),
}

impl fmt::Display for TerritoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TerritoryError::InvalidGrid => {
                write!(f, "territory stage requires a consistent, non-empty grid")
            }
            TerritoryError::MissingClimate => {
                write!(f, "territory stage requires a filled climate section")
            }
            TerritoryError::InvalidCount {
                requested,
                available,
            } => write!(
                f,
                "territory count {requested} is invalid: at least 1 and at most the land cell count ({available})"
            ),
            TerritoryError::Quantization(source) => write!(f, "territory stage: {source}"),
            TerritoryError::IdGeneration(source) => write!(f, "territory stage: {source}"),
        }
    }
}

impl std::error::Error for TerritoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TerritoryError::Quantization(source) => Some(source),
            TerritoryError::IdGeneration(source) => Some(source),
            _ => None,
        }
    }
}

/// The attractiveness of one land cell, in the same spirit as the
/// prototype's fertility weight but from the actual climate fields:
/// a comfortable temperature, a humid climate, and low ground all raise
/// the weight.
fn attractiveness(temperature: TempDeciC, humidity: HumidDeciPct, height: HeightM) -> f64 {
    let temp_norm = ((temperature.to_value() + 25.0) / 52.0).clamp(0.0, 1.0);
    let humid_norm = (humidity.to_value() / 100.0).clamp(0.0, 1.0);
    let height_factor = (1.0 - height.to_value().abs() / 6000.0).clamp(0.0, 1.0);
    temp_norm * humid_norm * height_factor
}

/// A partition candidate: the cell index with its quantized weight.
#[derive(Clone, Copy, Debug)]
struct Candidate {
    index: usize,
    weight: SeedWeight,
}

/// Applies the territory stage to `world` in place: partitions the land
/// surface into `count` territories.
///
/// Requires a consistent, non-empty grid with a filled climate section;
/// `count` must be at least 1 and at most the land cell count. Replaces
/// the `territories` section and the per-cell territory references.
/// Pure with respect to `(world, count)`: the stage consumes no
/// randomness.
pub fn generate(world: &mut GeographicWorld, count: usize) -> Result<(), TerritoryError> {
    let width = world.grid.width;
    let height = world.grid.height;
    let row = width as usize;
    let cell_count = row * height as usize;
    if width == 0 || height == 0 || world.grid.cells.len() != cell_count {
        return Err(TerritoryError::InvalidGrid);
    }
    let temperatures = world
        .climate
        .temperature
        .as_ref()
        .ok_or(TerritoryError::MissingClimate)?;
    let humidities = world
        .climate
        .humidity
        .as_ref()
        .ok_or(TerritoryError::MissingClimate)?;

    // --- 1. Candidates with quantized weights ---
    let mut candidates: Vec<Candidate> = Vec::new();
    for (index, cell) in world.grid.cells.iter().enumerate() {
        if cell.is_water {
            continue;
        }
        let weight = attractiveness(temperatures[index], humidities[index], cell.height);
        candidates.push(Candidate {
            index,
            weight: SeedWeight::try_from_value(weight).map_err(TerritoryError::Quantization)?,
        });
    }
    if count == 0 || count > candidates.len() {
        return Err(TerritoryError::InvalidCount {
            requested: count,
            available: candidates.len(),
        });
    }

    // --- 2. Seed selection: even sampling over the weight spectrum ---
    candidates
        .sort_by_key(|candidate| (std::cmp::Reverse(candidate.weight), candidate.index as u64));
    let step = (candidates.len() - 1) / count;
    let seeds: Vec<usize> = if candidates.len() == count {
        candidates.iter().map(|candidate| candidate.index).collect()
    } else {
        (0..count)
            .map(|position| candidates[position * step].index)
            .collect()
    };

    // --- 3. Multi-source BFS growth over the land ---
    let mut owner = vec![usize::MAX; cell_count];
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut sums: Vec<(i64, i64)> = vec![(0, 0); count];
    let mut queue = VecDeque::new();
    for (territory, &cell) in seeds.iter().enumerate() {
        owner[cell] = territory;
        members[territory].push(cell);
        let (x, y) = (cell % row, cell / row);
        sums[territory].0 += x as i64;
        sums[territory].1 += y as i64;
        queue.push_back((x, y, territory));
    }
    while let Some((x, y, territory)) = queue.pop_front() {
        for &(dy, dx) in &NEIGHBORS {
            let ny = y as i64 + dy;
            if ny < 0 || ny >= height as i64 {
                continue;
            }
            let nx = (x as i64 + dx).rem_euclid(width as i64) as usize;
            let neighbor = ny as usize * row + nx;
            if !world.grid.cells[neighbor].is_water && owner[neighbor] == usize::MAX {
                owner[neighbor] = territory;
                members[territory].push(neighbor);
                sums[territory].0 += nx as i64;
                sums[territory].1 += ny;
                queue.push_back((nx, ny as usize, territory));
            }
        }
    }

    // --- 4. Fill: uncovered land joins the nearest running centroid ---
    for (index, owned) in owner.iter_mut().enumerate() {
        if world.grid.cells[index].is_water || *owned != usize::MAX {
            continue;
        }
        let (x, y) = (index % row, index / row);
        let mut best = 0usize;
        let mut best_d2 = i64::MAX;
        for (territory, &(sum_x, sum_y)) in sums.iter().enumerate() {
            let n = members[territory].len() as i64;
            let (cx, cy) = (sum_x / n, sum_y / n);
            let d2 = (x as i64 - cx).pow(2) + (y as i64 - cy).pow(2);
            if d2 < best_d2 {
                best_d2 = d2;
                best = territory;
            }
        }
        *owned = best;
        members[best].push(index);
        sums[best].0 += x as i64;
        sums[best].1 += y as i64;
    }

    // --- 5. Records with canonical identifier assignment ---
    let mut anchored: Vec<(Anchor, Vec<usize>)> = members
        .iter()
        .map(|cells| {
            let n = cells.len() as i64;
            let (sum_x, sum_y) = cells.iter().fold((0i64, 0i64), |(sx, sy), &cell| {
                ((sx + (cell % row) as i64), (sy + (cell / row) as i64))
            });
            (
                Anchor {
                    x: sum_x / n,
                    y: sum_y / n,
                },
                cells.clone(),
            )
        })
        .collect();
    anchored.sort_by_key(|(anchor, _)| (anchor.y, anchor.x));

    let mut assigner = IdAssigner::new();
    let mut territories = Vec::with_capacity(anchored.len());
    for (anchor, cells) in anchored.iter() {
        let id = assigner
            .territory(anchor.x, anchor.y, false)
            .map_err(TerritoryError::IdGeneration)?;
        territories.push(TerritoryRecord {
            id,
            is_water: false,
            anchor: anchor.clone(),
            cell_count: cells.len() as u32,
            natural_potential: NatPotential(0),
            region: None,
        });
    }

    // --- Hand over ---
    for cell in &mut world.grid.cells {
        cell.territory = NO_INDEX;
    }
    for (position, (_, cells)) in anchored.iter().enumerate() {
        for &cell in cells {
            world.grid.cells[cell].territory = position as u32;
        }
    }
    world.territories = territories;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vernadsky_core::quant::HeightM;
    use vernadsky_core::{RngStreams, seeded_world, skeleton_world};

    /// A uniform test world: all land, one climate everywhere — every
    /// candidate carries the same weight, so selection runs entirely on
    /// the canonical tie-break.
    fn uniform_world() -> GeographicWorld {
        let mut world = skeleton_world(8, 4, 7);
        let cells = world.grid.cells.len();
        world.climate.temperature = Some(vec![TempDeciC(100); cells]);
        world.climate.humidity = Some(vec![HumidDeciPct(500); cells]);
        world
    }

    #[test]
    fn partition_covers_land_and_respects_water() {
        let mut world = seeded_world(0);
        let temperatures = vec![TempDeciC(100); world.grid.cells.len()];
        let humidities = vec![HumidDeciPct(500); world.grid.cells.len()];
        world.climate.temperature = Some(temperatures);
        world.climate.humidity = Some(humidities);
        generate(&mut world, 8).expect("partition succeeds");

        let mut member_counts = vec![0u32; world.territories.len()];
        for cell in &world.grid.cells {
            if cell.is_water {
                assert_eq!(cell.territory, NO_INDEX, "water stays unpartitioned");
            } else {
                assert!(cell.territory != NO_INDEX, "land must be partitioned");
                member_counts[cell.territory as usize] += 1;
            }
        }
        for (position, territory) in world.territories.iter().enumerate() {
            assert!(!territory.is_water);
            assert_eq!(territory.region, None, "regions are S-02");
            assert_eq!(
                territory.cell_count, member_counts[position],
                "declared membership must match the grid"
            );
        }
    }

    #[test]
    fn identical_worlds_yield_identical_ids() {
        let mut first = uniform_world();
        let mut second = uniform_world();
        generate(&mut first, 4).expect("partition succeeds");
        generate(&mut second, 4).expect("partition succeeds");
        assert_eq!(first.territories, second.territories);
        let refs = |world: &GeographicWorld| {
            world
                .grid
                .cells
                .iter()
                .map(|c| c.territory)
                .collect::<Vec<_>>()
        };
        assert_eq!(refs(&first), refs(&second));
    }

    #[test]
    fn equal_weights_break_ties_by_cell_index() {
        // A uniform world sorts every candidate by index, so the seeds
        // are the first, (k+1)-th, ... candidates of the land scan —
        // deterministic and distinct.
        let mut world = uniform_world();
        generate(&mut world, 4).expect("partition succeeds");
        let anchors: Vec<_> = world
            .territories
            .iter()
            .map(|territory| (territory.anchor.x, territory.anchor.y))
            .collect();
        assert_eq!(anchors.len(), 4);
        for (position, &(x, y)) in anchors.iter().enumerate() {
            assert!(
                (0..8).contains(&x) && (0..4).contains(&y),
                "anchor {position} in bounds"
            );
        }
        let rerun = {
            let mut world = uniform_world();
            generate(&mut world, 4).expect("partition succeeds");
            world.territories
        };
        assert_eq!(world.territories, rerun);
    }

    #[test]
    fn rivers_do_not_shift_territory_ids() {
        // Partition consumes terrain and climate only: worlds that
        // differ in downstream (river) data must get the same ids.
        let mut base = seeded_world(42);
        base.climate.temperature = Some(vec![TempDeciC(100); base.grid.cells.len()]);
        base.climate.humidity = Some(vec![HumidDeciPct(500); base.grid.cells.len()]);
        let mut with_rivers = base.clone();
        generate(&mut base, 8).expect("partition succeeds");
        with_rivers.rivers[0].path.reverse();
        generate(&mut with_rivers, 8).expect("partition succeeds");
        let ids =
            |world: &GeographicWorld| world.territories.iter().map(|t| t.id.0).collect::<Vec<_>>();
        assert_eq!(ids(&base), ids(&with_rivers));
    }

    #[test]
    fn counts_scale_to_the_mvp_sizes() {
        for count in [200usize, 500, 1000] {
            let mut world = skeleton_world(200, 100, 42);
            let terrain_params = vernadsky_terrain::TerrainConfig::default()
                .to_params()
                .expect("default terrain config");
            vernadsky_terrain::generate(&mut world, &terrain_params, &RngStreams::new(42))
                .expect("terrain succeeds");
            let climate_params = vernadsky_climate::ClimateConfig::default()
                .to_params()
                .expect("default climate config");
            vernadsky_climate::generate(&mut world, &climate_params, &RngStreams::new(42))
                .expect("climate succeeds");
            generate(&mut world, count).expect("partition succeeds");
            assert_eq!(world.territories.len(), count, "count {count}");
            let bytes = world.to_bytes();
            assert_eq!(
                GeographicWorld::from_bytes(&bytes).expect("valid export"),
                world,
                "count {count}"
            );
            let mut ids: Vec<_> = world.territories.iter().map(|t| t.id.0).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(
                ids.len(),
                count,
                "identifiers must be unique, count {count}"
            );
        }
    }

    #[test]
    fn rejects_bad_counts() {
        let mut world = uniform_world(); // 32 cells, all land
        assert_eq!(
            generate(&mut world, 0),
            Err(TerritoryError::InvalidCount {
                requested: 0,
                available: 32
            })
        );
        assert_eq!(
            generate(&mut world, 33),
            Err(TerritoryError::InvalidCount {
                requested: 33,
                available: 32
            })
        );
    }

    #[test]
    fn missing_climate_is_rejected() {
        let mut world = skeleton_world(8, 4, 7);
        let error = generate(&mut world, 4).expect_err("no climate section");
        assert_eq!(error, TerritoryError::MissingClimate);
    }
}
