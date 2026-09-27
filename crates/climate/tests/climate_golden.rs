//! Golden manifest of the climate stage: content hashes of synthetic
//! worlds whose climate section is filled, for the fixed seed set of the
//! core manifest, verified on every run.
//!
//! The stage is a pure function of `(world grid, params, streams)`, so
//! each manifest entry pins one reproducible climate hand-off. The
//! default configuration is part of the pinned input. If a test here
//! fails, either nondeterminism slipped into the stage, or the model,
//! the schema, or the stage contract drifted — both must be deliberate,
//! reviewed changes. The core manifest (`vernadsky-core` tests) pins the
//! climate-free worlds and must stay untouched: adding this stage must
//! not shift any existing stream or export.
//!
//! # Regenerating the manifest
//!
//! ```text
//! cargo test -p vernadsky-climate --test climate_golden regenerate_manifest -- --ignored --nocapture
//! ```
//!
//! Run this only when a drift is intended, and commit the new values
//! together with the change that caused them.

use vernadsky_climate::{ClimateConfig, SMOOTHING_RADIUS, TEMP_MAX_C, TEMP_MIN_C, generate};
use vernadsky_core::quant::TempDeciC;
use vernadsky_core::{GeographicWorld, RngStreams, seeded_world};

/// The fixed seed set: working values plus the boundary seeds. Shared
/// with the core golden manifest.
const MANIFEST_SEEDS: [u64; 5] = [0, 42, 0x0000_DEAD_BEEF, 0x5EED_5EED_5EED_5EED, u64::MAX];

/// Content hashes of the climate-filled exports, one per manifest seed,
/// on the reference platform (Linux x86_64, pinned toolchain), with the
/// default stage configuration.
const MANIFEST_HASHES: [u64; 5] = [
    0x2976_6b38_1fbf_e0d7,
    0x2d8e_7cfe_16b3_ba08,
    0x638b_51c9_d366_0dc8,
    0x6b51_458a_5a4c_f50f,
    0x10fd_ffb7_b2e2_1358,
];

/// Builds a synthetic world with the climate stage applied (default
/// configuration).
fn world_with_climate(seed: u64) -> GeographicWorld {
    let mut world = seeded_world(seed);
    let params = ClimateConfig::default()
        .to_params()
        .expect("the default configuration is valid");
    generate(&mut world, &params, &RngStreams::new(seed)).expect("the stage succeeds");
    world
}

#[test]
fn manifest_hashes_match_the_climate_filled_exports() {
    for (seed, expected) in MANIFEST_SEEDS.iter().zip(MANIFEST_HASHES.iter()) {
        let actual = world_with_climate(*seed).content_hash();
        assert_eq!(
            actual, *expected,
            "content hash drifted for seed {seed:#018x}; if the drift is intended, regenerate the manifest"
        );
    }
}

#[test]
fn generation_is_deterministic() {
    for seed in MANIFEST_SEEDS {
        let first = world_with_climate(seed).to_bytes();
        let second = world_with_climate(seed).to_bytes();
        assert_eq!(
            first, second,
            "seed {seed:#018x} must generate deterministically"
        );
    }
}

#[test]
fn filled_worlds_round_trip() {
    for seed in MANIFEST_SEEDS {
        let world = world_with_climate(seed);
        assert_eq!(world.params.params_version, 1, "seed {seed:#018x}");
        assert!(world.params.climate.is_some());
        assert!(world.climate.temperature.is_some());
        assert!(world.climate.humidity.is_some());
        assert!(world.climate.precipitation.is_some());
        assert_eq!(
            GeographicWorld::from_bytes(&world.to_bytes()).expect("valid export"),
            world
        );
    }
}

#[test]
fn hash_detects_single_value_drift() {
    // Canary: the detector must react to the smallest possible climate
    // change, or the manifest would pass while the stage drifts.
    let mut drifted = world_with_climate(MANIFEST_SEEDS[0]);
    let temperature = drifted.climate.temperature.as_mut().expect("filled");
    temperature[0] = TempDeciC(temperature[0].0 + 1);
    assert_ne!(drifted.content_hash(), MANIFEST_HASHES[0]);
}

#[test]
fn climate_values_stay_in_physical_ranges() {
    for seed in MANIFEST_SEEDS {
        let world = world_with_climate(seed);
        let cell_count = world.grid.cells.len();
        let temperature = world.climate.temperature.as_ref().expect("filled");
        let humidity = world.climate.humidity.as_ref().expect("filled");
        let precipitation = world.climate.precipitation.as_ref().expect("filled");
        assert_eq!(temperature.len(), cell_count);
        for value in temperature {
            assert!(
                (TEMP_MIN_C..=TEMP_MAX_C).contains(&value.to_value()),
                "temperature {value:?} out of the physical range, seed {seed:#018x}"
            );
        }
        for value in humidity {
            assert!(
                (0.0..=100.0).contains(&value.to_value()),
                "humidity {value:?} out of the physical range, seed {seed:#018x}"
            );
        }
        for value in precipitation {
            assert!(
                value.to_value() >= 0.0 && value.to_value() <= 3000.0,
                "precipitation {value:?} out of the physical range, seed {seed:#018x}"
            );
        }
    }
}

#[test]
fn deep_ocean_cells_carry_full_pre_smoothing_humidity() {
    // Ocean cells are forced to saturation before the smoother runs, so
    // every cell whose full smoothing window (Chebyshev radius 3) is
    // ocean must stay at exactly 100 %.
    let world = world_with_climate(MANIFEST_SEEDS[0]);
    let width = world.grid.width as usize;
    let height = world.grid.height as usize;
    let humidity = world.climate.humidity.as_ref().expect("filled");
    let cells = &world.grid.cells;
    let radius = SMOOTHING_RADIUS;
    for y in 0..height {
        for x in 0..width {
            let mut window_is_ocean = true;
            for dy in 0..=2 * radius {
                for dx in 0..=2 * radius {
                    let sx = (x + dx).saturating_sub(radius).min(width - 1);
                    let sy = (y + dy).saturating_sub(radius).min(height - 1);
                    if !cells[sy * width + sx].is_water {
                        window_is_ocean = false;
                    }
                }
            }
            if window_is_ocean {
                assert_eq!(
                    humidity[y * width + x].to_value(),
                    100.0,
                    "deep ocean cell ({x}, {y}) must stay saturated"
                );
            }
        }
    }
}

#[test]
fn the_equator_band_is_warmer_than_the_polar_bands() {
    let world = world_with_climate(MANIFEST_SEEDS[0]);
    let height = world.grid.height as usize;
    let width = world.grid.width as usize;
    let temperature = world.climate.temperature.as_ref().expect("filled");
    let band_mean = |rows: std::ops::Range<usize>| -> f64 {
        let mut sum = 0.0;
        let mut count = 0;
        for y in rows {
            for x in 0..width {
                sum += temperature[y * width + x].to_value();
                count += 1;
            }
        }
        sum / f64::from(count as u32)
    };
    let polar_mean = band_mean(0..4) * 0.5 + band_mean(height - 4..height) * 0.5;
    let equatorial_mean = band_mean(height / 2 - 2..height / 2 + 2);
    assert!(
        equatorial_mean > polar_mean + 30.0,
        "the equator band ({equatorial_mean} °C) must be substantially warmer than the polar bands ({polar_mean} °C)"
    );
}

#[test]
#[ignore = "prints manifest hashes; run deliberately after an intended drift"]
fn regenerate_manifest() {
    for seed in MANIFEST_SEEDS {
        let hash = world_with_climate(seed).content_hash();
        println!("seed {seed:#018x} => 0x{hash:016x}");
    }
}
