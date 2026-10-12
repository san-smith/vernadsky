//! TOML configuration of the terrain stage.
//!
//! The config carries human-facing float fields; every accepted value is
//! validated against its domain and quantized onto the canonical
//! parameter lattices of [`TerrainParams`].
//!
//! A configuration document looks like this (all fields optional,
//! unknown fields are rejected):
//!
//! ```toml
//! erosion_enabled = true               # thermal + hydraulic passes
//! droplets_per_hundred_cells = 100.0   # drop count per 100 cells (1 %)
//! power = 0.02                         # hydraulic intensity [0.005, 0.2]
//! talus_m = 120.0                      # thermal talus angle, meters [10, 500]
//! features_across = 2.0                # base features around the map [0.5, 8]
//! min_feature_cells = 8                # detail floor, cells [1, 256]
//! land_ratio = 0.3                     # targeted land share [0.05, 0.95]
//! island_density = 0.5                 # island lift over the lowlands [0, 3]
//! land_hypsometry = 1.0                # land elevation curve exponent [1, 4]
//! ```
//!
//! The recipe surface — `land_ratio`, `island_density`, and
//! `features_across` — is what a consumer varies to shape a world type:
//! a supercontinent is a high land share with few base features and no
//! islands, an archipelago a low land share with many features and a
//! strong island layer.

use std::fmt;

use serde::Deserialize;
use vernadsky_core::quant::{CentiScalar, HeightM, QuantError};
use vernadsky_core::schema::{ErosionParams, TerrainParams};

/// Errors of TOML parameter loading and validation.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamsError {
    /// The TOML document is malformed or contains unknown fields.
    Parse(String),
    /// A field holds a non-finite value.
    NonFinite {
        /// Name of the offending field.
        field: &'static str,
    },
    /// A field is outside its conventional domain.
    OutOfDomain {
        /// Name of the offending field.
        field: &'static str,
        /// The offending value.
        value: f64,
    },
    /// A field could not be represented on its parameter lattice.
    Quantization {
        /// Name of the offending field.
        field: &'static str,
        /// The quantization failure.
        source: QuantError,
    },
}

impl fmt::Display for ParamsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParamsError::Parse(message) => write!(f, "invalid terrain config: {message}"),
            ParamsError::NonFinite { field } => {
                write!(f, "terrain config field {field} must be finite")
            }
            ParamsError::OutOfDomain { field, value } => write!(
                f,
                "terrain config field {field} = {value} is outside its documented domain"
            ),
            ParamsError::Quantization { field, source } => {
                write!(f, "terrain config field {field}: {source}")
            }
        }
    }
}

impl std::error::Error for ParamsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ParamsError::Quantization { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// TOML configuration of the terrain stage.
///
/// Defaults describe the calibrated profile: one and a half base
/// features across the map (a dominant continent at every resolution),
/// an eight-cell detail floor, thirty percent of the map as land with
/// the mid island layer, erosion on, one percent of the map area as
/// water drops, erosion power `0.02`, and a talus angle of `120 m`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TerrainConfig {
    /// Base features spanning the map circumference: the map-space
    /// normalization keeps this count independent of the grid
    /// resolution.
    #[serde(default = "default_features_across")]
    pub features_across: f64,
    /// The finest feature wavelength the octaves may produce, in cells:
    /// the detail floor against coastline speckle.
    #[serde(default = "default_min_feature_cells")]
    pub min_feature_cells: u32,
    /// The targeted share of land cells: the sea level is placed so that
    /// this share of the map stands above it.
    #[serde(default = "default_land_ratio")]
    pub land_ratio: f64,
    /// Strength of the island lift over the lowlands: `0` keeps the
    /// oceans free of the island layer. Up to `1` the layer is subtle
    /// (the default `0.5` is the port value); around `2` it raises
    /// island chains out of the deep ocean, and toward `3` it begins to
    /// break the continents themselves.
    #[serde(default = "default_island_density")]
    pub island_density: f64,
    /// The exponent of the land elevation curve: `1` spreads the land
    /// evenly up to 6000 m, larger values crowd it into lowlands under
    /// a few high ranges (Earth's hypsometric curve sits near `3`).
    #[serde(default = "default_land_hypsometry")]
    pub land_hypsometry: f64,
    /// Whether the thermal and hydraulic erosion passes run.
    #[serde(default = "default_erosion_enabled")]
    pub erosion_enabled: bool,
    /// Hydraulic droplets per hundred cells (the port value `100` is
    /// one percent of the map area).
    #[serde(default = "default_droplets")]
    pub droplets_per_hundred_cells: f64,
    /// Hydraulic erosion intensity per droplet step.
    #[serde(default = "default_power")]
    pub power: f64,
    /// Thermal talus angle, in meters.
    #[serde(default = "default_talus")]
    pub talus_m: f64,
}

fn default_features_across() -> f64 {
    1.5
}

fn default_min_feature_cells() -> u32 {
    8
}

fn default_land_ratio() -> f64 {
    0.3
}

fn default_island_density() -> f64 {
    0.5
}

fn default_land_hypsometry() -> f64 {
    1.0
}

fn default_erosion_enabled() -> bool {
    true
}

fn default_droplets() -> f64 {
    100.0
}

fn default_power() -> f64 {
    0.02
}

fn default_talus() -> f64 {
    120.0
}

impl Default for TerrainConfig {
    fn default() -> Self {
        Self {
            features_across: default_features_across(),
            min_feature_cells: default_min_feature_cells(),
            land_ratio: default_land_ratio(),
            island_density: default_island_density(),
            land_hypsometry: default_land_hypsometry(),
            erosion_enabled: default_erosion_enabled(),
            droplets_per_hundred_cells: default_droplets(),
            power: default_power(),
            talus_m: default_talus(),
        }
    }
}

impl TerrainConfig {
    /// Validates the configuration and converts it into the canonical
    /// parameter lattices.
    pub fn to_params(&self) -> Result<TerrainParams, ParamsError> {
        let erosion = if self.erosion_enabled {
            Some(ErosionParams {
                droplets_per_hundred_cells: centi(
                    "droplets_per_hundred_cells",
                    self.droplets_per_hundred_cells,
                    1.0..=1000.0,
                )?,
                power: centi("power", self.power, 0.005..=0.2)?,
                talus: HeightM::try_from_value(check("talus_m", self.talus_m, 10.0..=500.0)?)
                    .map_err(|source| ParamsError::Quantization {
                        field: "talus_m",
                        source,
                    })?,
            })
        } else {
            None
        };
        if !(1..=256).contains(&self.min_feature_cells) {
            return Err(ParamsError::OutOfDomain {
                field: "min_feature_cells",
                value: f64::from(self.min_feature_cells),
            });
        }
        Ok(TerrainParams {
            erosion,
            features_across: centi("features_across", self.features_across, 0.5..=8.0)?,
            min_feature_cells: self.min_feature_cells,
            land_ratio: centi("land_ratio", self.land_ratio, 0.05..=0.95)?,
            island_density: centi("island_density", self.island_density, 0.0..=3.0)?,
            land_hypsometry: centi("land_hypsometry", self.land_hypsometry, 1.0..=4.0)?,
        })
    }
}

/// Loads and validates terrain parameters from a TOML document.
pub fn from_toml_str(source: &str) -> Result<TerrainParams, ParamsError> {
    let config: TerrainConfig =
        toml::from_str(source).map_err(|error| ParamsError::Parse(error.to_string()))?;
    config.to_params()
}

fn centi(
    field: &'static str,
    value: f64,
    domain: std::ops::RangeInclusive<f64>,
) -> Result<CentiScalar, ParamsError> {
    let value = check(field, value, domain)?;
    CentiScalar::try_from_value(value).map_err(|source| ParamsError::Quantization { field, source })
}

fn check(
    field: &'static str,
    value: f64,
    domain: std::ops::RangeInclusive<f64>,
) -> Result<f64, ParamsError> {
    if !value.is_finite() {
        return Err(ParamsError::NonFinite { field });
    }
    if !domain.contains(&value) {
        return Err(ParamsError::OutOfDomain { field, value });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_canonical() {
        let params = TerrainConfig::default()
            .to_params()
            .expect("valid defaults");
        let erosion = params.erosion.expect("erosion on by default");
        assert_eq!(erosion.droplets_per_hundred_cells, CentiScalar(10_000));
        assert_eq!(erosion.power, CentiScalar(2));
        assert_eq!(erosion.talus, HeightM(120));
        assert_eq!(params.features_across, CentiScalar(150), "1.5 features");
        assert_eq!(params.min_feature_cells, 8);
        assert_eq!(params.land_ratio, vernadsky_core::LEGACY_LAND_RATIO);
        assert_eq!(params.island_density, vernadsky_core::LEGACY_ISLAND_DENSITY);
        assert_eq!(
            params.land_hypsometry,
            vernadsky_core::LEGACY_LAND_HYPSOMETRY
        );
    }

    #[test]
    fn parses_and_bounds_the_recipe_surface() {
        let params = from_toml_str(
            "land_ratio = 0.92
island_density = 0.0
features_across = 0.7
",
        )
        .expect("valid document");
        assert_eq!(params.land_ratio, CentiScalar(92));
        assert_eq!(params.island_density, CentiScalar(0));
        assert_eq!(params.features_across, CentiScalar(70));
        assert!(matches!(
            from_toml_str("land_ratio = 0.99\n"),
            Err(ParamsError::OutOfDomain {
                field: "land_ratio",
                ..
            })
        ));
        assert!(matches!(
            from_toml_str("island_density = 3.5\n"),
            Err(ParamsError::OutOfDomain {
                field: "island_density",
                ..
            })
        ));
        assert_eq!(
            from_toml_str("land_hypsometry = 3.0\n")
                .expect("valid document")
                .land_hypsometry,
            CentiScalar(300)
        );
        assert!(matches!(
            from_toml_str("land_hypsometry = 0.5\n"),
            Err(ParamsError::OutOfDomain {
                field: "land_hypsometry",
                ..
            })
        ));
    }

    #[test]
    fn rejects_out_of_domain_map_space_fields() {
        assert!(matches!(
            TerrainConfig {
                features_across: 0.1,
                ..TerrainConfig::default()
            }
            .to_params(),
            Err(ParamsError::OutOfDomain {
                field: "features_across",
                ..
            })
        ));
        assert!(matches!(
            from_toml_str("min_feature_cells = 0\n"),
            Err(ParamsError::OutOfDomain {
                field: "min_feature_cells",
                ..
            })
        ));
    }

    #[test]
    fn erosion_can_be_disabled() {
        let params = from_toml_str("erosion_enabled = false\n").expect("valid document");
        assert!(params.erosion.is_none());
    }

    #[test]
    fn parses_a_full_document() {
        let params = from_toml_str(
            "erosion_enabled = true\ndroplets_per_hundred_cells = 50.0\npower = 0.05\ntalus_m = 200.0\n",
        )
        .expect("valid document");
        let erosion = params.erosion.expect("erosion on");
        assert_eq!(erosion.droplets_per_hundred_cells, CentiScalar(5_000));
        assert_eq!(erosion.power, CentiScalar(5));
        assert_eq!(erosion.talus, HeightM(200));
    }

    #[test]
    fn rejects_unknown_fields_and_domains() {
        assert!(matches!(
            from_toml_str("unknown_field = 1\n"),
            Err(ParamsError::Parse(_))
        ));
        assert!(matches!(
            from_toml_str("power = 5.0\n"),
            Err(ParamsError::OutOfDomain { field: "power", .. })
        ));
        assert!(matches!(
            from_toml_str("talus_m = nan\n"),
            Err(ParamsError::NonFinite { field: "talus_m" })
        ));
    }
}
