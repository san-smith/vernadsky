//! Vernadsky core: seeded planetary geography generation.
//!
//! The generator core — the `GeographicWorld` schema, the generation
//! pipeline, and the versioned export — is developed in this crate. It
//! intentionally defines no public geography API yet; nothing here is stable
//! before the schema is frozen and versioned.
//!
//! # Invariants
//!
//! - Generation is a pure function of a seed plus parameters: identical
//!   inputs produce identical output on every run and every supported
//!   platform. The generation path uses no wall-clock time, thread
//!   scheduling, or external entropy.
//! - Results never depend on hash-map iteration order; ordered structures
//!   and stable identifiers are used instead.
//! - The crate contains no `unsafe` code; this is enforced by the workspace
//!   lints, not by convention.

pub mod quant;
