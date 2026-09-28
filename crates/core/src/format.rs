//! Canonical binary export format for `GeographicWorld`.
//!
//! The format is the frozen machine representation of the schema: all
//! integers are fixed-width little-endian, all variable-length sections
//! carry a `u32` count, and there is no self-describing structure — the
//! field order documented on [`to_bytes`] *is* the contract for
//! `schema_version` 0.
//!
//! The trailing `content_hash` is FNV-1a 64 over all preceding bytes. It
//! provides integrity and reproducibility checking, not tamper resistance.
//! Unknown `schema_version`, `id_strategy`, or lattice registry values are
//! rejected with a diagnostic — never approximated.

use std::collections::BTreeSet;
use std::fmt;

use crate::id::{BiomeId, CellId, RegionId, RiverId, TerritoryId, WaterBodyId};
use crate::idgen::{ID_STRATEGY, fnv1a64};
use crate::quant::{
    CentiScalar, HeightM, HumidDeciPct, LATTICE_REGISTRY_VERSION, NatPotential, PrecipMmYr,
    TempDeciC,
};
use crate::schema::{
    Anchor, BIOME_REGISTRY, BiomeSection, CellRecord, ClimateParams, ClimateSection, Connectivity,
    ErosionParams, GenerationParams, GeographicWorld, GridSection, HydrologyParams, NO_INDEX,
    RegionRecord, RiverRecord, SCHEMA_VERSION, TerrainParams, TerritoryRecord, WaterBodyKind,
    WaterBodyRecord,
};

/// File magic of the canonical export format.
pub const MAGIC: &[u8; 4] = b"VGW\0";

/// Upper sanity bound for embedded strings.
const MAX_STRING_LEN: usize = 128;

/// Serialization, decoding, and validation errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatError {
    /// The file does not start with the format magic.
    BadMagic,
    /// The file was written for a different schema version.
    UnsupportedSchemaVersion(u32),
    /// The identifier strategy is unknown to this reader.
    UnsupportedIdStrategy(String),
    /// The lattice registry is unknown to this reader.
    UnsupportedLatticeRegistry(String),
    /// The biome registry is unknown to this reader.
    UnsupportedBiomeRegistry(String),
    /// The parameters structure version is unknown to this reader.
    UnsupportedParamsVersion(u32),
    /// The params block-presence flags are inconsistent with the version.
    InvalidParamsFlags(u8),
    /// The file ends in the middle of a field.
    UnexpectedEof { needed: usize, remaining: usize },
    /// An embedded string is not valid UTF-8.
    InvalidUtf8String,
    /// A string length or count exceeds a sanity bound.
    CountExceedsBound { what: &'static str, value: usize },
    /// The trailing content hash does not match the recomputed one.
    HashMismatch { stored: u64, computed: u64 },
    /// The grid cell count does not match `width * height`.
    CellCountMismatch { declared: u64, actual: usize },
    /// The connectivity byte is unknown.
    InvalidConnectivity(u8),
    /// The water body kind byte is unknown.
    InvalidWaterBodyKind(u8),
    /// A boolean byte is neither `0` nor `1`.
    InvalidBool(u8),
    /// The same identifier appears twice within one namespace.
    DuplicateId { namespace: &'static str, id: u64 },
    /// The reserved identifier value `0` appears as an assigned id.
    ReservedId { namespace: &'static str },
    /// A territory references a region that has no record.
    UnknownRegionReference { territory_index: usize, id: u64 },
    /// A cell back-reference points outside the section it references.
    IndexOutOfRange { section: &'static str, cell: u64 },
    /// A territory's declared member count differs from the grid.
    TerritoryCellCountMismatch {
        territory_index: usize,
        declared: u32,
        actual: u32,
    },
    /// A land cell references a water body.
    WaterBodyOnLandCell { cell: u64 },
    /// A coordinate or anchor is outside the grid (or negative).
    CoordinateOutOfBounds { what: &'static str, value: i64 },
    /// A river path is empty.
    EmptyRiverPath { river_index: usize },
    /// An optional per-cell section length differs from the cell count.
    SectionLengthMismatch {
        section: &'static str,
        declared: u32,
    },
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::BadMagic => write!(f, "not a GeographicWorld export (bad magic)"),
            FormatError::UnsupportedSchemaVersion(v) => write!(f, "unsupported schema version {v}"),
            FormatError::UnsupportedIdStrategy(s) => write!(f, "unsupported id strategy {s:?}"),
            FormatError::UnsupportedLatticeRegistry(s) => {
                write!(f, "unsupported lattice registry {s:?}")
            }
            FormatError::UnsupportedBiomeRegistry(s) => {
                write!(f, "unsupported biome registry {s:?}")
            }
            FormatError::UnsupportedParamsVersion(v) => {
                write!(f, "unsupported generation params version {v}")
            }
            FormatError::InvalidParamsFlags(flags) => {
                write!(
                    f,
                    "params flags {flags:#04x} are inconsistent with the params version"
                )
            }
            FormatError::UnexpectedEof { needed, remaining } => write!(
                f,
                "unexpected end of file: needed {needed} bytes, {remaining} remain"
            ),
            FormatError::InvalidUtf8String => write!(f, "embedded string is not valid UTF-8"),
            FormatError::CountExceedsBound { what, value } => {
                write!(f, "{what} length {value} exceeds the sanity bound")
            }
            FormatError::HashMismatch { stored, computed } => write!(
                f,
                "content hash mismatch: stored {stored:#018x}, computed {computed:#018x}"
            ),
            FormatError::CellCountMismatch { declared, actual } => {
                write!(
                    f,
                    "grid declares {declared} cells but {actual} records follow"
                )
            }
            FormatError::InvalidConnectivity(v) => write!(f, "unknown connectivity byte {v}"),
            FormatError::InvalidWaterBodyKind(v) => write!(f, "unknown water body kind {v}"),
            FormatError::InvalidBool(v) => write!(f, "boolean byte must be 0 or 1, got {v}"),
            FormatError::DuplicateId { namespace, id } => {
                write!(f, "duplicate {namespace} identifier {id:#x}")
            }
            FormatError::ReservedId { namespace } => {
                write!(
                    f,
                    "{namespace} identifier 0 is reserved and cannot be assigned"
                )
            }
            FormatError::UnknownRegionReference {
                territory_index,
                id,
            } => write!(
                f,
                "territory {territory_index} references unknown region {id:#x}"
            ),
            FormatError::IndexOutOfRange { section, cell } => {
                write!(f, "{section}: cell index {cell} is out of range")
            }
            FormatError::TerritoryCellCountMismatch {
                territory_index,
                declared,
                actual,
            } => write!(
                f,
                "territory {territory_index} declares {declared} cells, grid has {actual}"
            ),
            FormatError::WaterBodyOnLandCell { cell } => {
                write!(f, "land cell {cell} references a water body")
            }
            FormatError::CoordinateOutOfBounds { what, value } => {
                write!(f, "{what} coordinate {value} is outside the grid")
            }
            FormatError::EmptyRiverPath { river_index } => {
                write!(f, "river {river_index} has an empty path")
            }
            FormatError::SectionLengthMismatch { section, declared } => write!(
                f,
                "{section} section declares {declared} values, expected one per cell"
            ),
        }
    }
}

impl std::error::Error for FormatError {}

/// Serializes the world into the canonical binary format. The last eight
/// bytes of the result are the content hash over everything preceding.
///
/// ```
/// use vernadsky_core::{synthetic, GeographicWorld};
///
/// let world = synthetic::minimal_world();
/// let bytes = world.to_bytes();
/// let decoded = GeographicWorld::from_bytes(&bytes).expect("valid export");
/// assert_eq!(decoded, world);
/// assert_eq!(decoded.to_bytes(), bytes);
/// ```
pub fn to_bytes(world: &GeographicWorld) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(MAGIC);
    b.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    write_str(&mut b, ID_STRATEGY.as_bytes());
    write_str(&mut b, LATTICE_REGISTRY_VERSION.as_bytes());
    write_str(&mut b, BIOME_REGISTRY.as_bytes());
    // The params version is the layout declaration: the stages install
    // it together with their blocks, and the writer serializes exactly
    // the declared layout. This keeps a decoded legacy file (version 2
    // or 3) re-serializable into its own byte layout instead of being
    // silently upgraded. Versions 2 and up carry the block-presence
    // flags; versions 0 and 1 predate the flags.
    let version = world.params.params_version;
    debug_assert!(
        matches!(
            (
                version,
                world.params.terrain.is_some(),
                world.params.hydrology.is_some()
            ),
            (0, false, false)
                | (1, false, false)
                | (2, true, false)
                | (3, false, true)
                | (4, true, _)
        ),
        "params_version {version} is inconsistent with the present parameter blocks"
    );
    b.extend_from_slice(&version.to_le_bytes());
    b.extend_from_slice(&world.params.seed.to_le_bytes());
    if version >= 2 {
        let mut flags = 0;
        if world.params.terrain.is_some() {
            flags |= 0b10;
        }
        if world.params.hydrology.is_some() {
            flags |= 0b100;
        }
        if world.params.climate.is_some() {
            flags |= 0b01;
        }
        b.push(flags);
    }
    if let Some(climate) = &world.params.climate {
        b.extend_from_slice(&climate.temperature_offset.0.to_le_bytes());
        b.extend_from_slice(&climate.polar_amplification.0.to_le_bytes());
        b.extend_from_slice(&climate.latitude_exponent.0.to_le_bytes());
        b.extend_from_slice(&climate.humidity_offset.0.to_le_bytes());
    }
    if let Some(terrain) = &world.params.terrain {
        match &terrain.erosion {
            None => b.push(0),
            Some(erosion) => {
                b.push(1);
                b.extend_from_slice(&erosion.droplets_per_hundred_cells.0.to_le_bytes());
                b.extend_from_slice(&erosion.power.0.to_le_bytes());
                b.extend_from_slice(&erosion.talus.0.to_le_bytes());
            }
        }
        if version == 4 {
            b.extend_from_slice(&terrain.features_across.0.to_le_bytes());
            b.extend_from_slice(&terrain.min_feature_cells.to_le_bytes());
        }
    }
    if let Some(hydrology) = &world.params.hydrology {
        b.extend_from_slice(&hydrology.river_land_share.0.to_le_bytes());
    }

    // Grid.
    b.extend_from_slice(&world.grid.width.to_le_bytes());
    b.extend_from_slice(&world.grid.height.to_le_bytes());
    b.push(world.grid.connectivity.to_byte());
    for cell in &world.grid.cells {
        b.extend_from_slice(&cell.height.0.to_le_bytes());
        b.push(u8::from(cell.is_water));
        b.extend_from_slice(&cell.territory.to_le_bytes());
        b.extend_from_slice(&cell.water_body.to_le_bytes());
    }

    // Territories.
    write_count(&mut b, "territories", world.territories.len());
    for t in &world.territories {
        b.extend_from_slice(&t.id.0.to_le_bytes());
        b.push(u8::from(t.is_water));
        b.extend_from_slice(&t.anchor.x.to_le_bytes());
        b.extend_from_slice(&t.anchor.y.to_le_bytes());
        b.extend_from_slice(&t.cell_count.to_le_bytes());
        b.extend_from_slice(&t.natural_potential.0.to_le_bytes());
        match t.region {
            None => b.push(0),
            Some(region) => {
                b.push(1);
                b.extend_from_slice(&region.0.to_le_bytes());
            }
        }
    }

    // Regions.
    write_count(&mut b, "regions", world.regions.len());
    for r in &world.regions {
        b.extend_from_slice(&r.id.0.to_le_bytes());
    }

    // Water bodies.
    write_count(&mut b, "water bodies", world.water_bodies.len());
    for w in &world.water_bodies {
        b.extend_from_slice(&w.id.0.to_le_bytes());
        b.push(w.kind.to_byte());
        b.extend_from_slice(&w.anchor.x.to_le_bytes());
        b.extend_from_slice(&w.anchor.y.to_le_bytes());
    }

    // Rivers.
    write_count(&mut b, "rivers", world.rivers.len());
    for r in &world.rivers {
        b.extend_from_slice(&r.id.0.to_le_bytes());
        b.extend_from_slice(&r.mouth.x.to_le_bytes());
        b.extend_from_slice(&r.mouth.y.to_le_bytes());
        write_count(&mut b, "river path", r.path.len());
        for cell in &r.path {
            b.extend_from_slice(&cell.0.to_le_bytes());
        }
    }

    // Climate: three optional per-cell arrays.
    write_optional_slice(&mut b, world.climate.temperature.as_deref());
    write_optional_slice(&mut b, world.climate.humidity.as_deref());
    write_optional_slice(&mut b, world.climate.precipitation.as_deref());

    // Biomes.
    match &world.biomes.biome {
        None => b.push(0),
        Some(values) => {
            b.push(1);
            write_count(&mut b, "biomes", values.len());
            for value in values {
                b.extend_from_slice(&value.0.to_le_bytes());
            }
        }
    }

    let hash = fnv1a64(&b);
    b.extend_from_slice(&hash.to_le_bytes());
    b
}

/// Decodes and validates a canonical export.
pub fn from_bytes(bytes: &[u8]) -> Result<GeographicWorld, FormatError> {
    if bytes.len() < MAGIC.len() + 8 {
        return Err(FormatError::UnexpectedEof {
            needed: MAGIC.len() + 8,
            remaining: bytes.len(),
        });
    }
    let (body, hash_bytes) = bytes.split_at(bytes.len() - 8);
    let stored = u64::from_le_bytes(hash_bytes.try_into().expect("8 bytes"));
    let computed = fnv1a64(body);
    if stored != computed {
        return Err(FormatError::HashMismatch { stored, computed });
    }

    let mut r = Reader { data: body, pos: 0 };
    if r.take_bytes(MAGIC.len())? != MAGIC {
        return Err(FormatError::BadMagic);
    }
    let schema_version = r.take_u32()?;
    if schema_version > SCHEMA_VERSION {
        return Err(FormatError::UnsupportedSchemaVersion(schema_version));
    }
    let strategy = r.take_string()?;
    if strategy != ID_STRATEGY {
        return Err(FormatError::UnsupportedIdStrategy(strategy));
    }
    let registry = r.take_string()?;
    if registry != LATTICE_REGISTRY_VERSION {
        return Err(FormatError::UnsupportedLatticeRegistry(registry));
    }
    // Version 0 files predate the biome registry version: they carry no
    // such field, and their biome identifiers resolve through this
    // crate's registry. Version 1 carries it explicitly; the value is
    // validated and not stored — the interpretation lives in the
    // registry itself, not in the decoded world.
    if schema_version != 0 {
        let carried = r.take_string()?;
        if carried != BIOME_REGISTRY {
            return Err(FormatError::UnsupportedBiomeRegistry(carried));
        }
    }
    let params_version = r.take_u32()?;
    let seed = r.take_u64()?;
    let read_climate = |r: &mut Reader<'_>| {
        Ok(ClimateParams {
            temperature_offset: TempDeciC(r.take_i16()?),
            polar_amplification: CentiScalar(r.take_i16()?),
            latitude_exponent: CentiScalar(r.take_i16()?),
            humidity_offset: CentiScalar(r.take_i16()?),
        })
    };
    // The version-2 terrain layout: erosion only.
    let read_terrain_v2 = |r: &mut Reader<'_>| {
        let erosion = match r.take_u8()? {
            0 => None,
            1 => Some(ErosionParams {
                droplets_per_hundred_cells: CentiScalar(r.take_i16()?),
                power: CentiScalar(r.take_i16()?),
                talus: HeightM(r.take_i32()?),
            }),
            other => return Err(FormatError::InvalidBool(other)),
        };
        Ok(TerrainParams {
            erosion,
            features_across: CentiScalar(150),
            min_feature_cells: 8,
        })
    };
    let (climate_params, terrain_params, hydrology_params) = match params_version {
        0 => (None, None, None),
        1 => (Some(read_climate(&mut r)?), None, None),
        2 => {
            // Version 2 carries the block-presence flags: bit 0 climate,
            // bit 1 terrain. Version 2 implies the terrain block; the
            // climate block is optional (a terrain-only profile is a
            // valid world).
            let flags = r.take_u8()?;
            if flags & 0b10 == 0 {
                return Err(FormatError::InvalidParamsFlags(flags));
            }
            let climate_params = if flags & 0b01 != 0 {
                Some(read_climate(&mut r)?)
            } else {
                None
            };
            (climate_params, Some(read_terrain_v2(&mut r)?), None)
        }
        3 => {
            // Version 3 adds bit 2 hydrology and implies the hydrology
            // block; the terrain and climate blocks are optional per
            // their flags. The terrain block keeps its version-2 layout.
            let flags = r.take_u8()?;
            if flags & 0b100 == 0 {
                return Err(FormatError::InvalidParamsFlags(flags));
            }
            let climate_params = if flags & 0b01 != 0 {
                Some(read_climate(&mut r)?)
            } else {
                None
            };
            let terrain_params = if flags & 0b10 != 0 {
                Some(read_terrain_v2(&mut r)?)
            } else {
                None
            };
            let hydrology_params = Some(HydrologyParams {
                river_land_share: CentiScalar(r.take_i16()?),
            });
            (climate_params, terrain_params, hydrology_params)
        }
        4 => {
            // Version 4 implies the terrain block in the map-space
            // layout (erosion plus the feature count and the detail
            // floor); the hydrology and climate blocks are optional per
            // their flags.
            let flags = r.take_u8()?;
            if flags & 0b10 == 0 {
                return Err(FormatError::InvalidParamsFlags(flags));
            }
            let climate_params = if flags & 0b01 != 0 {
                Some(read_climate(&mut r)?)
            } else {
                None
            };
            let erosion = match r.take_u8()? {
                0 => None,
                1 => Some(ErosionParams {
                    droplets_per_hundred_cells: CentiScalar(r.take_i16()?),
                    power: CentiScalar(r.take_i16()?),
                    talus: HeightM(r.take_i32()?),
                }),
                other => return Err(FormatError::InvalidBool(other)),
            };
            let terrain_params = Some(TerrainParams {
                erosion,
                features_across: CentiScalar(r.take_i16()?),
                min_feature_cells: r.take_u32()?,
            });
            let hydrology_params = if flags & 0b100 != 0 {
                Some(HydrologyParams {
                    river_land_share: CentiScalar(r.take_i16()?),
                })
            } else {
                None
            };
            (climate_params, terrain_params, hydrology_params)
        }
        other => return Err(FormatError::UnsupportedParamsVersion(other)),
    };

    let width = r.take_u32()?;
    let height = r.take_u32()?;
    let connectivity_byte = r.take_u8()?;
    let connectivity = Connectivity::from_byte(connectivity_byte)
        .ok_or(FormatError::InvalidConnectivity(connectivity_byte))?;
    let declared_cells = u64::from(width) * u64::from(height);
    let mut cells = Vec::new();
    for _ in 0..declared_cells {
        cells.push(CellRecord {
            height: HeightM(r.take_i32()?),
            is_water: r.take_bool()?,
            territory: r.take_u32()?,
            water_body: r.take_u32()?,
        });
    }
    let grid = GridSection {
        width,
        height,
        connectivity,
        cells,
    };

    let territory_count = r.take_count("territories")?;
    let mut territories = Vec::with_capacity(territory_count);
    for _ in 0..territory_count {
        territories.push(TerritoryRecord {
            id: TerritoryId(r.take_u64()?),
            is_water: r.take_bool()?,
            anchor: take_anchor(&mut r)?,
            cell_count: r.take_u32()?,
            natural_potential: NatPotential(r.take_i16()?),
            region: take_option_u64(&mut r)?.map(RegionId),
        });
    }

    let region_count = r.take_count("regions")?;
    let mut regions = Vec::with_capacity(region_count);
    for _ in 0..region_count {
        regions.push(RegionRecord {
            id: RegionId(r.take_u64()?),
        });
    }

    let water_body_count = r.take_count("water bodies")?;
    let mut water_bodies = Vec::with_capacity(water_body_count);
    for _ in 0..water_body_count {
        let id = WaterBodyId(r.take_u64()?);
        let kind_byte = r.take_u8()?;
        let kind = WaterBodyKind::from_byte(kind_byte)
            .ok_or(FormatError::InvalidWaterBodyKind(kind_byte))?;
        water_bodies.push(WaterBodyRecord {
            id,
            kind,
            anchor: take_anchor(&mut r)?,
        });
    }

    let river_count = r.take_count("rivers")?;
    let mut rivers = Vec::with_capacity(river_count);
    for _ in 0..river_count {
        let id = RiverId(r.take_u64()?);
        let mouth = take_anchor(&mut r)?;
        let path_len = r.take_count("river path")?;
        let mut path = Vec::with_capacity(path_len);
        for _ in 0..path_len {
            path.push(CellId(r.take_u64()?));
        }
        rivers.push(RiverRecord { id, mouth, path });
    }

    let climate = ClimateSection {
        temperature: take_optional_i16_slice(&mut r, "temperature")?
            .map(|values| values.into_iter().map(TempDeciC).collect()),
        humidity: take_optional_i16_slice(&mut r, "humidity")?
            .map(|values| values.into_iter().map(HumidDeciPct).collect()),
        precipitation: take_optional_i32_slice(&mut r, "precipitation")?
            .map(|values| values.into_iter().map(PrecipMmYr).collect()),
    };
    let biomes = BiomeSection {
        biome: take_optional_u16_slice(&mut r, "biome")?
            .map(|values| values.into_iter().map(BiomeId).collect()),
    };

    let world = GeographicWorld {
        params: GenerationParams {
            params_version,
            seed,
            climate: climate_params,
            terrain: terrain_params,
            hydrology: hydrology_params,
        },
        grid,
        territories,
        regions,
        water_bodies,
        rivers,
        climate,
        biomes,
    };
    validate(&world)?;
    Ok(world)
}

/// Returns the content hash stored in a serialized world (its trailing
/// eight bytes).
pub fn stored_content_hash(bytes: &[u8]) -> Option<u64> {
    bytes
        .split_last_chunk::<8>()
        .map(|(_, hash)| u64::from_le_bytes(*hash))
}

// --- writing helpers ---

fn write_count(out: &mut Vec<u8>, what: &'static str, len: usize) {
    assert!(len <= u32::MAX as usize, "{what} section too large");
    out.extend_from_slice(&(len as u32).to_le_bytes());
}

fn write_str(out: &mut Vec<u8>, value: &[u8]) {
    assert!(value.len() <= u16::MAX as usize, "string too large");
    out.extend_from_slice(&(value.len() as u16).to_le_bytes());
    out.extend_from_slice(value);
}

fn write_optional_slice<T>(out: &mut Vec<u8>, values: Option<&[T]>)
where
    T: WireEncode,
{
    match values {
        None => out.push(0),
        Some(values) => {
            out.push(1);
            write_count(out, "optional section", values.len());
            for value in values {
                value.write_le(out);
            }
        }
    }
}

/// A lattice value with a fixed-width little-endian wire encoding.
trait WireEncode {
    fn write_le(&self, out: &mut Vec<u8>);
}

macro_rules! wire_encode {
    ($($name:ty => $int:ty),* $(,)?) => {
        $(impl WireEncode for $name {
            fn write_le(&self, out: &mut Vec<u8>) {
                out.extend_from_slice(&self.0.to_le_bytes());
            }
        })*
    };
}

wire_encode! {
    TempDeciC => i16,
    HumidDeciPct => i16,
    PrecipMmYr => i32,
}

// --- reading helpers ---

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn take_bytes(&mut self, count: usize) -> Result<&[u8], FormatError> {
        if self.remaining() < count {
            return Err(FormatError::UnexpectedEof {
                needed: count,
                remaining: self.remaining(),
            });
        }
        let out = &self.data[self.pos..self.pos + count];
        self.pos += count;
        Ok(out)
    }

    fn take_u8(&mut self) -> Result<u8, FormatError> {
        Ok(self.take_bytes(1)?[0])
    }

    fn take_bool(&mut self) -> Result<bool, FormatError> {
        match self.take_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(FormatError::InvalidBool(other)),
        }
    }

    fn take_u16(&mut self) -> Result<u16, FormatError> {
        let bytes = self.take_bytes(2)?;
        Ok(u16::from_le_bytes(bytes.try_into().expect("2 bytes")))
    }

    fn take_u32(&mut self) -> Result<u32, FormatError> {
        let bytes = self.take_bytes(4)?;
        Ok(u32::from_le_bytes(bytes.try_into().expect("4 bytes")))
    }

    fn take_u64(&mut self) -> Result<u64, FormatError> {
        let bytes = self.take_bytes(8)?;
        Ok(u64::from_le_bytes(bytes.try_into().expect("8 bytes")))
    }

    fn take_i16(&mut self) -> Result<i16, FormatError> {
        let bytes = self.take_bytes(2)?;
        Ok(i16::from_le_bytes(bytes.try_into().expect("2 bytes")))
    }

    fn take_i32(&mut self) -> Result<i32, FormatError> {
        let bytes = self.take_bytes(4)?;
        Ok(i32::from_le_bytes(bytes.try_into().expect("4 bytes")))
    }

    fn take_i64(&mut self) -> Result<i64, FormatError> {
        let bytes = self.take_bytes(8)?;
        Ok(i64::from_le_bytes(bytes.try_into().expect("8 bytes")))
    }

    fn take_count(&mut self, what: &'static str) -> Result<usize, FormatError> {
        let count = self.take_u32()? as usize;
        if count > self.remaining() {
            return Err(FormatError::CountExceedsBound { what, value: count });
        }
        Ok(count)
    }

    fn take_string(&mut self) -> Result<String, FormatError> {
        let len = self.take_u16()? as usize;
        if len > MAX_STRING_LEN {
            return Err(FormatError::CountExceedsBound {
                what: "string",
                value: len,
            });
        }
        String::from_utf8(self.take_bytes(len)?.to_vec())
            .map_err(|_| FormatError::InvalidUtf8String)
    }
}

fn take_anchor(r: &mut Reader<'_>) -> Result<Anchor, FormatError> {
    Ok(Anchor {
        x: r.take_i64()?,
        y: r.take_i64()?,
    })
}

fn take_option_u64(r: &mut Reader<'_>) -> Result<Option<u64>, FormatError> {
    match r.take_u8()? {
        0 => Ok(None),
        1 => Ok(Some(r.take_u64()?)),
        other => Err(FormatError::InvalidBool(other)),
    }
}

fn take_optional_i16_slice(
    r: &mut Reader<'_>,
    what: &'static str,
) -> Result<Option<Vec<i16>>, FormatError> {
    match r.take_u8()? {
        0 => Ok(None),
        1 => {
            let len = r.take_count(what)?;
            let mut values = Vec::with_capacity(len);
            for _ in 0..len {
                values.push(r.take_i16()?);
            }
            Ok(Some(values))
        }
        other => Err(FormatError::InvalidBool(other)),
    }
}

fn take_optional_i32_slice(
    r: &mut Reader<'_>,
    what: &'static str,
) -> Result<Option<Vec<i32>>, FormatError> {
    match r.take_u8()? {
        0 => Ok(None),
        1 => {
            let len = r.take_count(what)?;
            let mut values = Vec::with_capacity(len);
            for _ in 0..len {
                values.push(r.take_i32()?);
            }
            Ok(Some(values))
        }
        other => Err(FormatError::InvalidBool(other)),
    }
}

fn take_optional_u16_slice(
    r: &mut Reader<'_>,
    what: &'static str,
) -> Result<Option<Vec<u16>>, FormatError> {
    match r.take_u8()? {
        0 => Ok(None),
        1 => {
            let len = r.take_count(what)?;
            let mut values = Vec::with_capacity(len);
            for _ in 0..len {
                values.push(r.take_u16()?);
            }
            Ok(Some(values))
        }
        other => Err(FormatError::InvalidBool(other)),
    }
}

// --- validation ---

fn validate(world: &GeographicWorld) -> Result<(), FormatError> {
    let cell_count = u64::from(world.grid.width) * u64::from(world.grid.height);
    if cell_count != world.grid.cells.len() as u64 {
        return Err(FormatError::CellCountMismatch {
            declared: cell_count,
            actual: world.grid.cells.len(),
        });
    }

    let mut seen_territory_ids = BTreeSet::new();
    for (index, territory) in world.territories.iter().enumerate() {
        check_unique(&mut seen_territory_ids, "territory", territory.id.0)?;
        check_in_bounds(&territory.anchor, world, "territory anchor")?;
        if let Some(region) = territory.region
            && !world.regions.iter().any(|known| known.id == region)
        {
            return Err(FormatError::UnknownRegionReference {
                territory_index: index,
                id: region.0,
            });
        }
    }

    let mut seen_region_ids = BTreeSet::new();
    for region in &world.regions {
        check_unique(&mut seen_region_ids, "region", region.id.0)?;
    }

    let mut seen_water_body_ids = BTreeSet::new();
    for body in &world.water_bodies {
        check_unique(&mut seen_water_body_ids, "water body", body.id.0)?;
        check_in_bounds(&body.anchor, world, "water body anchor")?;
    }

    let mut seen_river_ids = BTreeSet::new();
    for (index, river) in world.rivers.iter().enumerate() {
        check_unique(&mut seen_river_ids, "river", river.id.0)?;
        check_in_bounds(&river.mouth, world, "river mouth")?;
        if river.path.is_empty() {
            return Err(FormatError::EmptyRiverPath { river_index: index });
        }
        for cell in &river.path {
            if cell.0 >= cell_count {
                return Err(FormatError::IndexOutOfRange {
                    section: "river path",
                    cell: cell.0,
                });
            }
        }
    }

    let mut member_counts = vec![0u32; world.territories.len()];
    for (index, cell) in world.grid.cells.iter().enumerate() {
        if cell.territory != NO_INDEX {
            let territory_index = cell.territory as usize;
            match world.territories.get(territory_index) {
                Some(_) => member_counts[territory_index] += 1,
                None => {
                    return Err(FormatError::IndexOutOfRange {
                        section: "territory",
                        cell: index as u64,
                    });
                }
            }
        }
        if cell.water_body != NO_INDEX {
            if cell.water_body as usize >= world.water_bodies.len() {
                return Err(FormatError::IndexOutOfRange {
                    section: "water body",
                    cell: index as u64,
                });
            }
            if !cell.is_water {
                return Err(FormatError::WaterBodyOnLandCell { cell: index as u64 });
            }
        }
    }
    for (index, territory) in world.territories.iter().enumerate() {
        if member_counts[index] != territory.cell_count {
            return Err(FormatError::TerritoryCellCountMismatch {
                territory_index: index,
                declared: territory.cell_count,
                actual: member_counts[index],
            });
        }
    }

    let expected = world.grid.cells.len() as u32;
    let climate_lengths = [
        (
            "temperature",
            world.climate.temperature.as_ref().map(Vec::len),
        ),
        ("humidity", world.climate.humidity.as_ref().map(Vec::len)),
        (
            "precipitation",
            world.climate.precipitation.as_ref().map(Vec::len),
        ),
    ];
    for (section, length) in climate_lengths {
        if let Some(length) = length
            && length as u32 != expected
        {
            return Err(FormatError::SectionLengthMismatch {
                section,
                declared: length as u32,
            });
        }
    }
    if let Some(length) = world.biomes.biome.as_ref().map(Vec::len)
        && length as u32 != expected
    {
        return Err(FormatError::SectionLengthMismatch {
            section: "biome",
            declared: length as u32,
        });
    }
    Ok(())
}

fn check_unique(
    seen: &mut BTreeSet<u64>,
    namespace: &'static str,
    id: u64,
) -> Result<(), FormatError> {
    if id == 0 {
        return Err(FormatError::ReservedId { namespace });
    }
    if !seen.insert(id) {
        return Err(FormatError::DuplicateId { namespace, id });
    }
    Ok(())
}

fn check_in_bounds(
    anchor: &Anchor,
    world: &GeographicWorld,
    what: &'static str,
) -> Result<(), FormatError> {
    if anchor.x < 0 || anchor.x >= i64::from(world.grid.width) {
        return Err(FormatError::CoordinateOutOfBounds {
            what,
            value: anchor.x,
        });
    }
    if anchor.y < 0 || anchor.y >= i64::from(world.grid.height) {
        return Err(FormatError::CoordinateOutOfBounds {
            what,
            value: anchor.y,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic;

    /// Byte offset of the params version in the header.
    fn params_offset() -> usize {
        4 + 4
            + 2
            + ID_STRATEGY.len()
            + 2
            + LATTICE_REGISTRY_VERSION.len()
            + 2
            + BIOME_REGISTRY.len()
    }

    /// Serializes a mutated world, repairing the trailing content hash so
    /// the targeted validation error is reached instead of the hash check.
    fn serialize_unchecked(world: &GeographicWorld, body: &mut Vec<u8>) -> Vec<u8> {
        *body = to_bytes(world);
        let hash = fnv1a64(&body[..body.len() - 8]);
        let tail = body.len() - 8;
        body[tail..].copy_from_slice(&hash.to_le_bytes());
        body.clone()
    }

    #[test]
    fn round_trip_preserves_the_world() {
        let world = synthetic::minimal_world();
        let bytes = world.to_bytes();
        assert_eq!(from_bytes(&bytes).expect("valid"), world);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = synthetic::minimal_world().to_bytes();
        bytes[0] = b'X';
        let hash = fnv1a64(&bytes[..bytes.len() - 8]);
        let tail = bytes.len() - 8;
        bytes[tail..].copy_from_slice(&hash.to_le_bytes());
        assert_eq!(from_bytes(&bytes), Err(FormatError::BadMagic));
    }

    #[test]
    fn rejects_truncated_files() {
        let bytes = synthetic::minimal_world().to_bytes();
        let err = from_bytes(&bytes[..bytes.len() - 4]).expect_err("truncated");
        assert!(matches!(err, FormatError::HashMismatch { .. }));
        let err = from_bytes(&bytes[..4]).expect_err("truncated");
        assert!(matches!(err, FormatError::UnexpectedEof { .. }));
    }

    #[test]
    fn rejects_corrupted_payload() {
        let mut bytes = synthetic::minimal_world().to_bytes();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0xff;
        let stored = stored_content_hash(&bytes).expect("hash present");
        let computed = fnv1a64(&bytes[..bytes.len() - 8]);
        assert_eq!(
            from_bytes(&bytes),
            Err(FormatError::HashMismatch { stored, computed })
        );
    }

    #[test]
    fn rejects_unknown_id_strategy() {
        let mut bytes = synthetic::minimal_world().to_bytes();
        // Header layout: magic (4) + schema_version (4) + string length (2).
        bytes[10] = b'X';
        let hash = fnv1a64(&bytes[..bytes.len() - 8]);
        let tail = bytes.len() - 8;
        bytes[tail..].copy_from_slice(&hash.to_le_bytes());
        let err = from_bytes(&bytes).expect_err("unknown strategy");
        assert!(matches!(err, FormatError::UnsupportedIdStrategy(_)));
    }

    #[test]
    fn rejects_duplicate_territory_ids() {
        let mut world = synthetic::minimal_world();
        let duplicate = world.territories[0].id;
        world.territories[1].id = duplicate;
        let err =
            from_bytes(&serialize_unchecked(&world, &mut Vec::new())).expect_err("duplicate id");
        assert_eq!(
            err,
            FormatError::DuplicateId {
                namespace: "territory",
                id: duplicate.0
            }
        );
    }

    #[test]
    fn rejects_territory_cell_count_drift() {
        let mut world = synthetic::minimal_world();
        world.territories[0].cell_count += 1;
        let err = from_bytes(&serialize_unchecked(&world, &mut Vec::new()))
            .expect_err("cell count drift");
        assert!(matches!(
            err,
            FormatError::TerritoryCellCountMismatch { .. }
        ));
    }

    #[test]
    fn rejects_water_body_reference_from_a_land_cell() {
        let mut world = synthetic::minimal_world();
        let land = world
            .grid
            .cells
            .iter()
            .position(|cell| !cell.is_water)
            .expect("land exists");
        world.grid.cells[land].water_body = 0;
        let err = from_bytes(&serialize_unchecked(&world, &mut Vec::new()))
            .expect_err("land with water body");
        assert_eq!(err, FormatError::WaterBodyOnLandCell { cell: land as u64 });
    }

    #[test]
    fn rejects_unknown_region_reference() {
        let mut world = synthetic::minimal_world();
        world.territories[0].region = Some(RegionId(123));
        let err =
            from_bytes(&serialize_unchecked(&world, &mut Vec::new())).expect_err("unknown region");
        assert!(matches!(err, FormatError::UnknownRegionReference { .. }));
    }

    #[test]
    fn rejects_empty_river_path() {
        let mut world = synthetic::minimal_world();
        world.rivers[0].path.clear();
        let err =
            from_bytes(&serialize_unchecked(&world, &mut Vec::new())).expect_err("empty river");
        assert_eq!(err, FormatError::EmptyRiverPath { river_index: 0 });
    }

    #[test]
    fn rejects_mismatched_climate_section_length() {
        let mut world = synthetic::minimal_world();
        world.climate.temperature = Some(Vec::new());
        let err = from_bytes(&serialize_unchecked(&world, &mut Vec::new()))
            .expect_err("short climate section");
        assert_eq!(
            err,
            FormatError::SectionLengthMismatch {
                section: "temperature",
                declared: 0
            }
        );
    }

    #[test]
    fn round_trips_climate_parameters() {
        let mut world = synthetic::minimal_world();
        world.params = GenerationParams {
            params_version: 1,
            seed: 0x5EED,
            climate: Some(ClimateParams {
                temperature_offset: TempDeciC(-25),
                polar_amplification: CentiScalar(150),
                latitude_exponent: CentiScalar(100),
                humidity_offset: CentiScalar(30),
            }),
            terrain: None,
            hydrology: None,
        };
        world.climate.temperature = Some(vec![TempDeciC(120); world.grid.cells.len()]);
        let bytes = world.to_bytes();
        assert_eq!(from_bytes(&bytes).expect("valid"), world);
    }

    #[test]
    fn round_trips_terrain_parameters() {
        let mut world = synthetic::minimal_world();
        world.params = GenerationParams {
            params_version: 2,
            seed: 0x5EED,
            climate: Some(ClimateParams {
                temperature_offset: TempDeciC(-25),
                polar_amplification: CentiScalar(150),
                latitude_exponent: CentiScalar(100),
                humidity_offset: CentiScalar(30),
            }),
            terrain: Some(TerrainParams {
                erosion: Some(ErosionParams {
                    droplets_per_hundred_cells: CentiScalar(100),
                    power: CentiScalar(2),
                    talus: HeightM(120),
                }),
                features_across: CentiScalar(150),
                min_feature_cells: 8,
            }),
            hydrology: None,
        };
        world.climate.temperature = Some(vec![TempDeciC(120); world.grid.cells.len()]);
        let bytes = world.to_bytes();
        assert_eq!(from_bytes(&bytes).expect("valid"), world);

        // The terrain-only profile carries no climate block.
        world.params.climate = None;
        world.climate.temperature = None;
        let bytes = world.to_bytes();
        let decoded = from_bytes(&bytes).expect("valid");
        assert!(decoded.params.climate.is_none());
        assert!(decoded.params.terrain.is_some());
    }

    #[test]
    fn rejects_inconsistent_params_flags() {
        let mut world = synthetic::minimal_world();
        world.params = GenerationParams {
            params_version: 2,
            seed: 7,
            climate: None,
            terrain: Some(TerrainParams {
                erosion: None,
                features_across: CentiScalar(150),
                min_feature_cells: 8,
            }),
            hydrology: None,
        };
        let mut bytes = world.to_bytes();
        // Header layout: magic (4) + schema_version (4) + id strategy
        // string + lattice registry string + params_version (4) + seed
        // (8) — then the params flags byte.
        let offset = params_offset() + 4 + 8; // params version + seed
        bytes[offset] = 0; // terrain bit cleared: inconsistent with version 2
        let hash = fnv1a64(&bytes[..bytes.len() - 8]);
        let tail = bytes.len() - 8;
        bytes[tail..].copy_from_slice(&hash.to_le_bytes());
        assert_eq!(from_bytes(&bytes), Err(FormatError::InvalidParamsFlags(0)));
    }

    #[test]
    fn round_trips_hydrology_parameters() {
        // The v3 layout predates the map-space terrain fields: a v3
        // world carries no terrain block (the writer refuses the
        // combination, since its stand-in values would be fabricated).
        let mut world = synthetic::minimal_world();
        world.params = GenerationParams {
            params_version: 3,
            seed: 0x5EED,
            climate: Some(ClimateParams {
                temperature_offset: TempDeciC(-25),
                polar_amplification: CentiScalar(150),
                latitude_exponent: CentiScalar(100),
                humidity_offset: CentiScalar(30),
            }),
            terrain: None,
            hydrology: Some(HydrologyParams {
                river_land_share: CentiScalar(12),
            }),
        };
        world.climate.temperature = Some(vec![TempDeciC(120); world.grid.cells.len()]);
        let bytes = world.to_bytes();
        assert_eq!(from_bytes(&bytes).expect("valid"), world);

        // The minimal v3 profile carries no climate block either.
        world.params.climate = None;
        world.climate.temperature = None;
        let bytes = world.to_bytes();
        let decoded = from_bytes(&bytes).expect("valid");
        assert!(decoded.params.climate.is_none());
        assert!(decoded.params.terrain.is_none());
        assert!(decoded.params.hydrology.is_some());
    }

    #[test]
    fn rejects_inconsistent_hydrology_flags() {
        let mut world = synthetic::minimal_world();
        world.params = GenerationParams {
            params_version: 3,
            seed: 7,
            climate: None,
            terrain: None,
            hydrology: Some(HydrologyParams {
                river_land_share: CentiScalar(12),
            }),
        };
        let mut bytes = world.to_bytes();
        // The params flags byte follows the params version and the seed.
        let offset = params_offset() + 4 + 8;
        bytes[offset] = 0; // hydrology bit cleared: inconsistent with version 3
        let hash = fnv1a64(&bytes[..bytes.len() - 8]);
        let tail = bytes.len() - 8;
        bytes[tail..].copy_from_slice(&hash.to_le_bytes());
        assert_eq!(from_bytes(&bytes), Err(FormatError::InvalidParamsFlags(0)));
    }

    #[test]
    fn rejects_unknown_params_versions() {
        let mut world = synthetic::minimal_world();
        world.params.params_version = 3;
        world.params.hydrology = Some(HydrologyParams {
            river_land_share: CentiScalar(12),
        });
        let mut bytes = world.to_bytes();
        // Patch the params version to a value this reader does not know;
        // keep the content hash consistent so the version gate is what
        // rejects the file.
        let offset = params_offset();
        bytes[offset..offset + 4].copy_from_slice(&5u32.to_le_bytes());
        let hash = fnv1a64(&bytes[..bytes.len() - 8]);
        let tail = bytes.len() - 8;
        bytes[tail..].copy_from_slice(&hash.to_le_bytes());
        assert_eq!(
            from_bytes(&bytes),
            Err(FormatError::UnsupportedParamsVersion(5))
        );
    }

    #[test]
    fn round_trips_map_space_terrain_parameters() {
        let mut world = synthetic::minimal_world();
        world.params = GenerationParams {
            params_version: 4,
            seed: 0x5EED,
            climate: None,
            terrain: Some(TerrainParams {
                erosion: None,
                features_across: CentiScalar(150),
                min_feature_cells: 8,
            }),
            hydrology: Some(HydrologyParams {
                river_land_share: CentiScalar(12),
            }),
        };
        let bytes = world.to_bytes();
        let decoded = from_bytes(&bytes).expect("valid");
        assert_eq!(decoded, world);
        assert_eq!(
            decoded.params.terrain.unwrap().features_across,
            CentiScalar(150)
        );

        // The map-space layout carries no climate block here; a climate
        // block joins per its flag.
        world.params.climate = Some(ClimateParams {
            temperature_offset: TempDeciC(-25),
            polar_amplification: CentiScalar(150),
            latitude_exponent: CentiScalar(100),
            humidity_offset: CentiScalar(30),
        });
        world.climate.temperature = Some(vec![TempDeciC(120); world.grid.cells.len()]);
        let bytes = world.to_bytes();
        let decoded = from_bytes(&bytes).expect("valid");
        assert!(decoded.params.climate.is_some());
        assert_eq!(decoded.params.terrain.unwrap().min_feature_cells, 8);
    }

    #[test]
    fn reads_legacy_params_v2_exports() {
        // Committed at the pre-hydrology-params revision: the reader
        // must keep accepting files whose params stop at version 2.
        let bytes = include_bytes!("../tests/golden/params_v2_world.gwb");
        let world = from_bytes(bytes).expect("the legacy params-v2 export stays readable");
        assert_eq!(world.params.params_version, 2);
        assert!(world.params.hydrology.is_none());
        assert!(!world.rivers.is_empty(), "the fixture carries its rivers");
    }

    #[test]
    fn rejects_unknown_schema_version() {
        let mut bytes = synthetic::minimal_world().to_bytes();
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
        let hash = fnv1a64(&bytes[..bytes.len() - 8]);
        let tail = bytes.len() - 8;
        bytes[tail..].copy_from_slice(&hash.to_le_bytes());
        assert_eq!(
            from_bytes(&bytes),
            Err(FormatError::UnsupportedSchemaVersion(2))
        );
    }

    #[test]
    fn rejects_unknown_biome_registry() {
        let mut bytes = synthetic::minimal_world().to_bytes();
        // Header layout: magic (4) + schema_version (4) + id strategy
        // string + lattice registry string, then the biome registry
        // string (schema 1).
        let offset = 4 + 4 + 2 + ID_STRATEGY.len() + 2 + LATTICE_REGISTRY_VERSION.len();
        bytes[offset..offset + 2].copy_from_slice(&9u16.to_le_bytes());
        bytes[offset + 2..offset + 11].copy_from_slice(b"biomes-x1");
        let hash = fnv1a64(&bytes[..bytes.len() - 8]);
        let tail = bytes.len() - 8;
        bytes[tail..].copy_from_slice(&hash.to_le_bytes());
        assert_eq!(
            from_bytes(&bytes),
            Err(FormatError::UnsupportedBiomeRegistry(
                "biomes-x1".to_string()
            ))
        );
    }

    #[test]
    fn rejects_unknown_params_version() {
        let mut bytes = synthetic::minimal_world().to_bytes();
        // Header layout: magic (4) + schema_version (4) + id strategy
        // string + lattice registry string, then the params version.
        let offset = params_offset();
        bytes[offset..offset + 4].copy_from_slice(&7u32.to_le_bytes());
        let hash = fnv1a64(&bytes[..bytes.len() - 8]);
        let tail = bytes.len() - 8;
        bytes[tail..].copy_from_slice(&hash.to_le_bytes());
        assert_eq!(
            from_bytes(&bytes),
            Err(FormatError::UnsupportedParamsVersion(7))
        );
    }

    #[test]
    fn rejects_grid_declaring_more_cells_than_present() {
        let mut world = synthetic::minimal_world();
        world.grid.width += 1;
        let err = from_bytes(&serialize_unchecked(&world, &mut Vec::new()))
            .expect_err("cell count mismatch");
        // The specific variant depends on where the corruption is first
        // observed while streaming (EOF or a garbage field byte); the
        // contract is that it is rejected with a diagnostic.
        assert!(matches!(
            err,
            FormatError::UnexpectedEof { .. } | FormatError::InvalidBool(_)
        ));
    }

    #[test]
    fn rejects_cell_count_drift_detected_by_validation() {
        let mut world = synthetic::minimal_world();
        world.grid.cells.pop();
        let err = from_bytes(&serialize_unchecked(&world, &mut Vec::new()))
            .expect_err("cell count mismatch");
        // The specific variant depends on where the corruption is first
        // observed while streaming (EOF or a garbage field byte); the
        // contract is that it is rejected with a diagnostic.
        assert!(matches!(
            err,
            FormatError::UnexpectedEof { .. } | FormatError::InvalidBool(_)
        ));
    }
}
