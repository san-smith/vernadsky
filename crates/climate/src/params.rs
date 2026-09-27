//! TOML configuration of the climate stage.
//!
//! The config carries human-facing float fields; every accepted value is
//! validated against its conventional domain and quantized onto the
//! integer parameter lattices of [`ClimateParams`], so what reaches the
//! export and the stage is canonical and hashable.
//!
//! A configuration document looks like this (all fields optional,
//! unknown fields are rejected):
//!
//! ```toml
//! temperature_offset_c = -3.0     # global offset, °C, domain [-10, 10]
//! polar_amplification = 1.5       # offset amplification at the poles, [0, 3]
//! latitude_exponent = 1.0         # latitudinal profile compression, [0.5, 2]
//! humidity_offset = 0.3           # baseline moisture, normalized, [-0.4, 0.4]
//! ```

use std::fmt;

use serde::Deserialize;
use vernadsky_core::quant::{CentiScalar, QuantError, TempDeciC};
use vernadsky_core::schema::ClimateParams;

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
            ParamsError::Parse(message) => write!(f, "invalid climate config: {message}"),
            ParamsError::NonFinite { field } => {
                write!(f, "climate config field {field} must be finite")
            }
            ParamsError::OutOfDomain {
                field,
                value,
                domain,
            } => write!(
                f,
                "climate config field {field} = {value} is outside its domain {domain}"
            ),
            ParamsError::Quantization { field, source } => {
                write!(f, "climate config field {field}: {source}")
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

/// TOML configuration of the climate stage.
///
/// Defaults describe a temperate Earth-like setting: no global
/// temperature offset, moderate polar amplification, an uncompressed
/// latitudinal profile, and a baseline moisture of `0.3` of the
/// normalized unit.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ClimateConfig {
    /// Global temperature offset over the whole planet, in °C.
    #[serde(default = "default_temperature_offset")]
    pub temperature_offset_c: f64,
    /// Amplification of the offset toward the poles.
    #[serde(default = "default_polar_amplification")]
    pub polar_amplification: f64,
    /// Compression exponent of the latitudinal profile: values above `1`
    /// shrink the temperate belt, values below `1` widen it.
    #[serde(default = "default_latitude_exponent")]
    pub latitude_exponent: f64,
    /// Baseline moisture added to land cells, in the normalized unit.
    #[serde(default = "default_humidity_offset")]
    pub humidity_offset: f64,
}

fn default_temperature_offset() -> f64 {
    0.0
}

fn default_polar_amplification() -> f64 {
    1.0
}

fn default_latitude_exponent() -> f64 {
    1.0
}

fn default_humidity_offset() -> f64 {
    0.3
}

impl Default for ClimateConfig {
    fn default() -> Self {
        Self {
            temperature_offset_c: default_temperature_offset(),
            polar_amplification: default_polar_amplification(),
            latitude_exponent: default_latitude_exponent(),
            humidity_offset: default_humidity_offset(),
        }
    }
}

impl ClimateConfig {
    /// Validates the configuration and converts it into the canonical
    /// parameter lattices.
    pub fn to_params(&self) -> Result<ClimateParams, ParamsError> {
        Ok(ClimateParams {
            temperature_offset: TempDeciC::try_from_value(check(
                "temperature_offset_c",
                self.temperature_offset_c,
                -10.0..=10.0,
            )?)
            .map_err(|source| ParamsError::Quantization {
                field: "temperature_offset_c",
                source,
            })?,
            polar_amplification: CentiScalar::try_from_value(check(
                "polar_amplification",
                self.polar_amplification,
                0.0..=3.0,
            )?)
            .map_err(|source| ParamsError::Quantization {
                field: "polar_amplification",
                source,
            })?,
            latitude_exponent: CentiScalar::try_from_value(check(
                "latitude_exponent",
                self.latitude_exponent,
                0.5..=2.0,
            )?)
            .map_err(|source| ParamsError::Quantization {
                field: "latitude_exponent",
                source,
            })?,
            humidity_offset: CentiScalar::try_from_value(check(
                "humidity_offset",
                self.humidity_offset,
                -0.4..=0.4,
            )?)
            .map_err(|source| ParamsError::Quantization {
                field: "humidity_offset",
                source,
            })?,
        })
    }
}

/// Loads and validates climate parameters from a TOML document.
pub fn from_toml_str(source: &str) -> Result<ClimateParams, ParamsError> {
    let config: ClimateConfig =
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
            domain: "documented in `ClimateConfig`",
        });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_canonical() {
        let params = ClimateConfig::default()
            .to_params()
            .expect("valid defaults");
        assert_eq!(params.temperature_offset, TempDeciC(0));
        assert_eq!(params.polar_amplification, CentiScalar(100));
        assert_eq!(params.latitude_exponent, CentiScalar(100));
        assert_eq!(params.humidity_offset, CentiScalar(30));
    }

    #[test]
    fn parses_a_full_document() {
        let params = from_toml_str(
            "temperature_offset_c = -3.5\npolar_amplification = 2.0\nlatitude_exponent = 0.75\nhumidity_offset = -0.2\n",
        )
        .expect("valid document");
        assert_eq!(params.temperature_offset, TempDeciC(-35));
        assert_eq!(params.polar_amplification, CentiScalar(200));
        assert_eq!(params.latitude_exponent, CentiScalar(75));
        assert_eq!(params.humidity_offset, CentiScalar(-20));
    }

    #[test]
    fn rejects_unknown_fields() {
        let error = from_toml_str("unknown_field = 1\n").expect_err("unknown field");
        assert!(matches!(error, ParamsError::Parse(_)));
    }

    #[test]
    fn rejects_out_of_domain_values() {
        let error = from_toml_str("polar_amplification = 9.0\n").expect_err("out of domain");
        assert_eq!(
            error,
            ParamsError::OutOfDomain {
                field: "polar_amplification",
                value: 9.0,
                domain: "documented in `ClimateConfig`",
            }
        );
    }

    #[test]
    fn rejects_non_finite_values() {
        let error = from_toml_str("temperature_offset_c = nan\n").expect_err("non-finite");
        assert_eq!(
            error,
            ParamsError::NonFinite {
                field: "temperature_offset_c",
            }
        );
    }
}
