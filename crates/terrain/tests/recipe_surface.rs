//! The recipe surface of the terrain stage: the targeted land share, the
//! island density, and the land hypsometry shape the world at
//! fixture-scale grids. The
//! release-resolution structure test lives in `vernadsky-tools`
//! (`tests/recipe_structure.rs`).

use vernadsky_core::{GeographicWorld, RngStreams, skeleton_world};
use vernadsky_terrain::{LAND_RATIO_TOLERANCE, TerrainConfig, generate};

fn world(seed: u64, width: u32, height: u32, config: &TerrainConfig) -> GeographicWorld {
    let mut world = skeleton_world(width, height, seed);
    let params = config.to_params().expect("valid config");
    generate(&mut world, &params, &RngStreams::new(seed)).expect("terrain stage");
    world
}

fn land_share(world: &GeographicWorld) -> f64 {
    let land = world
        .grid
        .cells
        .iter()
        .filter(|cell| !cell.is_water)
        .count();
    land as f64 / world.grid.cells.len() as f64
}

/// Connected land masses at cell level (4-neighbors, x seamless, poles
/// clamped).
fn land_masses(world: &GeographicWorld) -> usize {
    let (width, height) = (world.grid.width as usize, world.grid.height as usize);
    let cells = &world.grid.cells;
    let mut seen = vec![false; cells.len()];
    let mut masses = 0;
    for start in 0..cells.len() {
        if cells[start].is_water || seen[start] {
            continue;
        }
        masses += 1;
        seen[start] = true;
        let mut stack = vec![start];
        while let Some(index) = stack.pop() {
            let (x, y) = (index % width, index / width);
            let mut neighbors = vec![
                y * width + (x + 1) % width,
                y * width + (x + width - 1) % width,
            ];
            if y > 0 {
                neighbors.push(index - width);
            }
            if y + 1 < height {
                neighbors.push(index + width);
            }
            for next in neighbors {
                if !cells[next].is_water && !seen[next] {
                    seen[next] = true;
                    stack.push(next);
                }
            }
        }
    }
    masses
}

#[test]
fn the_land_share_follows_the_target_across_its_domain() {
    for seed in [1, 42, 0x5EED] {
        for target in [0.05, 0.1, 0.3, 0.7, 0.9, 0.95] {
            let config = TerrainConfig {
                land_ratio: target,
                ..TerrainConfig::default()
            };
            let share = land_share(&world(seed, 128, 64, &config));
            assert!(
                (share - target).abs() <= LAND_RATIO_TOLERANCE,
                "seed {seed}, target {target}: land share {share:.4}"
            );
        }
    }
}

#[test]
fn the_island_density_raises_the_island_layer() {
    // A low land share leaves wide oceans for the island lift to act on;
    // the strong layer (2.0) is what an archipelago recipe reaches for.
    for seed in [1, 42, 0x5EED] {
        let masses = |density: f64| {
            land_masses(&world(
                seed,
                256,
                256,
                &TerrainConfig {
                    land_ratio: 0.3,
                    island_density: density,
                    ..TerrainConfig::default()
                },
            ))
        };
        let (none, strong) = (masses(0.0), masses(2.0));
        assert!(
            strong > none,
            "seed {seed}: island density 2.0 gives {strong} land masses, 0.0 gives {none}"
        );
    }
}

#[test]
fn the_export_carries_the_recipe_surface() {
    let config = TerrainConfig {
        land_ratio: 0.92,
        island_density: 0.0,
        features_across: 0.7,
        ..TerrainConfig::default()
    };
    let world = world(7, 48, 32, &config);
    assert_eq!(world.params.params_version, vernadsky_core::PARAMS_VERSION);
    let decoded = GeographicWorld::from_bytes(&world.to_bytes()).expect("round trip");
    let terrain = decoded.params.terrain.expect("terrain block");
    assert_eq!(terrain.land_ratio.0, 92);
    assert_eq!(terrain.island_density.0, 0);
    assert_eq!(terrain.features_across.0, 70);
    assert_eq!(
        terrain.land_hypsometry.0, 100,
        "the linear curve by default"
    );
}

#[test]
fn the_land_hypsometry_lowers_the_land_and_keeps_the_coast() {
    for seed in [1, 42, 0x5EED] {
        let world_with = |hypsometry: f64| {
            world(
                seed,
                256,
                128,
                &TerrainConfig {
                    land_hypsometry: hypsometry,
                    ..TerrainConfig::default()
                },
            )
        };
        let (linear, curved) = (world_with(1.0), world_with(3.0));
        let water = |world: &GeographicWorld| -> Vec<bool> {
            world.grid.cells.iter().map(|cell| cell.is_water).collect()
        };
        assert_eq!(
            water(&linear),
            water(&curved),
            "seed {seed}: the coast stays"
        );
        let highland = |world: &GeographicWorld| {
            let land: Vec<_> = world.grid.cells.iter().filter(|c| !c.is_water).collect();
            land.iter().filter(|c| c.height.0 > 1500).count() as f64 / land.len() as f64
        };
        let (before, after) = (highland(&linear), highland(&curved));
        assert!(
            after < before / 2.0,
            "seed {seed}: land above 1500 m {before:.2} -> {after:.2}"
        );
        let peak = |world: &GeographicWorld| world.grid.cells.iter().map(|c| c.height.0).max();
        assert!(peak(&curved) <= peak(&linear), "seed {seed}: no peak rises");
    }
}

/// Land–water cell edges per land cell: the coastline's length against
/// the land it encloses.
fn coastline_per_land(world: &GeographicWorld) -> f64 {
    let (width, height) = (world.grid.width as usize, world.grid.height as usize);
    let cells = &world.grid.cells;
    let mut edges = 0usize;
    for y in 0..height {
        for x in 0..width {
            let here = cells[y * width + x].is_water;
            if cells[y * width + (x + 1) % width].is_water != here {
                edges += 1;
            }
            if y + 1 < height && cells[(y + 1) * width + x].is_water != here {
                edges += 1;
            }
        }
    }
    let land = cells.iter().filter(|cell| !cell.is_water).count();
    edges as f64 / land as f64
}

#[test]
fn the_warp_frays_the_coast_at_the_same_land_share() {
    for seed in [1, 42, 0x5EED] {
        let world_with = |warp: f64| {
            world(
                seed,
                256,
                256,
                &TerrainConfig {
                    warp_strength: warp,
                    ..TerrainConfig::default()
                },
            )
        };
        let (plain, warped) = (world_with(0.0), world_with(0.15));
        assert!((land_share(&plain) - land_share(&warped)).abs() <= 2.0 * LAND_RATIO_TOLERANCE);
        let (before, after) = (coastline_per_land(&plain), coastline_per_land(&warped));
        assert!(
            after > before * 1.15,
            "seed {seed}: coastline per land cell {before:.3} -> {after:.3}"
        );
    }
}

#[test]
fn the_ridges_raise_mountain_chains() {
    for seed in [1, 42, 0x5EED] {
        let world_with = |ridges: f64| {
            world(
                seed,
                256,
                256,
                &TerrainConfig {
                    ridge_strength: ridges,
                    ..TerrainConfig::default()
                },
            )
        };
        let mountains = |world: &GeographicWorld| {
            let land: Vec<_> = world.grid.cells.iter().filter(|c| !c.is_water).collect();
            land.iter().filter(|c| c.height.0 > 2500).count() as f64 / land.len() as f64
        };
        let (flat, ranged) = (mountains(&world_with(0.0)), mountains(&world_with(0.5)));
        assert!(
            ranged > flat,
            "seed {seed}: land above 2500 m {flat:.3} -> {ranged:.3}"
        );
    }
}

#[test]
fn no_warp_and_no_ridges_keep_the_relief() {
    let plain = world(9, 96, 48, &TerrainConfig::default());
    let zeroed = world(
        9,
        96,
        48,
        &TerrainConfig {
            warp_strength: 0.0,
            ridge_strength: 0.0,
            warp_features: 9.0,
            ridge_features: 2.0,
            ..TerrainConfig::default()
        },
    );
    let heights = |world: &GeographicWorld| -> Vec<i32> {
        world.grid.cells.iter().map(|cell| cell.height.0).collect()
    };
    assert_eq!(
        heights(&plain),
        heights(&zeroed),
        "the features idle without strength"
    );
}
