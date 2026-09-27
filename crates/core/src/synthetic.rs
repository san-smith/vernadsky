//! Deterministic constructor of a minimal synthetic world.
//!
//! This module exists so the schema, the canonical export format, and the
//! identifier machinery can be exercised end to end before the real
//! generation stages come online. It is *not* a generation stage: the
//! terrain is a fixed analytic function, and everything derives from it
//! without random numbers.
//!
//! The output feeds the golden round-trip test in
//! `crates/core/tests/golden_round_trip.rs`; any change here changes the
//! golden file and must be reviewed deliberately.

use crate::id::{CellId, RegionId};
use crate::idgen::IdAssigner;
use crate::quant::{HeightM, NatPotential};
use crate::schema::{
    Anchor, BiomeSection, CellRecord, ClimateSection, Connectivity, GenerationParams,
    GeographicWorld, GridSection, NO_INDEX, RegionRecord, RiverRecord, TerritoryRecord,
    WaterBodyKind, WaterBodyRecord,
};

/// Grid width of the synthetic world.
const WIDTH: u32 = 48;
/// Grid height of the synthetic world.
const HEIGHT: u32 = 32;
/// Center of the synthetic island.
const CENTER_X: i64 = 24;
/// Center of the synthetic island.
const CENTER_Y: i64 = 16;
/// The column the synthetic river runs down.
const RIVER_X: u32 = 24;

/// Natural potential of the western synthetic territory, in permille.
const WEST_POTENTIAL: i16 = 500;
/// Natural potential of the eastern synthetic territory, in permille.
const EAST_POTENTIAL: i16 = 750;

/// Builds the minimal synthetic world.
///
/// The terrain is a radial island: heights come from a fixed analytic
/// function computed in `f64` and quantized at the hand-off; cells at or
/// below sea level are water. The land is split into two synthetic
/// territories (western and eastern half), grouped into one region; the
/// surrounding water forms a single ocean body; one river runs down the
/// center column.
pub fn minimal_world() -> GeographicWorld {
    let mut cells = Vec::with_capacity((WIDTH * HEIGHT) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let dx = i64::from(x) - CENTER_X;
            let dy = i64::from(y) - CENTER_Y;
            let squared_distance = (dx * dx + dy * dy) as f64;
            let height_f64 = 150.0 - 0.9 * squared_distance;
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

    // One river down the center column: start at the first land cell and
    // follow the column until the water line; the mouth is the last land
    // cell of the path.
    let mut path = Vec::new();
    for y in 0..HEIGHT {
        let cell = &cells[cell_index(RIVER_X, y)];
        if cell.is_water {
            if path.is_empty() {
                continue;
            }
            break;
        }
        path.push(CellId::from_xy(RIVER_X, y, WIDTH));
    }
    let mouth_cell = *path.last().expect("the river crosses land");
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
            seed: 0,
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
    use crate::schema::NO_INDEX;

    #[test]
    fn construction_is_deterministic() {
        assert_eq!(minimal_world(), minimal_world());
    }

    #[test]
    fn both_territories_hold_land_on_their_side() {
        let world = minimal_world();
        assert_eq!(world.territories.len(), 2);
        assert!(
            world
                .territories
                .iter()
                .all(|t| t.cell_count > 0 && !t.is_water && t.region.is_some())
        );
    }

    #[test]
    fn water_cells_reference_the_ocean() {
        let world = minimal_world();
        let water_cells = world.grid.cells.iter().filter(|cell| cell.is_water).count();
        assert!(water_cells > 0);
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
}
