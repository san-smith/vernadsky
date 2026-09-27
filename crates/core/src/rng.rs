//! Named random number streams (strategy `rng-streams-v1`).
//!
//! Generation draws its randomness from independent, reproducible streams.
//! A stream is addressed by `(world seed, stream name)` and derived as:
//!
//! ```text
//! input = [strategy_tag][stream name, UTF-8][world seed, u64 LE]
//! seed  = FNV-1a 64(input)                     (crate::idgen::fnv1a64)
//! rng   = ChaCha8Rng::seed_from_u64(seed)      (rand_core documented fill)
//! ```
//!
//! Properties, pinned by tests:
//!
//! - **Determinism.** The same world seed and name produce byte-identical
//!   sequences on every run and platform (ChaCha8 is integer-only).
//! - **Locality.** Introducing a new stream name never changes the bytes
//!   of existing streams, so stages come online without shifting each
//!   other's behavior.
//! - **Purity / re-entrancy.** [`RngStreams::stream`] is a pure function:
//!   every call returns the stream at position zero. The stage that owns
//!   a stream owns its consumption order.
//!
//! Naming convention: lowercase string constants owned by their stage,
//! dotted for hierarchy (`"heightmap"`, `"climate.temperature"`). A
//! rename changes the stream's output and is therefore a strategy
//! version change. Any change to the derivation is a new
//! [`RNG_STRATEGY`] value plus a breaking schema change.
//!
//! ```
//! use vernadsky_core::RngStreams;
//! use rand_chacha::rand_core::RngCore;
//!
//! let streams = RngStreams::new(0x5EED);
//! let mut climate = streams.stream("climate.temperature");
//! let mut rivers = streams.stream("rivers");
//!
//! let mut a = [0u8; 16];
//! climate.fill_bytes(&mut a);
//! let mut b = [0u8; 16];
//! rivers.fill_bytes(&mut b);
//! assert_ne!(a, b, "independent streams diverge");
//!
//! let mut replayed = streams.stream("climate.temperature");
//! let mut c = [0u8; 16];
//! replayed.fill_bytes(&mut c);
//! assert_eq!(a, c, "streams are re-entrant: same name, same position");
//! ```

use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::{RngCore, SeedableRng};

use crate::idgen::fnv1a64;

/// Identifier derivation strategy written into every export.
pub const RNG_STRATEGY: &str = "rng-streams-v1";

const STRATEGY_TAG: u8 = 1;

/// Factory of named ChaCha8 streams for one world seed.
///
/// The factory is valueless apart from the world seed: constructing it,
/// cloning it, or re-creating it has no effect on stream contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RngStreams {
    world_seed: u64,
}

impl RngStreams {
    /// Creates the stream factory for a world seed.
    pub fn new(world_seed: u64) -> Self {
        Self { world_seed }
    }

    /// The world seed this factory derives streams from.
    pub fn world_seed(&self) -> u64 {
        self.world_seed
    }

    /// Derives the stream for `name` at position zero.
    ///
    /// Pure and re-entrant: the result depends only on `(world_seed,
    /// name)`. The caller must treat the returned generator as owned by
    /// one stage and consume it in a stage-determined order.
    pub fn stream(&self, name: &str) -> ChaCha8Rng {
        let mut input = Vec::with_capacity(name.len() + 9);
        input.push(STRATEGY_TAG);
        input.extend_from_slice(name.as_bytes());
        input.extend_from_slice(&self.world_seed.to_le_bytes());
        ChaCha8Rng::seed_from_u64(fnv1a64(&input))
    }

    /// Fills `out` with the leading bytes of the named stream.
    ///
    /// Convenience for stages that only need a few deterministic bytes;
    /// equivalent to consuming [`RngStreams::stream`] from position zero.
    pub fn stream_bytes(&self, name: &str, out: &mut [u8]) {
        self.stream(name).fill_bytes(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::rand_core::RngCore;

    fn take_bytes(rng: &mut ChaCha8Rng, n: usize) -> Vec<u8> {
        let mut out = vec![0u8; n];
        rng.fill_bytes(&mut out);
        out
    }

    #[test]
    fn same_seed_and_name_reproduce_bytes_exactly() {
        let mut first = RngStreams::new(0x1234).stream("heightmap");
        let mut second = RngStreams::new(0x1234).stream("heightmap");
        assert_eq!(take_bytes(&mut first, 256), take_bytes(&mut second, 256));
    }

    #[test]
    fn different_names_diverge() {
        let streams = RngStreams::new(0x1234);
        let heightmap = take_bytes(&mut streams.stream("heightmap"), 64);
        let climate = take_bytes(&mut streams.stream("climate.temperature"), 64);
        assert_ne!(heightmap, climate);
    }

    #[test]
    fn world_seed_shifts_every_stream() {
        let names = ["heightmap", "climate.temperature", "rivers"];
        for name in names {
            let before = take_bytes(&mut RngStreams::new(1).stream(name), 64);
            let after = take_bytes(&mut RngStreams::new(2).stream(name), 64);
            assert_ne!(before, after, "seed must shift stream {name}");
        }
    }

    #[test]
    fn locality_introducing_a_name_leaves_others_untouched() {
        let without = RngStreams::new(7);
        let mut explicit = Vec::new();
        for name in ["heightmap", "climate.temperature"] {
            explicit.push(take_bytes(&mut without.stream(name), 64));
        }

        // A later pipeline version adds a stage with a new stream name.
        let with = RngStreams::new(7);
        let _new = with.stream("territory.seeds");
        let mut after = Vec::new();
        for name in ["heightmap", "climate.temperature"] {
            after.push(take_bytes(&mut with.stream(name), 64));
        }
        assert_eq!(explicit, after, "existing streams must not shift");
    }

    #[test]
    fn names_are_prefix_sensitive() {
        let streams = RngStreams::new(9);
        let short = take_bytes(&mut streams.stream("climate"), 32);
        let nested = take_bytes(&mut streams.stream("climate.temperature"), 32);
        assert_ne!(short, nested);
    }

    #[test]
    fn strategy_is_pinned() {
        assert_eq!(RNG_STRATEGY, "rng-streams-v1");
    }
}
