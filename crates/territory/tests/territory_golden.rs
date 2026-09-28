//! Golden manifest of the territory stage: content hashes of the real
//! pipeline extended with the land partition, for the fixed seed set,
//! verified on every run.
//!
//! The partition consumes terrain and climate only, so the earlier
//! manifests (`core`, `climate`, `biome`, `hydrology`, `terrain`) must
//! stay untouched. If a test here fails, either nondeterminism slipped
//! into the stage, or the weight model, the growth, or the identifier
//! derivation drifted — both must be deliberate, reviewed changes.
//!
//! # Regenerating the manifest
//!
//! ```text
//! cargo test -p vernadsky-territory --test territory_golden regenerate_manifest -- --ignored --nocapture
//! ```
//!
//! Run this only when a drift is intended, and commit the new values
//! together with the change that caused them.

use vernadsky_biome::generate as biomes;
use vernadsky_climate::{ClimateConfig, generate as climate};
use vernadsky_core::{GeographicWorld, RngStreams, skeleton_world};
use vernadsky_hydrology::generate as hydrology;
use vernadsky_terrain::{TerrainConfig, generate as terrain};
use vernadsky_territory::{generate, generate_regions};

/// The fixed seed set: working values plus the boundary seeds. Shared
/// with the earlier manifests.
const MANIFEST_SEEDS: [u64; 5] = [0, 42, 0x0000_DEAD_BEEF, 0x5EED_5EED_5EED_5EED, u64::MAX];

/// Content hashes of the fully staged exports (partition included), one
/// per manifest seed, on the reference platform (Linux x86_64, pinned
/// toolchain), with the default stage configurations and the fixture
/// territory count.
const MANIFEST_HASHES: [u64; 5] = [
    0x5dc9_0f27_778c_fe62,
    0x4698_7703_3a14_a208,
    0x822c_8b91_da8b_7439,
    0x6465_35ef_ec13_3508,
    0x96d6_9ec1_166a_5497,
];

/// Fixture grid and partition sizes of the golden runs.
const GOLDEN_WIDTH: u32 = 48;
const GOLDEN_HEIGHT: u32 = 32;
const GOLDEN_LAND_COUNT: usize = 12;
const GOLDEN_WATER_COUNT: usize = 8;

/// Builds a real-pipeline world partitioned into `GOLDEN_COUNT`
/// territories.
fn world_with_territories(seed: u64) -> GeographicWorld {
    let mut world = skeleton_world(GOLDEN_WIDTH, GOLDEN_HEIGHT, seed);
    let terrain_params = TerrainConfig::default()
        .to_params()
        .expect("the default configuration is valid");
    terrain(&mut world, &terrain_params, &RngStreams::new(seed))
        .expect("the terrain stage succeeds");
    let climate_params = ClimateConfig::default()
        .to_params()
        .expect("the default configuration is valid");
    climate(&mut world, &climate_params, &RngStreams::new(seed))
        .expect("the climate stage succeeds");
    biomes(&mut world, &RngStreams::new(seed)).expect("the biome stage succeeds");
    hydrology(&mut world, &vernadsky_core::HydrologyParams::default())
        .expect("the hydrology stage succeeds");
    generate(&mut world, GOLDEN_LAND_COUNT, GOLDEN_WATER_COUNT).expect("the partition succeeds");
    generate_regions(&mut world).expect("the regions succeed");
    world
}

#[test]
fn manifest_hashes_match_the_partitioned_exports() {
    for (seed, expected) in MANIFEST_SEEDS.iter().zip(MANIFEST_HASHES.iter()) {
        let actual = world_with_territories(*seed).content_hash();
        assert_eq!(
            actual, *expected,
            "content hash drifted for seed {seed:#018x}; if the drift is intended, regenerate the manifest"
        );
    }
}

#[test]
fn generation_is_deterministic() {
    for seed in MANIFEST_SEEDS {
        let first = world_with_territories(seed).to_bytes();
        let second = world_with_territories(seed).to_bytes();
        assert_eq!(
            first, second,
            "seed {seed:#018x} must generate deterministically"
        );
    }
}

#[test]
fn filled_worlds_round_trip() {
    for seed in MANIFEST_SEEDS {
        let world = world_with_territories(seed);
        assert_eq!(
            world.territories.len(),
            GOLDEN_LAND_COUNT + GOLDEN_WATER_COUNT,
            "seed {seed:#018x}"
        );
        assert_eq!(
            GeographicWorld::from_bytes(&world.to_bytes()).expect("valid export"),
            world
        );
    }
}

#[test]
fn hash_detects_single_value_drift() {
    // Canary: the detector must react to the smallest possible
    // partition change, or the manifest would pass while the stage
    // drifts.
    let mut drifted = world_with_territories(MANIFEST_SEEDS[0]);
    let anchor = &mut drifted.territories[0].anchor;
    anchor.x += 1;
    assert_ne!(drifted.content_hash(), MANIFEST_HASHES[0]);
}

#[test]
#[ignore = "prints manifest hashes; run deliberately after an intended drift"]
fn regenerate_manifest() {
    for seed in MANIFEST_SEEDS {
        let hash = world_with_territories(seed).content_hash();
        println!("seed {seed:#018x} => 0x{hash:016x}");
    }
}
