//! Golden seed manifest: content hashes of full exports for a fixed seed
//! set, verified on every run.
//!
//! The synthetic world is a pure function of its seed, so each manifest
//! entry pins one reproducible world. If a test here fails, either
//! nondeterminism slipped into the generator, or the schema/format/
//! constructor drifted — both must be deliberate, reviewed changes.
//!
//! # Regenerating the manifest and the golden file
//!
//! ```text
//! cargo test --test golden_seeds regenerate_manifest -- --ignored --nocapture
//! cargo test --test golden_round_trip regenerate_golden -- --ignored
//! ```
//!
//! Run these only when a drift is intended, and commit the new values
//! together with the change that caused them.

use vernadsky_core::{
    GeographicWorld, ID_STRATEGY, LATTICE_REGISTRY_VERSION, RNG_STRATEGY, SCHEMA_VERSION,
    seeded_world,
};

/// The fixed seed set: working values plus the boundary seeds.
const MANIFEST_SEEDS: [u64; 5] = [0, 42, 0x0000_DEAD_BEEF, 0x5EED_5EED_5EED_5EED, u64::MAX];

/// Content hashes of the exports, one per manifest seed, on the reference
/// platform (Linux x86_64, pinned toolchain). Cross-platform comparisons
/// with tolerances are the E-13 run's job, not this manifest's.
const MANIFEST_HASHES: [u64; 5] = [
    0x3cdf_c338_07a1_0c27,
    0xcc9c_562d_536d_9044,
    0x35a0_e497_54a3_999f,
    0x5570_976b_443f_5a99,
    0x28fc_091e_440b_c05e,
];

#[test]
fn manifest_pins_the_contract_versions() {
    assert_eq!(SCHEMA_VERSION, 0);
    assert_eq!(ID_STRATEGY, "content-hash-v1");
    assert_eq!(LATTICE_REGISTRY_VERSION, "lr-v1");
    assert_eq!(RNG_STRATEGY, "rng-streams-v1");
}

#[test]
fn manifest_hashes_match_the_exports() {
    for (seed, expected) in MANIFEST_SEEDS.iter().zip(MANIFEST_HASHES.iter()) {
        let world = seeded_world(*seed);
        let actual = world.content_hash();
        assert_eq!(
            actual, *expected,
            "content hash drifted for seed {seed:#018x}; if the drift is intended, regenerate the manifest"
        );
    }
}

#[test]
fn per_seed_construction_is_deterministic() {
    for seed in MANIFEST_SEEDS {
        assert_eq!(
            seeded_world(seed).to_bytes(),
            seeded_world(seed).to_bytes(),
            "seed {seed:#018x} must build deterministically"
        );
    }
}

#[test]
fn content_hash_detects_single_cell_drift() {
    // Canary: the detector must react to the smallest possible content
    // change, or the manifest would pass while the world drifts.
    let mut drifted = seeded_world(MANIFEST_SEEDS[0]);
    let cell = &mut drifted.grid.cells[0];
    cell.height = vernadsky_core::quant::HeightM(cell.height.0 + 7);
    assert_ne!(drifted.content_hash(), MANIFEST_HASHES[0]);
}

#[test]
fn every_manifest_seed_produces_a_valid_world() {
    for seed in MANIFEST_SEEDS {
        let world = seeded_world(seed);
        assert_eq!(world.territories.len(), 2, "seed {seed}");
        assert_eq!(world.rivers.len(), 1);
        assert!(!world.rivers[0].path.is_empty());
        assert_eq!(
            GeographicWorld::from_bytes(&world.to_bytes()).expect("valid export"),
            world
        );
    }
}

#[test]
#[ignore = "prints manifest hashes; run deliberately after an intended drift"]
fn regenerate_manifest() {
    for seed in MANIFEST_SEEDS {
        let hash = seeded_world(seed).content_hash();
        println!("seed {seed:#018x} => 0x{hash:016x}");
    }
}
