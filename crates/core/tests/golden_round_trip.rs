//! Golden round-trip tests for the canonical export format.
//!
//! `golden/minimal_world_v0.gwb` is the checked-in serialization of
//! [`vernadsky_core::synthetic::minimal_world`]. It pins the byte-level
//! format and the deterministic constructor: any change to the schema
//! layout, the serialization, or the synthetic world changes these bytes
//! and must be reviewed deliberately.
//!
//! # Regenerating the golden file
//!
//! ```text
//! cargo test --test golden_round_trip regenerate_golden -- --ignored
//! ```
//!
//! Run this only when a format or constructor change is intended, and
//! commit the new file together with the change that caused the drift.

use vernadsky_core::{GeographicWorld, synthetic};

const GOLDEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/golden/minimal_world_v0.gwb"
);

#[test]
fn constructed_world_matches_the_golden_file() {
    let bytes = synthetic::minimal_world().to_bytes();
    let golden = std::fs::read(GOLDEN_PATH).expect("golden file is checked in");
    assert_eq!(
        bytes, golden,
        "the synthetic world or the format drifted from the golden file"
    );
}

#[test]
fn golden_file_round_trips() {
    let golden = std::fs::read(GOLDEN_PATH).expect("golden file is checked in");
    let world = GeographicWorld::from_bytes(&golden).expect("golden file must validate");
    assert_eq!(world.to_bytes(), golden);
}

#[test]
fn golden_file_carries_a_consistent_content_hash() {
    let golden = std::fs::read(GOLDEN_PATH).expect("golden file is checked in");
    let world = GeographicWorld::from_bytes(&golden).expect("golden file must validate");
    let stored = vernadsky_core::format::stored_content_hash(&golden).expect("8 trailing bytes");
    assert_eq!(world.content_hash(), stored);
}

#[test]
#[ignore = "regenerates the golden file; run deliberately after an intended format change"]
fn regenerate_golden() {
    let bytes = synthetic::minimal_world().to_bytes();
    std::fs::write(GOLDEN_PATH, bytes).expect("golden file is writable");
}
