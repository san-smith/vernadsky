//! The versioned biome registry.
//!
//! The registry is the owner of biome identity: identifier ↔ name ↔
//! category ↔ canonical debug color. It is data, not an enumeration —
//! consumers must look entries up, never match over the list.
//!
//! # Evolution policy (`biomes-v1`)
//!
//! - **Add** a biome: append an entry with a fresh identifier (one that
//!   has never been used in any registry version).
//! - **Deprecate** a biome (removed, or split into finer ones): stop
//!   producing it in the classification, keep the entry with
//!   [`BiomeEntry::deprecated`] set. The identifier is never reassigned.
//! - **Redefine** the meaning of an existing identifier: forbidden
//!   inside a registry version; bump [`BIOME_REGISTRY_VERSION`] instead
//!   (the policy exists so this never becomes necessary).
//!
//! Unknown and deprecated identifiers are valid in exports; consumers
//! resolve them through [`entry`] and fall back gracefully.

use vernadsky_core::BiomeId;

/// Version of the biome registry written by this crate.
pub const BIOME_REGISTRY_VERSION: &str = "biomes-v1";

/// Canonical debug color of a biome.
pub type Rgb = [u8; 3];

/// Coarse category of a biome: which classification tier owns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BiomeCategory {
    /// Water bodies, by temperature and depth.
    Water,
    /// Highlands, by height and temperature.
    Mountain,
    /// Climate belt, by temperature and humidity.
    Climate,
}

/// One registry entry: the full stable identity of a biome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BiomeEntry {
    /// Stable identifier of the biome; `0` is reserved by the schema.
    pub id: BiomeId,
    /// Stable name within this registry version.
    pub name: &'static str,
    /// Coarse category the classification tier assigns from.
    pub category: BiomeCategory,
    /// Canonical debug color (ported from the mapgen prototype).
    pub color: Rgb,
    /// Whether the classification still produces this biome. Deprecated
    /// entries stay in the registry so older exports remain
    /// interpretable; their identifiers are never reassigned.
    pub deprecated: bool,
}

/// The registry, ordered by identifier.
///
/// Initial set: sixteen biomes ported from the mapgen prototype. This
/// table is the single place where the biome set is defined.
pub const REGISTRY: &[BiomeEntry] = &[
    BiomeEntry {
        id: BiomeId(1),
        name: "DeepOcean",
        category: BiomeCategory::Water,
        color: [0, 30, 80],
        deprecated: false,
    },
    BiomeEntry {
        id: BiomeId(2),
        name: "Ocean",
        category: BiomeCategory::Water,
        color: [0, 70, 140],
        deprecated: false,
    },
    BiomeEntry {
        id: BiomeId(3),
        name: "IcyOcean",
        category: BiomeCategory::Water,
        color: [120, 180, 220],
        deprecated: false,
    },
    BiomeEntry {
        id: BiomeId(4),
        name: "FrozenOcean",
        category: BiomeCategory::Water,
        color: [180, 200, 220],
        deprecated: false,
    },
    BiomeEntry {
        id: BiomeId(5),
        name: "Ice",
        category: BiomeCategory::Climate,
        color: [220, 230, 255],
        deprecated: false,
    },
    BiomeEntry {
        id: BiomeId(6),
        name: "Tundra",
        category: BiomeCategory::Climate,
        color: [200, 210, 190],
        deprecated: false,
    },
    BiomeEntry {
        id: BiomeId(7),
        name: "Taiga",
        category: BiomeCategory::Climate,
        color: [80, 120, 80],
        deprecated: false,
    },
    BiomeEntry {
        id: BiomeId(8),
        name: "TemperateForest",
        category: BiomeCategory::Climate,
        color: [60, 140, 60],
        deprecated: false,
    },
    BiomeEntry {
        id: BiomeId(9),
        name: "TropicalRainforest",
        category: BiomeCategory::Climate,
        color: [30, 100, 30],
        deprecated: false,
    },
    BiomeEntry {
        id: BiomeId(10),
        name: "Grassland",
        category: BiomeCategory::Climate,
        color: [140, 190, 100],
        deprecated: false,
    },
    BiomeEntry {
        id: BiomeId(11),
        name: "Shrubland",
        category: BiomeCategory::Climate,
        color: [160, 150, 100],
        deprecated: false,
    },
    BiomeEntry {
        id: BiomeId(12),
        name: "Savanna",
        category: BiomeCategory::Climate,
        color: [190, 170, 100],
        deprecated: false,
    },
    BiomeEntry {
        id: BiomeId(13),
        name: "Desert",
        category: BiomeCategory::Climate,
        color: [220, 200, 150],
        deprecated: false,
    },
    BiomeEntry {
        id: BiomeId(14),
        name: "Swamp",
        category: BiomeCategory::Climate,
        color: [70, 110, 60],
        deprecated: false,
    },
    BiomeEntry {
        id: BiomeId(15),
        name: "RockyMountain",
        category: BiomeCategory::Mountain,
        color: [140, 140, 140],
        deprecated: false,
    },
    BiomeEntry {
        id: BiomeId(16),
        name: "GlacialMountain",
        category: BiomeCategory::Mountain,
        color: [200, 220, 240],
        deprecated: false,
    },
];

/// Looks up a registry entry by identifier; `None` for identifiers this
/// registry version does not know (valid in exports, rendered with a
/// fallback by consumers).
pub fn entry(id: BiomeId) -> Option<&'static BiomeEntry> {
    REGISTRY.iter().find(|candidate| candidate.id == id)
}

/// Looks up an identifier by its stable name.
pub fn by_name(name: &str) -> Option<BiomeId> {
    REGISTRY
        .iter()
        .find(|candidate| candidate.name == name)
        .map(|candidate| candidate.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_invariants_hold() {
        assert_eq!(BIOME_REGISTRY_VERSION, "biomes-v1");
        let mut ids = Vec::new();
        let mut names = Vec::new();
        for (position, entry) in REGISTRY.iter().enumerate() {
            assert_ne!(entry.id.0, 0, "identifier 0 is reserved by the schema");
            assert_eq!(
                REGISTRY.iter().position(|probe| probe.id == entry.id),
                Some(position),
                "identifiers must be unique and ascending"
            );
            ids.push(entry.id.0);
            names.push(entry.name);
        }
        ids.sort_unstable();
        ids.dedup();
        names.sort_unstable();
        names.dedup();
        assert_eq!(ids.len(), REGISTRY.len(), "identifiers must be unique");
        assert_eq!(names.len(), REGISTRY.len(), "names must be unique");
    }

    #[test]
    fn lookup_by_id_and_name_agrees() {
        let tundra = entry(BiomeId(6)).expect("Tundra is registered");
        assert_eq!(tundra.name, "Tundra");
        assert_eq!(by_name("Tundra"), Some(BiomeId(6)));
        assert_eq!(by_name("NonBiome"), None);
        assert_eq!(entry(BiomeId(0)), None, "identifier 0 is reserved");
        assert_eq!(
            entry(BiomeId(9999)),
            None,
            "unknown ids are valid but unresolved"
        );
    }

    #[test]
    fn initial_set_has_no_deprecated_entries() {
        assert!(
            REGISTRY.iter().all(|entry| !entry.deprecated),
            "the initial port set is fully produced; deprecations appear only through later evolution"
        );
    }
}
