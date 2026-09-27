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
//! ```

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
/// Defaults describe the port profile: erosion on, one percent of the
/// map area as water drops, erosion power `0.02`, and a talus angle of
/// `120 m`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TerrainConfig {
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
        Ok(TerrainParams { erosion })
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
