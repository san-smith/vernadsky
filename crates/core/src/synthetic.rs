//! Deterministic constructor of seed-parameterized synthetic worlds.
//!
//! This module exists so the schema, the canonical export format, the
//! identifier machinery, and the RNG streams can be exercised end to end
//! before the real generation stages come online. It is *not* a real
//! generation stage: the terrain is an analytic island whose parameters
//! are perturbed by the world's own RNG streams.
//!
//! Stream consumption follows the `rng-streams-v1` strategy with the
//! fixture namespace `synthetic.*`:
//!
//! - `"synthetic.terrain"` — island center offset and height amplitude;
//! - `"synthetic.river"` — column of the synthetic river.
//!
//! The perturbations are bounded so that every seed yields a world that
//! satisfies the constructor invariants (two non-empty land territories,
//! one region, one ocean body, a river reaching the coast). The output
//! feeds the golden round-trip test and the golden seed manifest
//! (`crates/core/tests/`); any change here changes those artifacts and
//! must be reviewed deliberately.

use crate::id::{CellId, RegionId};
use crate::idgen::IdAssigner;
use crate::quant::{HeightM, NatPotential};
use crate::rng::RngStreams;
use crate::schema::{
    Anchor, BiomeSection, CellRecord, ClimateSection, Connectivity, GenerationParams,
    GeographicWorld, GridSection, NO_INDEX, RegionRecord, RiverRecord, TerritoryRecord,
    WaterBodyKind, WaterBodyRecord,
};
use rand_chacha::rand_core::RngCore;

/// Grid width of the synthetic world.
const WIDTH: u32 = 48;
/// Grid height of the synthetic world.
const HEIGHT: u32 = 32;
/// Nominal center of the synthetic island.
const CENTER_X: i64 = 24;
/// Nominal center of the synthetic island.
const CENTER_Y: i64 = 16;
/// Height amplitude at the island center, in meters, before perturbation.
const BASE_AMPLITUDE: f64 = 150.0;
/// Radial decay of the island slope, in meters per squared cell.
const SLOPE: f64 = 0.9;

/// Natural potential of the western synthetic territory, in permille.
const WEST_POTENTIAL: i16 = 500;
/// Natural potential of the eastern synthetic territory, in permille.
const EAST_POTENTIAL: i16 = 750;

/// Stream name: island center offset and height amplitude.
const STREAM_TERRAIN: &str = "synthetic.terrain";
/// Stream name: column of the synthetic river.
const STREAM_RIVER: &str = "synthetic.river";

/// Maps a random byte onto the integer range `[-range, range]`.
fn offset(byte: u8, range: i64) -> i64 {
    i64::from(byte) % (2 * range + 1) - range
}

/// Builds the synthetic world for `seed`.
///
/// The terrain is a radial island: heights come from a fixed analytic
/// function whose center and amplitude are perturbed by the
/// `"synthetic.terrain"` stream, computed in `f64` and quantized at the
/// hand-off; cells at or below sea level are water. The land is split
/// into two synthetic territories (western and eastern half), grouped
/// into one region; the surrounding water forms a single ocean body; one
/// river runs down a center-adjacent column.
///
/// The function is pure: identical seeds produce byte-identical exports,
/// and no stream of the world depends on any other.
pub fn seeded_world(seed: u64) -> GeographicWorld {
    let streams = RngStreams::new(seed);

    let mut terrain_bytes = [0u8; 3];
    streams
        .stream(STREAM_TERRAIN)
        .fill_bytes(&mut terrain_bytes);
    let center_x = CENTER_X + offset(terrain_bytes[0], 3);
    let center_y = CENTER_Y + offset(terrain_bytes[1], 3);
    // Amplitude stays within [0.8, 1.055] of the base, keeping the island
    // wide enough for both territories and the river on any seed.
    let amplitude = BASE_AMPLITUDE * (1000.0 + i64::from(terrain_bytes[2]) as f64) / 1000.0;

    let mut river_bytes = [0u8; 1];
    streams.stream(STREAM_RIVER).fill_bytes(&mut river_bytes);
    let river_x = (CENTER_X + offset(river_bytes[0], 4)) as u32;

    let mut cells = Vec::with_capacity((WIDTH * HEIGHT) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let dx = i64::from(x) - center_x;
            let dy = i64::from(y) - center_y;
            let squared_distance = (dx * dx + dy * dy) as f64;
            let height_f64 = amplitude - SLOPE * squared_distance;
            let is_water = height_f64 <= 0.0;
            let height = HeightM::try_from_value(height_f64).expect("synthetic height in range");
            cells.push(CellRecord {
                height,
                is_water,
                territory: NO_INDEX,
                water_body: NO_INDEX,
            });
        }
    }

    let cell_index = |x: u32, y: u32| -> usize { (y * WIDTH + x) as usize };

    // Two land territories: western and eastern halves of the island.
    let mut west: Vec<CellId> = Vec::new();
    let mut east: Vec<CellId> = Vec::new();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let cell = &cells[cell_index(x, y)];
            if !cell.is_water {
                let id = CellId::from_xy(x, y, WIDTH);
                if x < WIDTH / 2 {
                    west.push(id);
                } else {
                    east.push(id);
                }
            }
        }
    }

    // Canonical assignment order: ascending (anchor.y, anchor.x).
    let mut halves: Vec<(Anchor, Vec<CellId>, i16)> = Vec::new();
    for (members, potential) in [(west, WEST_POTENTIAL), (east, EAST_POTENTIAL)] {
        let anchor = centroid(&members);
        halves.push((anchor, members, potential));
    }
    halves.sort_by_key(|(anchor, _, _)| (anchor.y, anchor.x));

    let mut assigner = IdAssigner::new();
    let mut territory_records = Vec::with_capacity(halves.len());
    let mut territory_ids = Vec::with_capacity(halves.len());
    for (anchor, members, potential) in &halves {
        let id = assigner
            .territory(anchor.x, anchor.y, false)
            .expect("territory assignment");
        territory_ids.push(id);
        territory_records.push(TerritoryRecord {
            id,
            is_water: false,
            anchor: anchor.clone(),
            cell_count: members.len() as u32,
            natural_potential: NatPotential(*potential),
            region: None,
        });
        for member in members {
            cells[member.0 as usize].territory = (territory_records.len() - 1) as u32;
        }
    }

    // One region grouping both territories: its value is the smallest
    // member territory identifier.
    let region_id = RegionId(
        territory_ids
            .iter()
            .map(|id| id.0)
            .min()
            .expect("two territories exist"),
    );
    for record in &mut territory_records {
        record.region = Some(region_id);
    }
    let regions = vec![RegionRecord { id: region_id }];

    // One ocean body over every water cell.
    let water: Vec<CellId> = cells
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.is_water)
        .map(|(index, _)| CellId(index as u64))
        .collect();
    let water_anchor = centroid(&water);
    let ocean_id = assigner
        .water_body(water_anchor.x, water_anchor.y)
        .expect("water body assignment");
    let water_bodies = vec![WaterBodyRecord {
        id: ocean_id,
        kind: WaterBodyKind::Ocean,
        anchor: water_anchor,
    }];
    for cell_id in &water {
        cells[cell_id.0 as usize].water_body = 0;
    }

    // One river down the river column: start at the first land cell and
    // follow the column until the water line; the mouth is the last land
    // cell of the path.
    let mut path = Vec::new();
    for y in 0..HEIGHT {
        let cell = &cells[cell_index(river_x, y)];
        if cell.is_water {
            if path.is_empty() {
                continue;
            }
            break;
        }
        path.push(CellId::from_xy(river_x, y, WIDTH));
    }
    let mouth_cell = *path.last().expect("the river column crosses the island");
    let (mouth_x, mouth_y) = mouth_cell.to_xy(WIDTH);
    let river_id = assigner
        .river(i64::from(mouth_x), i64::from(mouth_y))
        .expect("river assignment");
    let rivers = vec![RiverRecord {
        id: river_id,
        mouth: Anchor {
            x: i64::from(mouth_x),
            y: i64::from(mouth_y),
        },
        path,
    }];

    GeographicWorld {
        params: GenerationParams {
            params_version: 0,
            seed,
            climate: None,
            terrain: None,
        },
        grid: GridSection {
            width: WIDTH,
            height: HEIGHT,
            connectivity: Connectivity::Four,
            cells,
        },
        territories: territory_records,
        regions,
        water_bodies,
        rivers,
        climate: ClimateSection::default(),
        biomes: BiomeSection::default(),
    }
}

/// The seed-zero synthetic world: the world behind the checked-in golden
/// file.
pub fn minimal_world() -> GeographicWorld {
    seeded_world(0)
}

/// Builds the dimension-parameterized skeleton for the real generation
/// pipeline: an empty grid at sea level with no water, no territories,
/// and no staged sections.
///
/// This is the entry of the real pipeline — the terrain, climate,
/// biome, and hydrology stages fill it in order. [`seeded_world`]
/// remains the self-contained E-04 fixture whose golden manifests pin
/// the synthetic pipeline.
///
/// The function is pure: identical arguments produce byte-identical
/// exports.
pub fn skeleton_world(width: u32, height: u32, seed: u64) -> GeographicWorld {
    assert!(
        width > 0 && height > 0,
        "the skeleton grid must be non-empty"
    );
    let cells = vec![
        CellRecord {
            height: HeightM(0),
            is_water: false,
            territory: NO_INDEX,
            water_body: NO_INDEX,
        };
        (width * height) as usize
    ];
    GeographicWorld {
        params: GenerationParams {
            params_version: 0,
            seed,
            climate: None,
            terrain: None,
        },
        grid: GridSection {
            width,
            height,
            connectivity: Connectivity::Four,
            cells,
        },
        territories: Vec::new(),
        regions: Vec::new(),
        water_bodies: Vec::new(),
        rivers: Vec::new(),
        climate: ClimateSection::default(),
        biomes: BiomeSection::default(),
    }
}

/// Integer centroid (floor) of the member cells: the anchor behind an
/// entity identifier.
fn centroid(members: &[CellId]) -> Anchor {
    let n = members.len() as i64;
    assert!(n > 0, "an entity must have at least one member cell");
    let (mut sum_x, mut sum_y) = (0i64, 0i64);
    for member in members {
        // The synthetic grid width is a compile-time constant of this
        // module; decode with the same width used to build the ids.
        let (x, y) = member.to_xy(WIDTH);
        sum_x += i64::from(x);
        sum_y += i64::from(y);
    }
    Anchor {
        x: sum_x / n,
        y: sum_y / n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GeographicWorld;

    /// Seed sweep shared by the invariant tests.
    const SWEEP: [u64; 6] = [0, 1, 42, 0x0000_DEAD_BEEF, 0x5EED_5EED_5EED_5EED, u64::MAX];

    #[test]
    fn construction_is_deterministic() {
        for seed in SWEEP {
            assert_eq!(seeded_world(seed), seeded_world(seed), "seed {seed}");
        }
    }

    #[test]
    fn seeds_change_the_world() {
        for seed_pair in SWEEP.windows(2) {
            assert_ne!(
                seeded_world(seed_pair[0]).to_bytes(),
                seeded_world(seed_pair[1]).to_bytes(),
                "seeds {} and {} must produce different worlds",
                seed_pair[0],
                seed_pair[1]
            );
        }
    }

    #[test]
    fn every_seed_satisfies_the_constructor_invariants() {
        for seed in SWEEP {
            let world = seeded_world(seed);
            assert_eq!(world.territories.len(), 2, "seed {seed}");
            assert!(
                world
                    .territories
                    .iter()
                    .all(|t| t.cell_count > 0 && !t.is_water && t.region.is_some())
            );
            assert_eq!(world.regions.len(), 1);
            assert_eq!(world.water_bodies.len(), 1);
            assert_eq!(world.rivers.len(), 1);
            assert!(!world.rivers[0].path.is_empty());
            // The export must validate: the constructor never relies on
            // the reader being lenient.
            let bytes = world.to_bytes();
            assert_eq!(GeographicWorld::from_bytes(&bytes).expect("valid"), world);
        }
    }

    #[test]
    fn water_cells_reference_the_ocean() {
        let world = minimal_world();
        assert!(
            world
                .grid
                .cells
                .iter()
                .filter(|cell| cell.is_water)
                .all(|cell| cell.water_body == 0)
        );
        assert!(
            world
                .grid
                .cells
                .iter()
                .filter(|cell| !cell.is_water)
                .all(|cell| cell.water_body == NO_INDEX)
        );
    }

    #[test]
    fn minimal_world_is_seed_zero() {
        assert_eq!(minimal_world(), seeded_world(0));
        assert_eq!(minimal_world().params.seed, 0);
    }
}
