//! Derivation of stable identifiers (strategy `content-hash-v1`).
//!
//! Identifier values are derived from the entity's canonical integer
//! anchor, so they are stable whenever the anchor is stable, and deriving
//! them never touches floats, wall-clock time, or collection iteration
//! order.
//!
//! Mechanism, pinned by this module and versioned through
//! [`ID_STRATEGY`]:
//!
//! 1. Build the canonical input: `[strategy_tag][kind_tag][anchor fields,
//!    i64 little-endian][kind attributes, u8][probe: u32 little-endian]`.
//! 2. Hash it with FNV-1a 64 (in-crate, see [`fnv1a64`]).
//! 3. Take the low 63 bits; `0` is reserved and counts as a collision.
//! 4. On collision, increment the probe counter and repeat. Entities are
//!    assigned in a canonical order defined by the caller, which makes the
//!    whole sequence deterministic.
//!
//! Any change to this layout, to anchor definitions, or to the hash is a
//! new `id_strategy` value plus a breaking schema change.

use std::collections::BTreeSet;
use std::fmt;

use crate::id::{RiverId, TerritoryId, WaterBodyId};

/// Identifier derivation strategy written into every export.
pub const ID_STRATEGY: &str = "content-hash-v1";

const STRATEGY_TAG: u8 = 1;
const KIND_TERRITORY: u8 = 1;
const KIND_WATER_BODY: u8 = 2;
const KIND_RIVER: u8 = 3;

/// FNV-1a 64 offset basis.
pub const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64 prime.
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Low 63 bits carry the identifier; the top bit stays clear.
const ID_MASK: u64 = 0x7fff_ffff_ffff_ffff;
/// The reserved unset value; never assigned.
const RESERVED_ID: u64 = 0;
/// Defensive cap on collision probes; practically unreachable.
const PROBE_LIMIT: u32 = 1 << 20;

/// FNV-1a 64: the in-crate, dependency-free hash behind the strategy.
///
/// The `wrapping` multiplication is the definition of FNV-1a (arithmetic
/// modulo 2⁶⁴), not an overflow hazard.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Errors produced by identifier assignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdGenError {
    /// The probe loop hit its defensive cap without finding a free value.
    ProbeLimitExceeded,
}

impl fmt::Display for IdGenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdGenError::ProbeLimitExceeded => {
                write!(f, "identifier probe limit exceeded without a free value")
            }
        }
    }
}

impl std::error::Error for IdGenError {}

/// Assigns identifiers within one kind namespace.
#[derive(Debug, Default)]
struct IdSpace {
    used: BTreeSet<u64>,
}

impl IdSpace {
    /// Builds the canonical input for `probe = 0..PROBE_LIMIT` and returns
    /// the first free derived value.
    fn assign(&mut self, kind_tag: u8, anchors: &[i64], attrs: &[u8]) -> Result<u64, IdGenError> {
        let mut input = Vec::with_capacity(2 + anchors.len() * 8 + attrs.len() + 4);
        input.push(STRATEGY_TAG);
        input.push(kind_tag);
        for anchor in anchors {
            input.extend_from_slice(&anchor.to_le_bytes());
        }
        input.extend_from_slice(attrs);
        let probe_offset = input.len();
        input.extend_from_slice(&[0, 0, 0, 0]);

        for probe in 0..PROBE_LIMIT {
            input[probe_offset..probe_offset + 4].copy_from_slice(&probe.to_le_bytes());
            let candidate = fnv1a64(&input) & ID_MASK;
            if candidate != RESERVED_ID && self.used.insert(candidate) {
                return Ok(candidate);
            }
        }
        Err(IdGenError::ProbeLimitExceeded)
    }
}

/// Assigns stable identifiers across all kind namespaces of one world.
///
/// The caller must assign entities in a canonical order (for territories
/// and water bodies: ascending `(anchor.y, anchor.x)`), which makes the
/// derived identifiers fully deterministic.
#[derive(Debug, Default)]
pub struct IdAssigner {
    territories: IdSpace,
    water_bodies: IdSpace,
    rivers: IdSpace,
}

impl IdAssigner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Derives a territory identifier from its integer centroid and its
    /// land/water attribute.
    pub fn territory(
        &mut self,
        centroid_x: i64,
        centroid_y: i64,
        is_water: bool,
    ) -> Result<TerritoryId, IdGenError> {
        let attrs = [u8::from(is_water)];
        Ok(TerritoryId(self.territories.assign(
            KIND_TERRITORY,
            &[centroid_x, centroid_y],
            &attrs,
        )?))
    }

    /// Derives a water body identifier from its integer centroid.
    pub fn water_body(
        &mut self,
        centroid_x: i64,
        centroid_y: i64,
    ) -> Result<WaterBodyId, IdGenError> {
        Ok(WaterBodyId(self.water_bodies.assign(
            KIND_WATER_BODY,
            &[centroid_x, centroid_y],
            &[],
        )?))
    }

    /// Derives a river identifier from its mouth cell.
    pub fn river(&mut self, mouth_x: i64, mouth_y: i64) -> Result<RiverId, IdGenError> {
        Ok(RiverId(self.rivers.assign(
            KIND_RIVER,
            &[mouth_x, mouth_y],
            &[],
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_matches_reference_vectors() {
        assert_eq!(fnv1a64(b""), FNV_OFFSET_BASIS);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn identical_anchors_get_distinct_ids_across_namespaces() {
        let mut assigner = IdAssigner::new();
        let t = assigner.territory(10, 20, false).expect("assigned");
        let w = assigner.water_body(10, 20).expect("assigned");
        assert_ne!(t.0, w.0, "kind tag participates in the hash input");
    }

    #[test]
    fn probe_resolves_collisions_deterministically() {
        // Two entities with the same anchor collide on the first probe;
        // both assigners must resolve them identically.
        let mut first = IdAssigner::new();
        let mut second = IdAssigner::new();
        let a1 = first.territory(7, 7, false).expect("assigned");
        let a2 = first.territory(7, 7, false).expect("assigned");
        let b1 = second.territory(7, 7, false).expect("assigned");
        let b2 = second.territory(7, 7, false).expect("assigned");
        assert_eq!(a1, b1);
        assert_eq!(a2, b2);
        assert_ne!(a1, a2, "collision must not collapse into one value");
    }

    #[test]
    fn ids_stay_in_the_63_bit_domain() {
        let mut assigner = IdAssigner::new();
        for x in 0..64 {
            let t = assigner
                .territory(i64::from(x), 0, false)
                .expect("assigned");
            assert_eq!(t.0 & !ID_MASK, 0, "top bit must stay clear");
            assert_ne!(t.0, 0, "reserved value must never be assigned");
        }
    }
}
