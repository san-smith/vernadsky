//! River network: flow accumulation and source-to-mouth paths.
//!
//! The flow model is a port of the mapgen prototype: cells sorted by
//! height (descending), each passing its flow to the lowest of its 8
//! neighbors; ice produces no flow, deserts evaporate half of the
//! passing flow. Cells draining at least the river threshold carry a
//! river. Paths run from each network source down the flow graph until
//! the first water cell — converging rivers share the downstream cells,
//! and every path keeps its full length to its mouth.

use vernadsky_core::idgen::IdAssigner;
use vernadsky_core::quant::CentiScalar;
use vernadsky_core::schema::CellRecord;
use vernadsky_core::{Anchor, BiomeId, CellId, RiverId};

use crate::HydrologyError;

/// The absolute floor of the drainage threshold, in cells' worth of
/// starting flow: the mapgen port value. The floor only binds on tiny
/// worlds — a land-share threshold would drop below one cell there and
/// dissolve the river network (the synthetic fixture's drainage caps at
/// roughly the island radius, ~13 cells).
pub(crate) const THRESHOLD_FLOOR: f64 = 8.0;

/// The drainage threshold of a river cell for one land component: the
/// maximum of the absolute floor and the configured share of the
/// component's land cells. Per-component scaling keeps every landmass's
/// river network proportional to its own size — a fragmented
/// archipelago keeps rivers, and a supercontinent does not drown in
/// them. The share keeps the network consistent across grid
/// resolutions; the floor keeps tiny worlds from dissolving it.
///
/// Flow units approximate cells' worth of starting flow (deserts
/// evaporate, ice never starts), so the land-cell count is the natural
/// base; this is an approximation by design and documented as such.
pub(crate) fn river_threshold(share: CentiScalar, component_land_cells: usize) -> f64 {
    let fraction = f64::from(share.0) * CentiScalar::QUANTUM / 100.0;
    (fraction * component_land_cells as f64).max(THRESHOLD_FLOOR)
}

/// The river tuning of one generation run: which registry biomes
/// produce no flow or evaporate it. The drainage threshold travels
/// separately as a per-cell slice — [`river_threshold`] derives it per
/// land component from the configured share.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RiverConfig {
    /// The biome whose cells produce no flow.
    pub ice: BiomeId,
    /// The biome that evaporates half of the passing flow.
    pub desert: BiomeId,
}

/// The eight neighbor offsets, in fixed scan order (the map is seamless
/// along x, the poles clamped).
const DIRECTIONS: [(i64, i64); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// A traced river: the stable identifier, the mouth anchor (the last
/// land cell before the water), and the member cell indices ordered from
/// the source towards the mouth.
pub(crate) struct RiverPath {
    pub id: RiverId,
    pub mouth: Anchor,
    pub cells: Vec<usize>,
}

/// Accumulates the flow over `cells` and extracts the river paths.
///
/// `ice` cells produce no flow; `desert` cells evaporate half of the
/// passing flow. Land cells draining at least `river_threshold` units
/// carry a river. Paths that end in a land pit instead of a water cell
/// are dropped — an endorheic basin is a documented v0 limit.
pub(crate) fn network(
    cells: &[CellRecord],
    biomes: &[BiomeId],
    width: u32,
    height: u32,
    config: RiverConfig,
    cell_threshold: &[f64],
    assigner: &mut IdAssigner,
) -> Result<Vec<RiverPath>, HydrologyError> {
    let RiverConfig { ice, desert } = config;
    let w = width as usize;
    let h = height as usize;
    let cell_count = cells.len();

    // --- Flow accumulation ---
    let mut flow = vec![1.0f64; cell_count];
    let mut targets = vec![usize::MAX; cell_count];
    let mut order: Vec<usize> = (0..cell_count).collect();
    // High ground first, canonical tie-break by cell index (ADR-0004
    // §5.5: sort decisions never depend on raw float orders).
    order.sort_by_key(|&index| (std::cmp::Reverse(cells[index].height.0), index as u64));
    for &index in &order {
        if biomes[index] == ice {
            flow[index] = 0.0;
            continue;
        }
        let y = index / w;
        let x = index % w;
        let mut lowest = cells[index].height.0;
        let mut target = usize::MAX;
        for &(dy, dx) in &DIRECTIONS {
            let ny = y as i64 + dy;
            if ny < 0 || ny >= h as i64 {
                continue;
            }
            let nx = (x as i64 + dx).rem_euclid(w as i64) as usize;
            let neighbor = ny as usize * w + nx;
            if cells[neighbor].height.0 < lowest {
                lowest = cells[neighbor].height.0;
                target = neighbor;
            }
        }
        if target != usize::MAX {
            targets[index] = target;
            let evaporation = if biomes[index] == desert { 0.5 } else { 1.0 };
            flow[target] += flow[index] * evaporation;
        }
    }

    // --- River cells and their sources ---
    let is_river: Vec<bool> = (0..cell_count)
        .map(|index| !cells[index].is_water && flow[index] >= cell_threshold[index])
        .collect();
    let mut has_tributary = vec![false; cell_count];
    for index in 0..cell_count {
        if is_river[index] {
            let target = targets[index];
            if target != usize::MAX {
                has_tributary[target] = true;
            }
        }
    }
    let sources: Vec<usize> = (0..cell_count)
        .filter(|&index| is_river[index] && !has_tributary[index])
        .collect();

    // --- Trace every source down the flow graph to the water ---
    let mut traced: Vec<(Anchor, Vec<usize>)> = Vec::new();
    for source in sources {
        let mut path = Vec::new();
        let mut current = source;
        let mouth = loop {
            path.push(current);
            let next = targets[current];
            if next == usize::MAX {
                break None; // a land pit: never reaches the water
            }
            if cells[next].is_water {
                break Some(current); // the mouth is the last land cell
            }
            current = next;
        };
        if let Some(mouth_cell) = mouth {
            let (x, y) = CellId(mouth_cell as u64).to_xy(width);
            traced.push((
                Anchor {
                    x: i64::from(x),
                    y: i64::from(y),
                },
                path,
            ));
        }
    }

    // --- Canonical identifier assignment ---
    traced.sort_by_key(|(mouth, _)| (mouth.y, mouth.x));
    let mut paths = Vec::with_capacity(traced.len());
    for (mouth, cells) in traced {
        let id = assigner
            .river(mouth.x, mouth.y)
            .map_err(HydrologyError::IdGeneration)?;
        paths.push(RiverPath { id, mouth, cells });
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vernadsky_core::NO_INDEX;
    use vernadsky_core::quant::HeightM;

    /// A 7×7 world: water border, a radial hill of the given peak height
    /// in the center (height decays by 45 m per Chebyshev ring, so the
    /// outer ring is water and the 5×5 core is land).
    fn cone_world(peak: i32) -> Vec<CellRecord> {
        let mut cells = Vec::new();
        for y in 0..7u32 {
            for x in 0..7u32 {
                let ring = 3i32 - (x as i32 - 3).abs().max((y as i32 - 3).abs());
                let height = peak - ring * 45;
                cells.push(CellRecord {
                    height: HeightM(height.max(-1)),
                    is_water: height.max(-1) <= 0,
                    territory: 0,
                    water_body: NO_INDEX,
                });
            }
        }
        cells
    }

    fn all_land_biomes(cells: &[CellRecord], biome: BiomeId) -> Vec<BiomeId> {
        cells
            .iter()
            .map(|cell| if cell.is_water { BiomeId(1) } else { biome })
            .collect()
    }

    #[test]
    fn the_cone_drains_rivers_to_the_coast() {
        let cells = cone_world(120);
        let biomes = all_land_biomes(&cells, BiomeId(10)); // Grassland everywhere
        let mut assigner = IdAssigner::new();
        let cell_thresholds = vec![3.0; 7 * 7];
        let paths = network(
            &cells,
            &biomes,
            7,
            7,
            RiverConfig {
                ice: BiomeId(5),
                desert: BiomeId(13),
            },
            &cell_thresholds,
            &mut assigner,
        )
        .expect("paths");
        assert!(!paths.is_empty(), "a cone island must carry rivers");
        for path in &paths {
            assert!(!path.cells.is_empty());
            // Every cell is land and the path descends monotonically.
            for pair in path.cells.windows(2) {
                assert!(!cells[pair[0]].is_water);
                assert!(cells[pair[1]].height.0 <= cells[pair[0]].height.0);
            }
            // The mouth is land and touches water.
            let mouth = path.cells[path.cells.len() - 1];
            assert!(!cells[mouth].is_water);
            let (mx, my) = (mouth % 7, mouth / 7);
            let touches_water = DIRECTIONS.iter().any(|&(dy, dx)| {
                let ny = my as i64 + dy;
                if ny < 0 || ny >= 7 {
                    return false;
                }
                let nx = (mx as i64 + dx).rem_euclid(7) as usize;
                cells[ny as usize * 7 + nx].is_water
            });
            assert!(touches_water, "the mouth must touch the water");
        }
    }

    #[test]
    fn ice_produces_no_rivers() {
        let cells = cone_world(120);
        let biomes = all_land_biomes(&cells, BiomeId(5)); // Ice everywhere on land
        let mut assigner = IdAssigner::new();
        let cell_thresholds = vec![3.0; 7 * 7];
        let paths = network(
            &cells,
            &biomes,
            7,
            7,
            RiverConfig {
                ice: BiomeId(5),
                desert: BiomeId(13),
            },
            &cell_thresholds,
            &mut assigner,
        )
        .expect("paths");
        assert!(paths.is_empty(), "frozen ground must not carry rivers");
    }

    #[test]
    fn identical_input_yields_identical_paths() {
        let cells = cone_world(200);
        let biomes = all_land_biomes(&cells, BiomeId(10));
        let cell_thresholds = vec![3.0; 7 * 7];
        let run = |assigner: &mut IdAssigner| {
            network(
                &cells,
                &biomes,
                7,
                7,
                RiverConfig {
                    ice: BiomeId(5),
                    desert: BiomeId(13),
                },
                &cell_thresholds,
                assigner,
            )
            .expect("paths")
            .into_iter()
            .map(|path| (path.id.0, path.mouth.clone(), path.cells))
            .collect::<Vec<_>>()
        };
        let mut first = IdAssigner::new();
        let mut second = IdAssigner::new();
        assert_eq!(run(&mut first), run(&mut second));
    }
}
