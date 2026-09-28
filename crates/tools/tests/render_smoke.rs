//! Smoke tests of the debug tool: raster dimensions, layer colors,
//! graceful skip of absent sections, and export round-trips.

use vernadsky_core::{GeographicWorld, RngStreams, seeded_world};
use vernadsky_tools::render::{
    LAKE_COLOR, LAND_COLOR, Layer, NO_TERRITORY_COLOR, OCEAN_COLOR, RIVER_COLOR,
    RIVERS_WATER_COLOR, SEA_COLOR, WATER_COLOR, rasterize, zoom,
};
use vernadsky_tools::{build_fixture_world, climate_params_from_toml};

/// A world with the climate stage applied (default configuration).
fn world_with_climate(seed: u64) -> GeographicWorld {
    let mut world = seeded_world(seed);
    let params = vernadsky_climate::ClimateConfig::default()
        .to_params()
        .expect("default config is valid");
    vernadsky_climate::generate(&mut world, &params, &RngStreams::new(seed))
        .expect("climate stage succeeds");
    world
}

#[test]
fn rasters_match_the_grid_dimensions() {
    let world = world_with_climate(0);
    // The biome section stays absent until E-05 S-02 (biome stage).
    let present = [
        Layer::Height,
        Layer::Water,
        Layer::Temperature,
        Layer::Humidity,
        Layer::Precipitation,
        Layer::Territory,
    ];
    for layer in present {
        let raster = rasterize(&world, layer)
            .unwrap_or_else(|| panic!("layer {} must render on a full world", layer.name()));
        assert_eq!(raster.width, world.grid.width, "layer {}", layer.name());
        assert_eq!(raster.height, world.grid.height, "layer {}", layer.name());
        assert_eq!(raster.pixels.len(), world.grid.cells.len());
    }
    assert!(rasterize(&world, Layer::Biome).is_none());
}

#[test]
fn water_layer_distinguishes_water_and_land() {
    let world = seeded_world(0);
    let raster = rasterize(&world, Layer::Water).expect("water always renders");
    for (cell, pixel) in world.grid.cells.iter().zip(raster.pixels.iter()) {
        let expected = if cell.is_water {
            WATER_COLOR
        } else {
            LAND_COLOR
        };
        assert_eq!(pixel, &expected);
    }
}

#[test]
fn absent_sections_are_reported_as_none() {
    // A bare synthetic world carries no climate or biome sections yet.
    let world = seeded_world(0);
    assert!(rasterize(&world, Layer::Temperature).is_none());
    assert!(rasterize(&world, Layer::Humidity).is_none());
    assert!(rasterize(&world, Layer::Precipitation).is_none());
    assert!(rasterize(&world, Layer::Biome).is_none());
    assert!(rasterize(&world, Layer::Height).is_some());
    assert!(rasterize(&world, Layer::Water).is_some());
    assert!(rasterize(&world, Layer::Territory).is_some());
}

#[test]
fn biome_layer_uses_registry_colors() {
    let mut world = seeded_world(0);
    let params = vernadsky_climate::ClimateConfig::default()
        .to_params()
        .expect("default config is valid");
    vernadsky_climate::generate(&mut world, &params, &RngStreams::new(0))
        .expect("climate stage succeeds");
    vernadsky_biome::generate(&mut world, &RngStreams::new(0)).expect("biome stage succeeds");

    let raster = rasterize(&world, Layer::Biome).expect("biome section is filled");
    for pixel in &raster.pixels {
        assert!(
            vernadsky_biome::registry::REGISTRY
                .iter()
                .any(|entry| &entry.color == pixel),
            "biome pixel {pixel:?} must be a registry color"
        );
    }
}

#[test]
fn water_layer_shades_by_water_body_kind() {
    let world = build_fixture_world(0, None).expect("pipeline succeeds");
    let raster = rasterize(&world, Layer::Water).expect("water always renders");
    for (cell, pixel) in world.grid.cells.iter().zip(raster.pixels.iter()) {
        let expected = if !cell.is_water {
            LAND_COLOR
        } else if cell.water_body == vernadsky_core::NO_INDEX {
            continue;
        } else {
            match world.water_bodies[cell.water_body as usize].kind {
                vernadsky_core::WaterBodyKind::Ocean => OCEAN_COLOR,
                vernadsky_core::WaterBodyKind::Sea => SEA_COLOR,
                vernadsky_core::WaterBodyKind::Lake => LAKE_COLOR,
            }
        };
        assert_eq!(pixel, &expected);
    }
}

#[test]
fn rivers_layer_highlights_the_network() {
    // Seed 0's real terrain stays below the river threshold; seed 42
    // carries a network.
    let params = vernadsky_climate::ClimateConfig::default()
        .to_params()
        .expect("default config is valid");
    let world = build_fixture_world(42, Some(&params)).expect("pipeline succeeds");
    let raster = rasterize(&world, Layer::Rivers).expect("rivers always render");
    let river_pixels = raster
        .pixels
        .iter()
        .filter(|pixel| **pixel == RIVER_COLOR)
        .count();
    assert!(river_pixels > 0, "the real terrain must carry rivers");
}

#[test]
fn territory_layer_covers_the_partitioned_land() {
    let params = vernadsky_climate::ClimateConfig::default()
        .to_params()
        .expect("default config is valid");
    let world = build_fixture_world(0, Some(&params)).expect("pipeline succeeds");
    let raster = rasterize(&world, Layer::Territory).expect("territories are partitioned");
    for (cell, pixel) in world.grid.cells.iter().zip(raster.pixels.iter()) {
        let expected = if cell.territory == vernadsky_core::NO_INDEX {
            NO_TERRITORY_COLOR // unpartitioned surface: the water
        } else {
            continue;
        };
        assert_eq!(pixel, &expected, "water must stay the background");
    }
}

#[test]
fn zoom_preserves_content_dimensions() {
    let world = world_with_climate(0);
    let raster = rasterize(&world, Layer::Temperature).expect("climate present");
    let doubled = zoom(&raster, 2);
    assert_eq!(doubled.width, raster.width * 2);
    assert_eq!(doubled.height, raster.height * 2);
    // Nearest-neighbor: every source pixel must appear at (0, 0) of its
    // zoom block.
    assert_eq!(doubled.pixels[0], raster.pixels[0]);
}

#[test]
fn built_world_dump_round_trips() {
    let params = vernadsky_climate::ClimateConfig::default()
        .to_params()
        .expect("default config is valid");
    let world = build_fixture_world(42, Some(&params)).expect("pipeline succeeds");
    let bytes = world.to_bytes();
    assert_eq!(
        world.params.params_version, 3,
        "terrain, climate, and hydrology applied"
    );
    assert!(world.params.terrain.is_some(), "terrain parameters applied");
    let decoded = GeographicWorld::from_bytes(&bytes).expect("valid export");
    assert_eq!(decoded, world);
    assert_eq!(decoded.content_hash(), world.content_hash());
}

#[test]
fn climate_config_from_toml_changes_the_world() {
    let params = climate_params_from_toml("temperature_offset_c = 10.0\n").expect("valid config");
    let warm = build_fixture_world(0, Some(&params)).expect("pipeline succeeds");
    let plain = build_fixture_world(0, None).expect("pipeline succeeds");
    assert_ne!(warm.to_bytes(), plain.to_bytes(), "config must matter");
}
