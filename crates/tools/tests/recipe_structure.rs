//! Structure of the recipe surface at release resolutions (scale-review
//! §4: stage changes are inspected at 512² and above, fixture grids do
//! not catch scale dependence).
//!
//! For every combination of the land share, the base feature count, and
//! the island density, the full pipeline runs at 512² and 1024² with
//! 900 land territories and water territories of the mean land
//! territory area, over three seeds. The test checks that the land
//! share meets its target, that the water territories keep their
//! target area, and that the land structure — the largest land
//! component's share of the land territories — does not depend on the
//! resolution. It prints the table the recipes are calibrated against.
//!
//! The structure criterion is the mean drift over the seeds, not the
//! drift of every seed: a world near the percolation threshold — many
//! mid-sized masses one isthmus away from merging — swings its largest
//! component on a single narrow strait, and a strait can open at one
//! resolution and close at the other (single-seed drifts up to ~16
//! points were measured at strong island layers, up to ~8 at the
//! default one). That sensitivity predates the recipe surface; the
//! mean shows whether the structure itself depends on the resolution.
//!
//! Release-only and ignored by default (about a minute):
//!
//! ```text
//! cargo test --release -p vernadsky-tools --test recipe_structure -- --ignored --nocapture
//! ```

use vernadsky_terrain::{LAND_RATIO_TOLERANCE, TerrainConfig};
use vernadsky_tools::{WaterSplit, build_world_with, land_components};

const LAND_TERRITORIES: usize = 900;
const SEEDS: [u64; 3] = [1, 42, 20_261_012];
/// The largest allowed resolution drift of the largest component's
/// share of the land territories, in percentage points, averaged over
/// the seeds.
const STRUCTURE_DRIFT_POINTS: f64 = 10.0;
/// The allowed miss of the mean water territory area against its
/// target.
const WATER_AREA_TOLERANCE: f64 = 0.15;

struct Row {
    land_share: f64,
    components: usize,
    largest_share: f64,
    water_area_miss: f64,
}

fn measure(
    seed: u64,
    side: u32,
    land_ratio: f64,
    features_across: f64,
    island_density: f64,
) -> Row {
    let terrain = TerrainConfig {
        land_ratio,
        features_across,
        island_density,
        ..TerrainConfig::default()
    }
    .to_params()
    .expect("valid recipe surface");
    let climate = vernadsky_climate::ClimateConfig::default()
        .to_params()
        .expect("default climate");
    let cells = (side as usize) * (side as usize);
    // The water target follows the land share the recipe asks for, the
    // way a consumer derives it before generation.
    let target_area = ((cells as f64 * land_ratio) / LAND_TERRITORIES as f64).round() as u32;
    let world = build_world_with(
        seed,
        side,
        side,
        LAND_TERRITORIES,
        WaterSplit::CellsPerTerritory(target_area.max(1)),
        Some(&terrain),
        Some(&climate),
        None,
    )
    .expect("pipeline");
    let land_cells = world
        .grid
        .cells
        .iter()
        .filter(|cell| !cell.is_water)
        .count();
    let water: Vec<u32> = world
        .territories
        .iter()
        .filter(|t| t.is_water)
        .map(|t| t.cell_count)
        .collect();
    let mean_water = water.iter().map(|&c| f64::from(c)).sum::<f64>() / water.len().max(1) as f64;
    let components = land_components(&world);
    Row {
        land_share: land_cells as f64 / cells as f64,
        components: components.len(),
        largest_share: f64::from(components[0]) / LAND_TERRITORIES as f64,
        water_area_miss: (mean_water / f64::from(target_area.max(1)) - 1.0).abs(),
    }
}

#[test]
#[ignore = "release-resolution structure test; run with --release -- --ignored"]
fn the_recipe_surface_is_resolution_independent() {
    let mut failures = Vec::new();
    println!(
        "land  feat  isl  seed      | 512²: land% comps largest% water-miss% | 1024²: land% comps largest% water-miss%"
    );
    for land_ratio in [0.1, 0.3, 0.7] {
        for features_across in [0.7, 1.5, 4.0] {
            for island_density in [0.0, 0.5, 2.0] {
                let label =
                    format!("land {land_ratio} feat {features_across} isl {island_density}");
                let mut drifts = Vec::new();
                for seed in SEEDS {
                    let small = measure(seed, 512, land_ratio, features_across, island_density);
                    let large = measure(seed, 1024, land_ratio, features_across, island_density);
                    println!(
                        "{land_ratio:.2}  {features_across:.1}   {island_density:.1}  {seed:<9} | {:5.1} {:5} {:8.1} {:6.1}       | {:5.1} {:5} {:8.1} {:6.1}",
                        100.0 * small.land_share,
                        small.components,
                        100.0 * small.largest_share,
                        100.0 * small.water_area_miss,
                        100.0 * large.land_share,
                        large.components,
                        100.0 * large.largest_share,
                        100.0 * large.water_area_miss,
                    );
                    for (side, row) in [(512, &small), (1024, &large)] {
                        if (row.land_share - land_ratio).abs() > LAND_RATIO_TOLERANCE {
                            failures.push(format!(
                                "{label} seed {seed} at {side}²: land share {:.4}",
                                row.land_share
                            ));
                        }
                        if row.water_area_miss > WATER_AREA_TOLERANCE {
                            failures.push(format!(
                                "{label} seed {seed} at {side}²: water area miss {:.3}",
                                row.water_area_miss
                            ));
                        }
                    }
                    drifts.push(100.0 * (small.largest_share - large.largest_share).abs());
                }
                let mean = drifts.iter().sum::<f64>() / drifts.len() as f64;
                let max = drifts.iter().cloned().fold(0.0, f64::max);
                println!("    largest-component drift: mean {mean:.1}, max {max:.1} points");
                if mean > STRUCTURE_DRIFT_POINTS {
                    failures.push(format!(
                        "{label}: largest component drifts {mean:.1} points on average"
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "structure failures:\n{}",
        failures.join("\n")
    );
}
