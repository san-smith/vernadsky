//! Golden manifest of the derived-feature stage: content hashes of the
//! full pipeline ending with enrichment, for the fixed seed set,
//! verified on every run.
//!
//! The enrichment writes `natural_potential` of every land territory, so
//! the manifest pins the whole staged pipeline with suitability. The
//! earlier manifests must stay untouched: this stage consumes no
//! randomness and comes online last.
//!
//! # Regenerating the manifest
//!
//! ```text
//! cargo test -p vernadsky-export --test export_golden regenerate_manifest -- --ignored --nocapture
//! ```
//!
//! Run this only when a drift is intended, and commit the new values
//! together with the change that caused them.

use vernadsky_biome::generate as biomes;
use vernadsky_climate::{ClimateConfig, generate as climate};
use vernadsky_core::{GeographicWorld, RngStreams, skeleton_world};
use vernadsky_export::{enrich, facts};
use vernadsky_hydrology::generate as hydrology;
use vernadsky_terrain::TerrainConfig;
use vernadsky_territory::{generate as partition, generate_regions};

/// The fixed seed set: working values plus the boundary seeds. Shared
/// with the earlier manifests.
const MANIFEST_SEEDS: [u64; 5] = [0, 42, 0x0000_DEAD_BEEF, 0x5EED_5EED_5EED_5EED, u64::MAX];

/// Content hashes of the fully staged and enriched exports, one per
/// manifest seed, on the reference platform (Linux x86_64, pinned
/// toolchain), with the default stage configurations.
const MANIFEST_HASHES: [u64; 5] = [
    0xa44a_8b37_764e_efd7,
    0x77b9_f172_9727_0568,
    0xb82b_0cc3_47f2_d1af,
    0xc060_3605_1d7d_31cf,
    0x31a8_6ab9_5874_0adc,
];

/// Fixture grid and partition sizes of the golden runs.
const GOLDEN_WIDTH: u32 = 48;
const GOLDEN_HEIGHT: u32 = 32;
const GOLDEN_LAND_COUNT: usize = 12;
const GOLDEN_WATER_COUNT: usize = 8;

/// Builds a real-pipeline world enriched with the derived features.
fn enriched_world(seed: u64) -> GeographicWorld {
    let mut world = skeleton_world(GOLDEN_WIDTH, GOLDEN_HEIGHT, seed);
    let terrain_params = TerrainConfig::default()
        .to_params()
        .expect("the default configuration is valid");
    vernadsky_terrain::generate(&mut world, &terrain_params, &RngStreams::new(seed))
        .expect("the terrain stage succeeds");
    let climate_params = ClimateConfig::default()
        .to_params()
        .expect("the default configuration is valid");
    climate(&mut world, &climate_params, &RngStreams::new(seed))
        .expect("the climate stage succeeds");
    biomes(&mut world, &RngStreams::new(seed)).expect("the biome stage succeeds");
    hydrology(&mut world, &vernadsky_core::HydrologyParams::default())
        .expect("the hydrology stage succeeds");
    partition(&mut world, GOLDEN_LAND_COUNT, GOLDEN_WATER_COUNT).expect("the partition succeeds");
    generate_regions(&mut world).expect("the regions succeed");
    enrich(&mut world).expect("the enrichment succeeds");
    world
}

#[test]
fn manifest_hashes_match_the_enriched_exports() {
    for (seed, expected) in MANIFEST_SEEDS.iter().zip(MANIFEST_HASHES.iter()) {
        let actual = enriched_world(*seed).content_hash();
        assert_eq!(
            actual, *expected,
            "content hash drifted for seed {seed:#018x}; if the drift is intended, regenerate the manifest"
        );
    }
}

#[test]
fn enrichment_is_deterministic_and_idempotent() {
    for seed in MANIFEST_SEEDS {
        let mut world = enriched_world(seed);
        let first = world.to_bytes();
        enrich(&mut world).expect("a second enrichment succeeds");
        assert_eq!(first, world.to_bytes(), "seed {seed:#018x} must be stable");
    }
}

#[test]
fn filled_worlds_round_trip() {
    for seed in MANIFEST_SEEDS {
        let world = enriched_world(seed);
        assert_eq!(
            GeographicWorld::from_bytes(&world.to_bytes()).expect("valid export"),
            world,
            "seed {seed:#018x}"
        );
    }
}

#[test]
fn hash_detects_single_value_drift() {
    // Canary: the detector must react to the smallest possible
    // enrichment change, or the manifest would pass while the stage
    // drifts.
    let mut drifted = enriched_world(MANIFEST_SEEDS[0]);
    drifted.territories[0].natural_potential.0 += 1;
    assert_ne!(drifted.content_hash(), MANIFEST_HASHES[0]);
}

#[test]
fn potentials_stay_in_the_permille_domain() {
    for seed in MANIFEST_SEEDS {
        let world = enriched_world(seed);
        for territory in &world.territories {
            assert!(
                (0..=1000).contains(&territory.natural_potential.0),
                "potential {:?} outside 0..=1000 permille, seed {seed:#018x}",
                territory.natural_potential
            );
        }
    }
}

#[test]
fn the_facts_match_the_world() {
    let world = enriched_world(42);
    let all = facts(&world).expect("facts succeed");
    assert_eq!(all.len(), world.territories.len());

    // Coastal territories exist on a world with land and water.
    assert!(
        all.iter().filter(|fact| fact.is_coastal).count() >= 1,
        "a world with an ocean must have coastal land"
    );
    // The real terrain carries mountains, so some facts flag them.
    assert!(
        all.iter().filter(|fact| fact.is_mountainous).count() >= 1,
        "the fixture relief must carry a mountainous territory"
    );
    // Seed 42 carries rivers (verified by the hydrology stage), so some
    // territories hold river mouths.
    assert!(
        all.iter()
            .map(|fact| fact.river_mouths.len())
            .sum::<usize>()
            >= 1,
        "seed 42 has rivers, their mouths must be reported"
    );
}

#[test]
#[ignore = "prints manifest hashes; run deliberately after an intended drift"]
fn regenerate_manifest() {
    for seed in MANIFEST_SEEDS {
        let hash = enriched_world(seed).content_hash();
        println!("seed {seed:#018x} => 0x{hash:016x}");
    }
}
