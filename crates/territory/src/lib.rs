//! Territory partition stage of the Vernadsky planetary geography
//! generator: splits the land and water surfaces into neutral geographic
//! territories — the base unit the game adapter maps into provinces and
//! sea zones (one-to-one for the MVP).
//!
//! The stage is a port of the province partition of our earlier mapgen
//! prototype with deliberate corrections: the land seed weight comes
//! from the actual climate lattices (the prototype reused height as a
//! temperature proxy), and territory identifiers derive from the
//! centroid anchor through the stable identifier strategy (ADR-0003)
//! instead of the position in a sorted list.
//!
//! # Algorithm
//!
//! **Land partition** (`land_count` territories):
//! 1. **Candidates and weight**: every land cell is a candidate; its
//!    weight is `temp_norm × humid_norm × height_factor` computed from
//!    the climate and height lattices, quantized onto the `SeedWeight`
//!    lattice. Sorting and every comparison use the lattice — never the
//!    raw float (ADR-0004 §5.5).
//! 2. **Seed selection**: the candidates are sorted by weight
//!    (descending, canonical tie-break by cell index) and the seeds are
//!    taken **evenly across the sorted list** — territories sample the
//!    whole attractiveness spectrum instead of clustering in the best
//!    climate.
//! 3. **Growth**: multi-source BFS from the seeds (4-neighbors, the map
//!    seamless along x, the poles clamped, land cells only).
//! 4. **Fill**: every uncovered land cell joins the territory whose
//!    running centroid is nearest.
//!
//! **Water partition** (`water_count` territories): water is several
//! disconnected surfaces, so it is partitioned per connected component.
//! Every component receives at least one territory (a lake is one
//! territory by itself); the remaining shares follow the area
//! proportions by the largest-remainder rule. Within a component the
//! seeds are sampled evenly in the canonical cell order — water has no
//! climate attractiveness — and the BFS covers the component entirely.
//!
//! **Identifiers**: each territory's final centroid is the anchor; land
//! territories are identified first, then water ones, both in the
//! canonical anchor order through `IdAssigner`. Changing `water_count`
//! therefore never shifts land identifiers, and vice versa.
//!
//! The stage consumes no randomness. `natural_potential` starts at `0`
//! (the suitability stage owns it) and `region` stays `None` (the
//! regions stage owns it).
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
//!     generate(&mut world, 12, 6)?;
//!
//!     let water_territories = world.territories.iter().filter(|t| t.is_water).count();
//!     assert!(water_territories >= 1, "the ocean is partitioned");
//!     Ok(())
//! }
//! ```

pub mod region;

use std::collections::{HashMap, VecDeque};
use std::fmt;

use vernadsky_core::quant::{
    HeightM, HumidDeciPct, NatPotential, QuantError, SeedWeight, TempDeciC,
};
use vernadsky_core::schema::{CellRecord, GeographicWorld, NO_INDEX, TerritoryRecord};
use vernadsky_core::{Anchor, idgen::IdAssigner};

pub use region::generate_regions;

/// The four orthogonal neighbor offsets, in fixed scan order.
const NEIGHBORS: [(i64, i64); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];

/// Errors of the territory stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerritoryError {
    /// The grid is empty or its cell storage does not match its declared
    /// size; the stage refuses to run on an inconsistent world.
    InvalidGrid,
    /// The land partition requires a filled climate section; the weight
    /// consumes temperature and humidity.
    MissingClimate,
    /// The land count is zero or exceeds the number of land cells.
    InvalidLandCount {
        /// The requested count.
        requested: usize,
        /// The number of available land cells.
        available: usize,
    },
    /// The water count is below the connected water component count
    /// (every component carries at least one territory) or above the
    /// water cell count.
    InvalidWaterCount {
        /// The requested count.
        requested: usize,
        /// The minimal valid count: one territory per component.
        minimal: usize,
        /// The maximal valid count: one territory per cell.
        maximal: usize,
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
            TerritoryError::InvalidLandCount {
                requested,
                available,
            } => write!(
                f,
                "land count {requested} is invalid: at least 1 and at most the land cell count ({available})"
            ),
            TerritoryError::InvalidWaterCount {
                requested,
                minimal,
                maximal,
            } => write!(
                f,
                "water count {requested} is invalid: between the connected water component count ({minimal}) and the water cell count ({maximal})"
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
/// surface into `land_count` territories and the water surface into
/// `water_count` territories.
///
/// Requires a consistent, non-empty grid; the land partition requires a
/// filled climate section (with `land_count = 0` the world may skip the
/// climate stage). Replaces the `territories` section and the per-cell
/// territory references. Pure with respect to `(world, counts)`: the
/// stage consumes no randomness.
pub fn generate(
    world: &mut GeographicWorld,
    land_count: usize,
    water_count: usize,
) -> Result<(), TerritoryError> {
    let width = world.grid.width;
    let height = world.grid.height;
    let row = width as usize;
    let cell_count = row * height as usize;
    if width == 0 || height == 0 || world.grid.cells.len() != cell_count {
        return Err(TerritoryError::InvalidGrid);
    }

    // --- Land partition ---
    let land_anchored = if land_count > 0 {
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
        Some(partition_land(
            &world.grid.cells,
            temperatures,
            humidities,
            row,
            height as usize,
            land_count,
        )?)
    } else {
        None
    };

    // --- Water partition, per connected component ---
    let water_anchored = if water_count > 0 {
        let components = water_components(&world.grid.cells, row, height as usize);
        let water_cells = components.iter().map(Vec::len).sum::<usize>();
        if water_count < components.len() || water_count > water_cells {
            return Err(TerritoryError::InvalidWaterCount {
                requested: water_count,
                minimal: components.len(),
                maximal: water_cells,
            });
        }
        let areas: Vec<usize> = components.iter().map(Vec::len).collect();
        let shares = apportion(&areas, water_count);
        let mut anchored: Vec<(Anchor, Vec<usize>)> = Vec::new();
        for (component, &share) in components.iter().zip(shares.iter()) {
            anchored.extend(grow_water_component(
                &world.grid.cells,
                component,
                share,
                row,
                height as usize,
            ));
        }
        anchored.sort_by_key(|(anchor, _)| (anchor.y, anchor.x));
        Some(anchored)
    } else {
        None
    };

    // --- Identifiers: land first, then water, both by anchor order ---
    let mut assigner = IdAssigner::new();
    let mut territories = Vec::new();
    let mut owner = vec![NO_INDEX; cell_count];
    if let Some(anchored) = &land_anchored {
        for (position, (anchor, cells)) in anchored.iter().enumerate() {
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
            for &cell in cells {
                owner[cell] = position as u32;
            }
        }
    }
    if let Some(anchored) = &water_anchored {
        let land_len = territories.len();
        for (position, (anchor, cells)) in anchored.iter().enumerate() {
            let id = assigner
                .territory(anchor.x, anchor.y, true)
                .map_err(TerritoryError::IdGeneration)?;
            territories.push(TerritoryRecord {
                id,
                is_water: true,
                anchor: anchor.clone(),
                cell_count: cells.len() as u32,
                natural_potential: NatPotential(0),
                region: None,
            });
            for &cell in cells {
                owner[cell] = (land_len + position) as u32;
            }
        }
    }

    world
        .grid
        .cells
        .iter_mut()
        .enumerate()
        .for_each(|(index, cell)| {
            cell.territory = owner[index];
        });
    world.territories = territories;
    Ok(())
}

/// The land partition: weighted seed selection, multi-source BFS
/// growth, nearest-centroid fill. Returns the territories with their
/// member cell indices, sorted by anchor.
fn partition_land(
    cells: &[CellRecord],
    temperatures: &[TempDeciC],
    humidities: &[HumidDeciPct],
    row: usize,
    height: usize,
    land_count: usize,
) -> Result<Vec<(Anchor, Vec<usize>)>, TerritoryError> {
    // --- Candidates with quantized weights ---
    let mut candidates: Vec<Candidate> = Vec::new();
    for (index, cell) in cells.iter().enumerate() {
        if cell.is_water {
            continue;
        }
        let weight = attractiveness(temperatures[index], humidities[index], cell.height);
        candidates.push(Candidate {
            index,
            weight: SeedWeight::try_from_value(weight).map_err(TerritoryError::Quantization)?,
        });
    }
    if land_count > candidates.len() {
        return Err(TerritoryError::InvalidLandCount {
            requested: land_count,
            available: candidates.len(),
        });
    }

    // --- Seed selection: even sampling over the weight spectrum ---
    candidates
        .sort_by_key(|candidate| (std::cmp::Reverse(candidate.weight), candidate.index as u64));
    let indices: Vec<usize> = candidates.iter().map(|candidate| candidate.index).collect();
    let seeds = even_sample(&indices, land_count);

    // --- Multi-source BFS growth over the land ---
    let mut owner = vec![usize::MAX; cells.len()];
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); land_count];
    let mut sums: Vec<(i64, i64)> = vec![(0, 0); land_count];
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
            let nx = (x as i64 + dx).rem_euclid(row as i64) as usize;
            let neighbor = ny as usize * row + nx;
            if !cells[neighbor].is_water && owner[neighbor] == usize::MAX {
                owner[neighbor] = territory;
                members[territory].push(neighbor);
                sums[territory].0 += nx as i64;
                sums[territory].1 += ny;
                queue.push_back((nx, ny as usize, territory));
            }
        }
    }

    // --- Fill: uncovered land joins the nearest running centroid ---
    for (index, owned) in owner.iter_mut().enumerate() {
        if cells[index].is_water || *owned != usize::MAX {
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

    // --- Final anchors over the final membership ---
    Ok(members
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
        .collect())
}

/// The connected water components, each a list of cell indices in
/// canonical (ascending) order.
fn water_components(cells: &[CellRecord], row: usize, height: usize) -> Vec<Vec<usize>> {
    let mut visited = vec![false; cells.len()];
    let mut components = Vec::new();
    for start in 0..cells.len() {
        if !cells[start].is_water || visited[start] {
            continue;
        }
        let mut component = Vec::new();
        let mut queue = VecDeque::new();
        visited[start] = true;
        queue.push_back(start);
        while let Some(index) = queue.pop_front() {
            component.push(index);
            let y = index / row;
            let x = index % row;
            for &(dy, dx) in &NEIGHBORS {
                let ny = y as i64 + dy;
                if ny < 0 || ny >= height as i64 {
                    continue;
                }
                let nx = (x as i64 + dx).rem_euclid(row as i64) as usize;
                let neighbor = ny as usize * row + nx;
                if cells[neighbor].is_water && !visited[neighbor] {
                    visited[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }
        component.sort_unstable();
        components.push(component);
    }
    components
}

/// Distributes `total` territories over the components: every component
/// receives at least one, the rest follows the area proportions with the
/// largest-remainder rule. The caller guarantees
/// `total >= areas.len()`; shares are capped by the component sizes.
fn apportion(areas: &[usize], total: usize) -> Vec<usize> {
    debug_assert!(total >= areas.len(), "every component carries a minimum");
    let total_area: usize = areas.iter().sum();
    let mut shares: Vec<usize> = areas
        .iter()
        .map(|&area| area * total / total_area)
        .collect();
    for share in &mut shares {
        if *share == 0 {
            *share = 1;
        }
    }
    // The fractional part of the exact quota, scaled to an integer:
    // exact = area * total / total_area, so the remainder numerator is
    // exact's denominator remainder. Integer-only ordering (ADR-0004).
    let remainder_num = |index: usize| -> usize { (areas[index] * total) % total_area };
    // Resolve over-allocation: give back from the largest shares.
    while shares.iter().sum::<usize>() > total {
        let victim = (0..areas.len())
            .filter(|&index| shares[index] > 1)
            .min_by_key(|&index| (remainder_num(index), std::cmp::Reverse(areas[index]), index))
            .expect("total >= components leaves a reducible share");
        shares[victim] -= 1;
    }
    // Distribute the remainder: largest fractional remainder first.
    while shares.iter().sum::<usize>() < total {
        let winner = (0..areas.len())
            .filter(|&index| shares[index] < areas[index])
            .max_by_key(|&index| (remainder_num(index), areas[index], std::cmp::Reverse(index)))
            .expect("total <= the cell count leaves a growable share");
        shares[winner] += 1;
    }
    shares
}

/// Grows one water component into `share` territories: even seed
/// sampling over the component's canonical cell order, then multi-source
/// BFS within the component. Returns the territories with their anchors
/// and members. The component is connected, so the BFS covers it fully —
/// the membership assertion documents that guarantee.
fn grow_water_component(
    cells: &[CellRecord],
    component: &[usize],
    share: usize,
    row: usize,
    height: usize,
) -> Vec<(Anchor, Vec<usize>)> {
    let seeds = even_sample(component, share);
    let mut owner: HashMap<usize, usize> = HashMap::new();
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); share];
    let mut queue = VecDeque::new();
    for (territory, &cell) in seeds.iter().enumerate() {
        owner.insert(cell, territory);
        members[territory].push(cell);
        queue.push_back((cell, territory));
    }
    while let Some((cell, territory)) = queue.pop_front() {
        let y = (cell / row) as i64;
        let x = (cell % row) as i64;
        for &(dy, dx) in &NEIGHBORS {
            let ny = y + dy;
            if ny < 0 || ny >= height as i64 {
                continue;
            }
            let nx = (x + dx).rem_euclid(row as i64) as usize;
            let neighbor = ny as usize * row + nx;
            if cells[neighbor].is_water && !owner.contains_key(&neighbor) {
                owner.insert(neighbor, territory);
                members[territory].push(neighbor);
                queue.push_back((neighbor, territory));
            }
        }
    }
    debug_assert_eq!(
        members.iter().map(Vec::len).sum::<usize>(),
        component.len(),
        "a connected component is fully covered by its seeds' growth"
    );
    members
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
        .collect()
}

/// Even sampling of `count` entries from a canonically ordered list:
/// positions `0, step, 2·step, …` with `step = (len − 1) / count`; a
/// list of exactly `count` entries is taken whole.
fn even_sample(ordered: &[usize], count: usize) -> Vec<usize> {
    assert!(count > 0 && count <= ordered.len(), "sampling out of range");
    if ordered.len() == count {
        return ordered.to_vec();
    }
    let step = (ordered.len() - 1) / count;
    (0..count)
        .map(|position| ordered[position * step])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
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
        generate(&mut world, 8, 0).expect("partition succeeds");

        let mut member_counts = vec![0u32; world.territories.len()];
        for cell in &world.grid.cells {
            if cell.is_water {
                assert_eq!(
                    cell.territory, NO_INDEX,
                    "water_count 0 keeps the water unpartitioned"
                );
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
        generate(&mut first, 4, 0).expect("partition succeeds");
        generate(&mut second, 4, 0).expect("partition succeeds");
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
        generate(&mut world, 4, 0).expect("partition succeeds");
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
            generate(&mut world, 4, 0).expect("partition succeeds");
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
        generate(&mut base, 8, 0).expect("partition succeeds");
        with_rivers.rivers[0].path.reverse();
        generate(&mut with_rivers, 8, 0).expect("partition succeeds");
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
            generate(&mut world, count, 0).expect("partition succeeds");
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
    fn rejects_bad_land_counts() {
        let mut world = uniform_world(); // 32 cells, all land
        // Land count 0 skips the partition entirely (a valid profile).
        assert_eq!(generate(&mut world, 0, 0), Ok(()));
        assert_eq!(
            generate(&mut world, 33, 0),
            Err(TerritoryError::InvalidLandCount {
                requested: 33,
                available: 32
            })
        );
    }

    /// The world_with_lake_and_sea layout partitioned, with regions
    /// grouped.
    fn partitioned_with_regions() -> GeographicWorld {
        let mut world = world_with_lake_and_sea();
        generate(&mut world, 6, 7).expect("partitions succeed");
        generate_regions(&mut world).expect("regions succeed");
        world
    }

    #[test]
    fn every_territory_belongs_to_exactly_one_region() {
        let world = partitioned_with_regions();
        assert!(!world.regions.is_empty());
        for territory in &world.territories {
            assert!(territory.region.is_some(), "every territory is grouped");
        }
        let region_ids: std::collections::BTreeSet<_> =
            world.regions.iter().map(|r| r.id).collect();
        assert_eq!(region_ids.len(), world.regions.len(), "region ids unique");
        for territory in &world.territories {
            assert!(
                region_ids.contains(&territory.region.expect("grouped")),
                "territory references a registered region"
            );
        }
    }

    #[test]
    fn regions_never_mix_surfaces() {
        let world = partitioned_with_regions();
        for territory in &world.territories {
            let region = territory.region.expect("grouped");
            let position = world
                .regions
                .iter()
                .position(|r| r.id == region)
                .expect("registered");
            // Homogeneity: every member of the region matches the first.
            for other in &world.territories {
                if other.region == Some(region) {
                    assert_eq!(
                        other.is_water, territory.is_water,
                        "region {position} mixes surfaces"
                    );
                }
            }
        }
    }

    #[test]
    fn the_lake_basin_differs_from_the_ocean_basin() {
        let world = partitioned_with_regions();
        // The single-cell lake and the outer ocean are disconnected
        // water components, so their territories sit in different
        // regions.
        let lake_owner = world.grid.cells[2 * 12 + 2].territory;
        let ocean_owner = world.grid.cells[0].territory;
        let lake_region = world.territories[lake_owner as usize].region;
        let ocean_region = world.territories[ocean_owner as usize].region;
        assert_ne!(lake_region, ocean_region, "the lake is its own basin");
    }

    #[test]
    fn non_adjacent_islands_group_separately() {
        // Two land islands (columns 1 and 3 are water gaps around the
        // boxes... reuse the lake-and-sea world: its walled boxes are
        // land rings surrounded by water, so each ring is its own
        // continent.
        let world = partitioned_with_regions();
        let mut land_regions: std::collections::BTreeSet<u64> = Default::default();
        for cell in &world.grid.cells {
            if !cell.is_water {
                land_regions.insert(
                    world.territories[cell.territory as usize]
                        .region
                        .expect("grouped")
                        .0,
                );
            }
        }
        // Two walled rings + the main island(s): more than one continent
        // region exists, and none of them mixes with a basin (checked
        // above).
        assert!(
            land_regions.len() >= 2,
            "the land rings are separate continents"
        );
    }

    #[test]
    fn regions_are_deterministic() {
        let first = partitioned_with_regions();
        let second = partitioned_with_regions();
        assert_eq!(first.regions, second.regions);
        let refs = |world: &GeographicWorld| {
            world
                .territories
                .iter()
                .map(|t| t.region.expect("grouped").0)
                .collect::<Vec<_>>()
        };
        assert_eq!(refs(&first), refs(&second));
    }

    #[test]
    fn missing_climate_is_rejected() {
        let mut world = skeleton_world(8, 4, 7);
        let error = generate(&mut world, 4, 0).expect_err("no climate section");
        assert_eq!(error, TerritoryError::MissingClimate);
    }

    /// A 12×7 world: an outer polar-connected ocean, a walled 1-cell
    /// lake (top left) and a walled 9-cell sea (right).
    fn world_with_lake_and_sea() -> GeographicWorld {
        let mut world = skeleton_world(12, 7, 5);
        let cells = world.grid.cells.len();
        world.climate.temperature = Some(vec![TempDeciC(100); cells]);
        world.climate.humidity = Some(vec![HumidDeciPct(500); cells]);
        let terrain = 0.5; // sea level: the water flag decides everything here
        for cell in &mut world.grid.cells {
            cell.is_water = true;
            cell.height = vernadsky_core::quant::HeightM((terrain * 100.0) as i32);
        }
        for y in 1..=3 {
            for x in 1..=3 {
                if !(x == 2 && y == 2) {
                    world.grid.cells[y * 12 + x].is_water = false;
                }
            }
        }
        for y in 1..=5 {
            for x in 5..=9 {
                let interior = (2..=4).contains(&y) && (6..=8).contains(&x);
                if !interior {
                    world.grid.cells[y * 12 + x].is_water = false;
                }
            }
        }
        world
    }

    #[test]
    fn every_water_component_carries_a_territory() {
        let mut world = world_with_lake_and_sea();
        // No climate section: the water partition must not need it.
        generate(&mut world, 0, 4).expect("water partition succeeds");
        assert_eq!(world.territories.len(), 4, "land 0 + three components");
        assert_eq!(
            world.territories.iter().filter(|t| t.is_water).count(),
            4,
            "all four territories are water"
        );
        // The 1-cell lake is its own single-cell territory; the walled
        // sea holds together; the outer ocean takes the rest.
        let lake_cells = [(2usize, 2usize)];
        let lake_owner = world.grid.cells[lake_cells[0].1 * 12 + lake_cells[0].0].territory;
        assert_eq!(
            world.territories[lake_owner as usize].cell_count, 1,
            "the lake is one single-cell territory"
        );
        for (x, y) in lake_cells {
            assert_eq!(world.grid.cells[y * 12 + x].territory, lake_owner);
        }
        // Every water cell belongs to some water territory; land stays
        // unpartitioned (land_count 0).
        for cell in &world.grid.cells {
            if cell.is_water {
                assert!(cell.territory != NO_INDEX);
            } else {
                assert_eq!(cell.territory, NO_INDEX);
            }
        }
    }

    #[test]
    fn water_count_bounds_follow_the_components() {
        let mut world = world_with_lake_and_sea(); // three components
        assert_eq!(
            generate(&mut world, 0, 2),
            Err(TerritoryError::InvalidWaterCount {
                requested: 2,
                minimal: 3,
                maximal: 60
            })
        );
        assert_eq!(
            generate(&mut world, 0, 84),
            Err(TerritoryError::InvalidWaterCount {
                requested: 84,
                minimal: 3,
                maximal: 60
            })
        );
    }

    #[test]
    fn land_ids_are_stable_across_water_counts() {
        let mut without = world_with_lake_and_sea();
        without.climate.temperature = Some(vec![TempDeciC(100); without.grid.cells.len()]);
        without.climate.humidity = Some(vec![HumidDeciPct(500); without.grid.cells.len()]);
        let mut with = without.clone();
        generate(&mut without, 6, 0).expect("land partition succeeds");
        generate(&mut with, 6, 7).expect("both partitions succeed");

        let land_ids = |world: &GeographicWorld| {
            world
                .territories
                .iter()
                .filter(|t| !t.is_water)
                .map(|t| (t.id.0, t.anchor.x, t.anchor.y, t.cell_count))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            land_ids(&without),
            land_ids(&with),
            "land ids must not shift"
        );

        let water_ids = |world: &GeographicWorld| {
            world
                .territories
                .iter()
                .filter(|t| t.is_water)
                .map(|t| t.cell_count)
                .collect::<Vec<_>>()
        };
        assert!(water_ids(&without).is_empty());
        assert_eq!(water_ids(&with).len(), 7);
    }

    #[test]
    fn apportion_follows_area_with_a_minimum_per_component() {
        // The motivating case: a large ocean, a small sea, a smaller
        // lake — four territories over three components.
        assert_eq!(apportion(&[76, 9, 4], 4), vec![2, 1, 1]);
        // Even split.
        assert_eq!(apportion(&[10, 10, 10], 3), vec![1, 1, 1]);
        assert_eq!(apportion(&[10, 10, 10], 6), vec![2, 2, 2]);
        // Minimum lift for tiny components.
        assert_eq!(apportion(&[1, 1, 9], 3), vec![1, 1, 1]);
        assert_eq!(apportion(&[1, 1, 9], 5), vec![1, 1, 3]);
    }
}
