//! Stable identifier spaces of `GeographicWorld`.
//!
//! Every identified entity kind has its own newtype, so cross-kind
//! collisions are impossible at the type level. Identifiers are part of
//! the public contract: consumers correlate data across exports by them.
//!
//! The raw value `0` is reserved and never assigned; it means "unset".
//! Identifier values are derived by the strategy declared in the export
//! header (see [`crate::idgen`]) and must not be synthesized by hand.
//!
//! Debug and [`Display`] render a kind-prefixed hexadecimal value
//! (`t:1a2b`, `r:…`, `w:…`, `v:…`, `c:…`, `b:…`) for readable logs.

use std::fmt;

macro_rules! id_newtype {
    ($(#[$doc:meta])* $name:ident($storage:ty), $prefix:literal) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub $storage);

        impl $name {
            /// The reserved "unset" value; never assigned to an entity.
            pub const RESERVED: $storage = 0;

            /// Whether this identifier is the reserved unset value.
            pub fn is_reserved(self) -> bool {
                self.0 == Self::RESERVED
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, ":{:x}"), self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Debug::fmt(self, f)
            }
        }
    };
}

id_newtype! {
    /// A cell of the generation grid: the row-major index `y * width + x`.
    ///
    /// Cell identity *is* its grid coordinate; changing the resolution or
    /// the index order is a breaking schema change.
    CellId(u64), "c"
}

id_newtype! {
    /// A territory: the neutral geographic partition unit (land or water).
    ///
    /// Game concepts such as Province, State, or SeaZone are consumer-side
    /// constructs built from territories and cells.
    TerritoryId(u64), "t"
}

id_newtype! {
    /// A region: a macro-group of territories (continent or sea basin).
    ///
    /// By policy the value equals the smallest member territory
    /// identifier, which keeps it stable while that member exists.
    RegionId(u64), "r"
}

id_newtype! {
    /// A classified water body (ocean, sea, or lake).
    WaterBodyId(u64), "w"
}

id_newtype! {
    /// A river: an ordered path from its source towards its mouth.
    RiverId(u64), "v"
}

id_newtype! {
    /// A biome, assigned by the versioned biome registry.
    BiomeId(u16), "b"
}

impl CellId {
    /// Row-major index of the cell `(x, y)` on a grid of the given width.
    pub fn from_xy(x: u32, y: u32, width: u32) -> Self {
        Self(u64::from(y) * u64::from(width) + u64::from(x))
    }

    /// Inverse of [`CellId::from_xy`]: the cell coordinates on a grid of
    /// the given width.
    pub fn to_xy(self, width: u32) -> (u32, u32) {
        let w = u64::from(width);
        ((self.0 % w) as u32, (self.0 / w) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_index_round_trip() {
        let (width, height) = (48u32, 32u32);
        for y in 0..height {
            for x in 0..width {
                let id = CellId::from_xy(x, y, width);
                assert_eq!(id.to_xy(width), (x, y));
            }
        }
        assert_eq!(CellId::from_xy(5, 3, 48).0, 3 * 48 + 5);
    }

    #[test]
    fn reserved_value_is_recognized() {
        assert_eq!(TerritoryId::RESERVED, 0);
        assert!(TerritoryId(0).is_reserved());
        assert!(!TerritoryId(1).is_reserved());
    }

    #[test]
    fn debug_uses_kind_prefixes() {
        assert_eq!(format!("{:?}", TerritoryId(0x1a2b)), "t:1a2b");
        assert_eq!(format!("{:?}", RiverId(0xff)), "v:ff");
        assert_eq!(format!("{}", BiomeId(9)), "b:9");
    }
}
