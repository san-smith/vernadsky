//! TOML configuration of the hydrology stage.
//!
//! The config carries human-facing float fields; every accepted value is
//! validated against its conventional domain and quantized onto the
//! integer parameter lattices of [`HydrologyParams`], so what reaches the
//! export and the stage is canonical and hashable.
//!
//! A configuration document looks like this (all fields optional,
//! unknown fields are rejected):
//!
//! ```toml
//! river_land_share_percent = 0.12  # drainage threshold as a share of
//!                                  # the land cells, percent, [0.01, 10]
//! ```

use std::fmt;

use serde::Deserialize;
use vernadsky_core::quant::{CentiScalar, QuantError};
use vernadsky_core::schema::HydrologyParams;

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
        /// The conventional domain, human-readable.
        domain: &'static str,
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
            ParamsError::Parse(message) => write!(f, "invalid hydrology config: {message}"),
            ParamsError::NonFinite { field } => {
                write!(f, "hydrology config field {field} must be finite")
            }
            ParamsError::OutOfDomain {
                field,
                value,
                domain,
            } => write!(
                f,
                "hydrology config field {field} = {value} is outside its domain ({domain})"
            ),
            ParamsError::Quantization { field, source } => {
                write!(f, "hydrology config field {field}: {source}")
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

/// Tuning of the hydrology stage: the resolution-independent river
/// threshold share. Defaults describe the calibrated `0.12%` share —
/// the value that keeps the visually validated river network of the
/// `200×100` reference grid across resolutions.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HydrologyConfig {
    /// The drainage threshold as a share of the land cells, in percent:
    /// a land cell carries a river when its accumulated flow reaches
    /// the maximum of the algorithm's absolute floor and this share of
    /// the world's land cells.
    #[serde(default = "default_river_land_share_percent")]
    pub river_land_share_percent: f64,
}

fn default_river_land_share_percent() -> f64 {
    0.12
}

impl Default for HydrologyConfig {
    fn default() -> Self {
        Self {
            river_land_share_percent: default_river_land_share_percent(),
        }
    }
}

impl HydrologyConfig {
    /// Validates the configuration and converts it into the canonical
    /// parameter lattices.
    pub fn to_params(&self) -> Result<HydrologyParams, ParamsError> {
        Ok(HydrologyParams {
            river_land_share: CentiScalar::try_from_value(check(
                "river_land_share_percent",
                self.river_land_share_percent,
                0.01..=10.0,
            )?)
            .map_err(|source| ParamsError::Quantization {
                field: "river_land_share_percent",
                source,
            })?,
        })
    }
}

/// Loads and validates hydrology parameters from a TOML document.
pub fn from_toml_str(source: &str) -> Result<HydrologyParams, ParamsError> {
    let config: HydrologyConfig =
        toml::from_str(source).map_err(|error| ParamsError::Parse(error.to_string()))?;
    config.to_params()
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
        return Err(ParamsError::OutOfDomain {
            field,
            value,
            domain: "documented in `HydrologyConfig`",
        });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_core_default() {
        assert_eq!(
            HydrologyConfig::default().to_params().unwrap(),
            HydrologyParams::default()
        );
    }

    #[test]
    fn quantizes_the_share_onto_the_lattice() {
        let params = HydrologyConfig {
            river_land_share_percent: 0.125,
        }
        .to_params()
        .unwrap();
        assert_eq!(
            params.river_land_share.0, 12,
            "0.125% → 12.5 steps → ties-to-even 12"
        );
    }

    #[test]
    fn rejects_out_of_domain_shares() {
        let error = HydrologyConfig {
            river_land_share_percent: 0.0,
        }
        .to_params()
        .unwrap_err();
        assert!(matches!(error, ParamsError::OutOfDomain { .. }));

        let error = from_toml_str("river_land_share_percent = 20.0\n").unwrap_err();
        assert!(matches!(error, ParamsError::OutOfDomain { .. }));
    }

    #[test]
    fn rejects_unknown_fields_and_non_finite_values() {
        let error = from_toml_str("unknown_field = 1\n").unwrap_err();
        assert!(matches!(error, ParamsError::Parse(_)));

        let error = from_toml_str("river_land_share_percent = nan\n").unwrap_err();
        assert!(matches!(error, ParamsError::NonFinite { .. }));
    }
}
