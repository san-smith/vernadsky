//! Golden manifests of the terrain stage: content hashes of the real
//! pipeline (skeleton → terrain → climate → biomes → hydrology) for the
//! fixed seed set, in both generation profiles — with erosion and
//! without — verified on every run.
//!
//! This manifest is the executable form of the E-05 exit criterion: the
//! full physical world (relief, water, climate, biomes, rivers) is
//! stable and hashable on real terrain. The synthetic manifests of the
//! earlier stages (`core`, `climate`, `biome`, `hydrology`) pin their
//! own fixture pipeline and must stay untouched: the terrain stage
//! consumes no stream that any of them uses, so coming online shifts
//! nothing there.
//!
//! # Regenerating the manifests
//!
//! ```text
//! cargo test -p vernadsky-terrain --test terrain_golden regenerate_manifests -- --ignored --nocapture
//! ```
//!
//! Run this only when a drift is intended, and commit the new values
//! together with the change that caused them.

use vernadsky_biome::generate as biomes;
use vernadsky_climate::{ClimateConfig, generate as climate};
use vernadsky_core::{GeographicWorld, RngStreams, skeleton_world};
use vernadsky_hydrology::generate as hydrology;
use vernadsky_terrain::{TerrainConfig, generate as terrain};

/// The fixed seed set: working values plus the boundary seeds. Shared
/// with the earlier manifests.
const MANIFEST_SEEDS: [u64; 5] = [0, 42, 0x0000_DEAD_BEEF, 0x5EED_5EED_5EED_5EED, u64::MAX];

/// Content hashes of the eroded-profile pipeline exports, one per
/// manifest seed, on the reference platform (Linux x86_64, pinned
/// toolchain), with the default stage configurations.
const MANIFEST_HASHES_ERODED: [u64; 5] = [
    0x97a0_f86f_f70a_22da,
    0xeed2_94f9_a88a_2a1b,
    0xb80a_f208_4e44_3dae,
    0x45dd_fb2e_6593_2bc5,
    0x1aad_40b6_7689_1111,
];

/// Content hashes of the no-erosion profile: the same pipeline with
/// `erosion_enabled = false`.
const MANIFEST_HASHES_PLAIN: [u64; 5] = [
    0x1d30_17df_c536_fd7f,
    0xc53f_2030_2eb6_b996,
    0x8764_0528_8313_dd7e,
    0xbca4_6174_0048_232f,
    0xa886_6db0_8ccc_b239,
];

/// Fixture grid of the golden runs (the fixture world dimensions).
const GOLDEN_WIDTH: u32 = 48;
const GOLDEN_HEIGHT: u32 = 32;

/// Builds a real-pipeline world: skeleton grid, terrain with the given
/// erosion setting, then climate, biomes, and hydrology.
fn real_pipeline_world(seed: u64, erosion: bool) -> GeographicWorld {
    let mut world = skeleton_world(GOLDEN_WIDTH, GOLDEN_HEIGHT, seed);
    let mut terrain_config = TerrainConfig::default();
    terrain_config.erosion_enabled = erosion;
    let terrain_params = terrain_config.to_params().expect("the config is valid");
    terrain(&mut world, &terrain_params, &RngStreams::new(seed)).expect("the terrain stage");
    let climate_params = ClimateConfig::default()
        .to_params()
        .expect("the default configuration is valid");
    climate(&mut world, &climate_params, &RngStreams::new(seed))
        .expect("the climate stage succeeds");
    biomes(&mut world, &RngStreams::new(seed)).expect("the biome stage succeeds");
    hydrology(&mut world, &vernadsky_core::HydrologyParams::default())
        .expect("the hydrology stage succeeds");
    world
}

#[test]
fn manifest_hashes_match_the_eroded_pipeline() {
    for (seed, expected) in MANIFEST_SEEDS.iter().zip(MANIFEST_HASHES_ERODED.iter()) {
        let actual = real_pipeline_world(*seed, true).content_hash();
        assert_eq!(
            actual, *expected,
            "content hash drifted for seed {seed:#018x}; if the drift is intended, regenerate the manifests"
        );
    }
}

#[test]
fn manifest_hashes_match_the_plain_pipeline() {
    for (seed, expected) in MANIFEST_SEEDS.iter().zip(MANIFEST_HASHES_PLAIN.iter()) {
        let actual = real_pipeline_world(*seed, false).content_hash();
        assert_eq!(
            actual, *expected,
            "content hash drifted for seed {seed:#018x}; if the drift is intended, regenerate the manifests"
        );
    }
}

#[test]
fn the_profiles_genuinely_differ() {
    for seed in MANIFEST_SEEDS {
        let eroded = real_pipeline_world(seed, true).to_bytes();
        let plain = real_pipeline_world(seed, false).to_bytes();
        assert_ne!(
            eroded, plain,
            "erosion must change the world, seed {seed:#018x}"
        );
    }
}

#[test]
fn generation_is_deterministic() {
    for seed in MANIFEST_SEEDS {
        for erosion in [true, false] {
            let first = real_pipeline_world(seed, erosion).to_bytes();
            let second = real_pipeline_world(seed, erosion).to_bytes();
            assert_eq!(first, second, "seed {seed:#018x} must be deterministic");
        }
    }
}

#[test]
fn filled_worlds_round_trip() {
    for seed in MANIFEST_SEEDS {
        let world = real_pipeline_world(seed, true);
        assert_eq!(world.params.params_version, 4, "seed {seed:#018x}");
        assert!(world.params.terrain.is_some());
        assert!(world.params.climate.is_some());
        assert!(world.params.hydrology.is_some());
        assert_eq!(
            GeographicWorld::from_bytes(&world.to_bytes()).expect("valid export"),
            world
        );
    }
}

#[test]
fn hash_detects_single_value_drift() {
    // Canary: the detector must react to the smallest possible terrain
    // change, or the manifest would pass while the stage drifts.
    let mut drifted = real_pipeline_world(MANIFEST_SEEDS[0], true);
    let cell = &mut drifted.grid.cells[0];
    cell.height = vernadsky_core::quant::HeightM(cell.height.0 + 7);
    assert_ne!(drifted.content_hash(), MANIFEST_HASHES_ERODED[0]);
}

#[test]
fn relief_stays_in_the_physical_scale_and_carries_land() {
    for seed in MANIFEST_SEEDS {
        let world = real_pipeline_world(seed, true);
        let mut land = 0usize;
        for cell in &world.grid.cells {
            let meters = cell.height.to_value();
            assert!(
                (-6000.0..=6000.0).contains(&meters),
                "height {meters} m outside the physical scale, seed {seed:#018x}"
            );
            if !cell.is_water {
                land += 1;
                assert!(meters > 0.0, "land cells must be above sea level");
            }
        }
        let ratio = land as f64 / world.grid.cells.len() as f64;
        assert!(
            (0.2..=0.4).contains(&ratio),
            "land ratio {ratio:.2} drifted from the targeted belt, seed {seed:#018x}"
        );
        // The downstream stages filled their sections on the real
        // relief.
        assert!(world.climate.temperature.is_some());
        assert!(world.biomes.biome.is_some());
        assert!(!world.water_bodies.is_empty());
    }
}

#[test]
#[ignore = "prints manifest hashes; run deliberately after an intended drift"]
fn regenerate_manifests() {
    for seed in MANIFEST_SEEDS {
        println!(
            "eroded seed {seed:#018x} => 0x{:016x}",
            real_pipeline_world(seed, true).content_hash()
        );
        println!(
            "plain  seed {seed:#018x} => 0x{:016x}",
            real_pipeline_world(seed, false).content_hash()
        );
    }
}
