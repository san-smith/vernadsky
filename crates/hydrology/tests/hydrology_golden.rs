//! Golden manifest of the hydrology stage: content hashes of synthetic
//! worlds with the climate, biome, and hydrology sections filled, for
//! the fixed seed set of the core manifest, verified on every run.
//!
//! The earlier manifests (`core`, `climate`, `biome`) must stay
//! untouched: this stage consumes no randomness, so coming online must
//! not shift any earlier stage's output. If a test here fails, either
//! nondeterminism slipped into the stage, or the classification, the
//! flow model, or the identifier derivation drifted — both must be
//! deliberate, reviewed changes.
//!
//! # Regenerating the manifest
//!
//! ```text
//! cargo test -p vernadsky-hydrology --test hydrology_golden regenerate_manifest -- --ignored --nocapture
//! ```
//!
//! Run this only when a drift is intended, and commit the new values
//! together with the change that caused them.

use vernadsky_biome::generate as biomes;
use vernadsky_climate::{ClimateConfig, generate as climate};
use vernadsky_core::schema::WaterBodyKind;
use vernadsky_core::{GeographicWorld, RngStreams, seeded_world};
use vernadsky_hydrology::generate;

/// The fixed seed set: working values plus the boundary seeds. Shared
/// with the earlier manifests.
const MANIFEST_SEEDS: [u64; 5] = [0, 42, 0x0000_DEAD_BEEF, 0x5EED_5EED_5EED_5EED, u64::MAX];

/// Content hashes of the fully staged exports, one per manifest seed, on
/// the reference platform (Linux x86_64, pinned toolchain), with the
/// default stage configurations.
const MANIFEST_HASHES: [u64; 5] = [
    0x37ca_b152_1295_1ea5,
    0xa43e_25f6_3fb2_5146,
    0xfb2c_cb35_9128_b9cd,
    0x56c3_c409_7f0c_6a0b,
    0x47c8_6bda_0c8e_c206,
];

/// Builds a synthetic world through the full current pipeline.
fn world_with_hydrology(seed: u64) -> GeographicWorld {
    let mut world = seeded_world(seed);
    let params = ClimateConfig::default()
        .to_params()
        .expect("the default configuration is valid");
    climate(&mut world, &params, &RngStreams::new(seed)).expect("the climate stage succeeds");
    biomes(&mut world, &RngStreams::new(seed)).expect("the biome stage succeeds");
    generate(&mut world).expect("the hydrology stage succeeds");
    world
}

#[test]
fn manifest_hashes_match_the_fully_staged_exports() {
    for (seed, expected) in MANIFEST_SEEDS.iter().zip(MANIFEST_HASHES.iter()) {
        let actual = world_with_hydrology(*seed).content_hash();
        assert_eq!(
            actual, *expected,
            "content hash drifted for seed {seed:#018x}; if the drift is intended, regenerate the manifest"
        );
    }
}

#[test]
fn generation_is_deterministic() {
    for seed in MANIFEST_SEEDS {
        let first = world_with_hydrology(seed).to_bytes();
        let second = world_with_hydrology(seed).to_bytes();
        assert_eq!(
            first, second,
            "seed {seed:#018x} must generate deterministically"
        );
    }
}

#[test]
fn filled_worlds_round_trip() {
    for seed in MANIFEST_SEEDS {
        let world = world_with_hydrology(seed);
        assert!(!world.water_bodies.is_empty(), "seed {seed:#018x}");
        assert_eq!(
            GeographicWorld::from_bytes(&world.to_bytes()).expect("valid export"),
            world
        );
    }
}

#[test]
fn hash_detects_single_value_drift() {
    // Canary: the detector must react to the smallest possible
    // hydrology change, or the manifest would pass while the stage
    // drifts.
    let mut drifted = world_with_hydrology(MANIFEST_SEEDS[0]);
    drifted.water_bodies[0].anchor.x += 1;
    assert_ne!(drifted.content_hash(), MANIFEST_HASHES[0]);
}

#[test]
fn the_synthetic_world_carries_one_ocean() {
    // The fixture water ring is fully connected and reaches both polar
    // rows: exactly one component, classified as the ocean.
    for seed in MANIFEST_SEEDS {
        let world = world_with_hydrology(seed);
        assert_eq!(world.water_bodies.len(), 1, "seed {seed:#018x}");
        assert_eq!(world.water_bodies[0].kind, WaterBodyKind::Ocean);
        // Every water cell references it; no land cell does.
        let ocean_index = 0u32;
        for cell in &world.grid.cells {
            if cell.is_water {
                assert_eq!(cell.water_body, ocean_index, "seed {seed:#018x}");
            } else {
                assert_eq!(cell.water_body, vernadsky_core::NO_INDEX);
            }
        }
    }
}

#[test]
fn river_paths_end_at_the_water() {
    let mut total_rivers = 0;
    for seed in MANIFEST_SEEDS {
        let world = world_with_hydrology(seed);
        let width = world.grid.width;
        for river in &world.rivers {
            total_rivers += 1;
            assert!(!river.path.is_empty(), "seed {seed:#018x}");
            for cell in &river.path {
                assert!(
                    !world.grid.cells[cell.0 as usize].is_water,
                    "river paths stay on land, seed {seed:#018x}"
                );
            }
            let (mx, my) = (river.mouth.x, river.mouth.y);
            // The mouth must be one of the path cells.
            let mouth_index = (my as usize) * width as usize + mx as usize;
            let mouth_in_path = river.path.iter().any(|cell| cell.0 as usize == mouth_index);
            assert!(mouth_in_path, "the mouth belongs to the path");
        }
    }
    assert!(total_rivers > 0, "the golden set must carry rivers");
}

#[test]
#[ignore = "prints manifest hashes; run deliberately after an intended drift"]
fn regenerate_manifest() {
    for seed in MANIFEST_SEEDS {
        let hash = world_with_hydrology(seed).content_hash();
        println!("seed {seed:#018x} => 0x{hash:016x}");
    }
}
