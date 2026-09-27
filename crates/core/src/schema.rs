//! The `GeographicWorld` schema, version 0.
//!
//! This is the frozen data contract of the generator: grid, territories,
//! regions, water bodies, rivers, and the stage-owned sections (climate,
//! biomes) that later stages fill in without changing field semantics.
//! Any change to the meaning of a field requires a new `schema_version`.
//!
//! Public fields use typed lattices ([`crate::quant`]) and typed
//! identifiers ([`crate::id`]) only; raw integers and floats never appear
//! in the public surface.
//!
//! A world is serialized through the canonical binary format — see
//! [`GeographicWorld::to_bytes`] for a round-trip example.

use crate::id::{BiomeId, CellId, RegionId, RiverId, TerritoryId, WaterBodyId};
use crate::quant::{CentiScalar, HeightM, HumidDeciPct, NatPotential, PrecipMmYr, TempDeciC};

/// Schema version written by this crate. Any change to the meaning of an
/// existing field bumps this value.
pub const SCHEMA_VERSION: u32 = 0;

/// Sentinel for "no index" in cell back-references (`u32::MAX`). It is an
/// array-index sentinel, distinct from the reserved identifier value `0`.
pub const NO_INDEX: u32 = u32::MAX;

/// Parameters of the terrain stage (params version 2).
///
/// Values are stored on integer lattices so that parameters hash and
/// round-trip canonically like every other exported quantity. The
/// meaningful semantic domains are enforced by producers (see
/// `vernadsky-terrain`); the lattice types only bound the storage.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TerrainParams {
    /// Erosion configuration; `None` when the profile runs without
    /// erosion.
    pub erosion: Option<ErosionParams>,
}

/// Hydraulic erosion tuning (the thermal pass is deterministic and
/// keeps port constants).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ErosionParams {
    /// Water drops per hundred cells of the grid, in hundredths
    /// (the port convention is one percent of the map area).
    pub droplets_per_hundred_cells: CentiScalar,
    /// Erosion intensity of a droplet step, in hundredths
    /// (port convention 0.01–0.05).
    pub power: CentiScalar,
    /// Thermal talus angle: the height difference (in meters) above
    /// which material slides to a neighbor.
    pub talus: HeightM,
}

/// Parameters that, together with the generator version and its seed,
/// allow reproducing the world. Stage-specific parameters are added here
/// (with a `params_version` bump) as stages come online.
///
/// The version is derived from the structure: `0` is the bare
/// `{ seed }` layout of worlds generated before any stage parameters
/// existed, `1` adds the [`GenerationParams::climate`] block, `2` adds
/// the block-presence flags and the [`GenerationParams::terrain`]
/// block. Writers derive the version from the present blocks, readers
/// accept every known version and reject unknown ones with a
/// diagnostic.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct GenerationParams {
    /// Version of the parameters structure itself: `2` when the terrain
    /// block is present, `1` when only the climate block is, `0` for the
    /// bare seed.
    pub params_version: u32,
    /// Master seed of the generation. Stored from the start; consumed by
    /// the seeded stages as they come online.
    pub seed: u64,
    /// Parameters of the climate stage; present from params version 1.
    pub climate: Option<ClimateParams>,
    /// Parameters of the terrain stage; present from params version 2.
    pub terrain: Option<TerrainParams>,
}

/// Parameters of the climate stage (params version 1).
///
/// Values are stored on integer lattices so that parameters hash and
/// round-trip canonically like every other exported quantity. The
/// meaningful semantic domains are enforced by producers (see
/// `vernadsky-climate`); the lattice types only bound the storage.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClimateParams {
    /// Global temperature offset applied over the whole planet, in
    /// tenths of a degree Celsius.
    pub temperature_offset: TempDeciC,
    /// Latitude-dependent amplification of the temperature offset, in
    /// hundredths (conventional domain `0..=300`, i.e. `0..=3`).
    pub polar_amplification: CentiScalar,
    /// Compression exponent of the latitudinal temperature profile, in
    /// hundredths (conventional domain `50..=200`, i.e. `0.5..=2`).
    pub latitude_exponent: CentiScalar,
    /// Baseline moisture offset of the humidity field, in hundredths of
    /// the normalized moisture unit (conventional domain `-40..=40`).
    pub humidity_offset: CentiScalar,
}

/// Grid-cell adjacency convention. The convention is part of the contract:
/// stages and consumers derive neighborhood from it, not from assumption.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Connectivity {
    /// Edge-adjacent cells only.
    Four,
    /// Edge- and corner-adjacent cells.
    Eight,
}

impl Connectivity {
    /// Canonical byte encoding.
    pub fn to_byte(self) -> u8 {
        match self {
            Connectivity::Four => 0,
            Connectivity::Eight => 1,
        }
    }

    /// Decodes the canonical byte encoding.
    pub fn from_byte(value: u8) -> Option<Self> {
        match value {
            0 => Some(Connectivity::Four),
            1 => Some(Connectivity::Eight),
            _ => None,
        }
    }
}

/// The regular cell grid with per-cell state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridSection {
    /// Number of columns; `CellId` is row-major over this width.
    pub width: u32,
    /// Number of rows.
    pub height: u32,
    /// Adjacency convention of the grid.
    pub connectivity: Connectivity,
    /// Exactly `width * height` records, row-major.
    pub cells: Vec<CellRecord>,
}

/// Per-cell state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellRecord {
    /// Quantized surface height; sea surface is `0`, negative is depth.
    pub height: HeightM,
    /// Whether the cell is covered by water (sea, lake, or river surface
    /// handling is a stage decision; this flag is the physical state).
    pub is_water: bool,
    /// Index into `GeographicWorld::territories`, or [`NO_INDEX`].
    pub territory: u32,
    /// Index into `GeographicWorld::water_bodies`, or [`NO_INDEX`].
    pub water_body: u32,
}

/// An integer point in cell-index space: the anchor of an entity.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Anchor {
    /// Cell-index coordinate along the width axis.
    pub x: i64,
    /// Cell-index coordinate along the height axis.
    pub y: i64,
}

/// A territory: the neutral geographic partition unit (land or water).
/// Game-level constructs are consumer-side and out of scope here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerritoryRecord {
    /// Stable territory identifier.
    pub id: TerritoryId,
    /// Whether the territory partitions water rather than land.
    pub is_water: bool,
    /// Integer centroid of the member cells; the anchor behind the
    /// identifier. Exported so consumers can re-derive and verify IDs.
    pub anchor: Anchor,
    /// Declared number of member cells; validated against the grid.
    pub cell_count: u32,
    /// Starting natural potential of the territory, in permille.
    pub natural_potential: NatPotential,
    /// Region the territory belongs to, if any.
    pub region: Option<RegionId>,
}

/// A region: a macro-group of territories (continent or sea basin).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionRecord {
    /// Stable region identifier.
    pub id: RegionId,
}

/// Water body classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WaterBodyKind {
    /// Ocean.
    Ocean,
    /// Sea.
    Sea,
    /// Lake.
    Lake,
}

impl WaterBodyKind {
    /// Canonical byte encoding.
    pub fn to_byte(self) -> u8 {
        match self {
            WaterBodyKind::Ocean => 0,
            WaterBodyKind::Sea => 1,
            WaterBodyKind::Lake => 2,
        }
    }

    /// Decodes the canonical byte encoding.
    pub fn from_byte(value: u8) -> Option<Self> {
        match value {
            0 => Some(WaterBodyKind::Ocean),
            1 => Some(WaterBodyKind::Sea),
            2 => Some(WaterBodyKind::Lake),
            _ => None,
        }
    }
}

/// A classified water body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaterBodyRecord {
    /// Stable water body identifier.
    pub id: WaterBodyId,
    /// Classification of the body.
    pub kind: WaterBodyKind,
    /// Integer centroid of the member cells; the anchor behind the
    /// identifier.
    pub anchor: Anchor,
}

/// A river: an ordered path of land cells ending at its mouth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiverRecord {
    /// Stable river identifier, anchored at the mouth.
    pub id: RiverId,
    /// The mouth cell: the last path cell before a water body.
    pub mouth: Anchor,
    /// Ordered path cells from source towards the mouth.
    pub path: Vec<CellId>,
}

/// Climate fields, one value per cell. Filled by the climate stage;
/// `None` until then.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClimateSection {
    /// Per-cell temperature.
    pub temperature: Option<Vec<TempDeciC>>,
    /// Per-cell relative humidity.
    pub humidity: Option<Vec<HumidDeciPct>>,
    /// Per-cell annual precipitation.
    pub precipitation: Option<Vec<PrecipMmYr>>,
}

/// Biome assignment, one value per cell. Filled by the biome stage;
/// `None` until then. Identifier values come from the versioned biome
/// registry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BiomeSection {
    /// Per-cell biome.
    pub biome: Option<Vec<BiomeId>>,
}

/// The version-0 geographic world produced by the generator and consumed
/// through the canonical binary export format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeographicWorld {
    /// Generation parameters needed to reproduce the world.
    pub params: GenerationParams,
    /// The cell grid.
    pub grid: GridSection,
    /// Territory partition of the world.
    pub territories: Vec<TerritoryRecord>,
    /// Regions grouping territories.
    pub regions: Vec<RegionRecord>,
    /// Classified water bodies.
    pub water_bodies: Vec<WaterBodyRecord>,
    /// Rivers.
    pub rivers: Vec<RiverRecord>,
    /// Stage-owned climate fields.
    pub climate: ClimateSection,
    /// Stage-owned biome fields.
    pub biomes: BiomeSection,
}

impl GeographicWorld {
    /// Serializes the world into the canonical binary format.
    pub fn to_bytes(&self) -> Vec<u8> {
        crate::format::to_bytes(self)
    }

    /// Decodes and validates a canonical export.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, crate::format::FormatError> {
        crate::format::from_bytes(bytes)
    }

    /// The content hash this world carries when serialized: FNV-1a 64 over
    /// the canonical serialization without the trailing hash field.
    pub fn content_hash(&self) -> u64 {
        let bytes = self.to_bytes();
        crate::format::stored_content_hash(&bytes).expect("serialization carries a hash")
    }
}
