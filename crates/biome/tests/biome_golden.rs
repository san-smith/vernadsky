//! Golden manifest of the biome stage: content hashes of synthetic
//! worlds with the climate and biome sections filled, for the fixed
//! seed set of the core manifest, verified on every run.
//!
//! The stage is a pure function of `(world, streams)`, so each manifest
//! entry pins one reproducible biome hand-off over the default climate
//! configuration. The core and climate manifests must stay untouched:
//! adding this stage must not shift any earlier stage's output. If a
//! test here fails, either nondeterminism slipped into the stage, or
//! the rules, the registry, or the schema drifted — both must be
//! deliberate, reviewed changes.
//!
//! # Regenerating the manifest
//!
//! ```text
//! cargo test -p vernadsky-biome --test biome_golden regenerate_manifest -- --ignored --nocapture
//! ```
//!
//! Run this only when a drift is intended, and commit the new values
//! together with the change that caused them.

use vernadsky_biome::{by_name, entry, generate};
use vernadsky_climate::{ClimateConfig, generate as climate};
use vernadsky_core::{BiomeId, GeographicWorld, RngStreams, seeded_world};

/// The fixed seed set: working values plus the boundary seeds. Shared
/// with the core and climate manifests.
const MANIFEST_SEEDS: [u64; 5] = [0, 42, 0x0000_DEAD_BEEF, 0x5EED_5EED_5EED_5EED, u64::MAX];

/// Content hashes of the climate-and-biome-filled exports, one per
/// manifest seed, on the reference platform (Linux x86_64, pinned
/// toolchain), with the default stage configuration.
const MANIFEST_HASHES: [u64; 5] = [
    0x14dd_bcdc_450e_9088,
    0xc4d2_ff93_142e_75b1,
    0x252c_84f0_d5e6_5503,
    0x7b86_93a7_75ce_3cbb,
    0x156d_985b_acdf_fbeb,
];

/// Builds a synthetic world with the climate and biome stages applied
/// (default configurations).
fn world_with_biomes(seed: u64) -> GeographicWorld {
    let mut world = seeded_world(seed);
    let params = ClimateConfig::default()
        .to_params()
        .expect("the default configuration is valid");
    climate(&mut world, &params, &RngStreams::new(seed)).expect("the climate stage succeeds");
    generate(&mut world, &RngStreams::new(seed)).expect("the biome stage succeeds");
    world
}

#[test]
fn manifest_hashes_match_the_biome_filled_exports() {
    for (seed, expected) in MANIFEST_SEEDS.iter().zip(MANIFEST_HASHES.iter()) {
        let actual = world_with_biomes(*seed).content_hash();
        assert_eq!(
            actual, *expected,
            "content hash drifted for seed {seed:#018x}; if the drift is intended, regenerate the manifest"
        );
    }
}

#[test]
fn generation_is_deterministic() {
    for seed in MANIFEST_SEEDS {
        let first = world_with_biomes(seed).to_bytes();
        let second = world_with_biomes(seed).to_bytes();
        assert_eq!(
            first, second,
            "seed {seed:#018x} must generate deterministically"
        );
    }
}

#[test]
fn filled_worlds_round_trip() {
    for seed in MANIFEST_SEEDS {
        let world = world_with_biomes(seed);
        assert!(world.biomes.biome.is_some(), "seed {seed:#018x}");
        assert_eq!(
            GeographicWorld::from_bytes(&world.to_bytes()).expect("valid export"),
            world
        );
    }
}

#[test]
fn hash_detects_single_value_drift() {
    // Canary: the detector must react to the smallest possible biome
    // change, or the manifest would pass while the stage drifts.
    let mut drifted = world_with_biomes(MANIFEST_SEEDS[0]);
    let biomes = drifted.biomes.biome.as_mut().expect("filled");
    biomes[0] = BiomeId(biomes[0].0 % 16 + 1);
    assert_ne!(drifted.content_hash(), MANIFEST_HASHES[0]);
}

#[test]
fn produced_biomes_are_registry_entries() {
    for seed in MANIFEST_SEEDS {
        let world = world_with_biomes(seed);
        let biomes = world.biomes.biome.as_ref().expect("filled");
        for id in biomes {
            let registry_entry = entry(*id).expect("produced biomes must be registered");
            assert!(
                !registry_entry.deprecated,
                "{} must not be produced",
                registry_entry.name
            );
        }
    }
}

#[test]
fn polar_waters_freeze_and_the_equator_stays_forested() {
    let world = world_with_biomes(MANIFEST_SEEDS[0]);
    let width = world.grid.width as usize;
    let height = world.grid.height as usize;
    let biomes = world.biomes.biome.as_ref().expect("filled");
    let tropical = by_name("TropicalRainforest").expect("registered");
    let frozen = by_name("FrozenOcean").expect("registered");
    let ice = by_name("Ice").expect("registered");

    // The polar rows freeze: open water becomes pack ice, and the rare
    // polar land cell (the island reaches high latitudes on some seeds)
    // is ice by the −12 °C isotherm.
    for x in 0..width {
        for y in [0, height - 1] {
            let index = y * width + x;
            let expected = if world.grid.cells[index].is_water {
                frozen
            } else {
                ice
            };
            assert_eq!(
                biomes[index], expected,
                "polar cell ({x}, {y}) must be frozen"
            );
        }
    }
    // The wet equatorial island cells stay rainforest: the equator band
    // is warm, and the moisture march saturates the windward side.
    let equator_rows = height / 2 - 2..height / 2 + 2;
    let rainforest_cells = equator_rows
        .flat_map(|y| (0..width).map(move |x| y * width + x))
        .filter(|index| !world.grid.cells[*index].is_water)
        .filter(|index| biomes[*index] == tropical)
        .count();
    assert!(
        rainforest_cells > 0,
        "the equatorial island must carry rainforest cells"
    );
}

#[test]
#[ignore = "prints manifest hashes; run deliberately after an intended drift"]
fn regenerate_manifest() {
    for seed in MANIFEST_SEEDS {
        let hash = world_with_biomes(seed).content_hash();
        println!("seed {seed:#018x} => 0x{hash:016x}");
    }
}
