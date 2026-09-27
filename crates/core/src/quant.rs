//! Lattice quantization for exported physical quantities.
//!
//! Physical quantities never cross a generation-stage boundary or leave the
//! crate as raw floats: every hand-off uses integer lattices — a fixed
//! quantum per quantity, stored as a typed integer. Float-to-lattice
//! conversion is allowed only through this module; ad-hoc `as` casts from
//! floats in stage code are a policy violation.
//!
//! Rounding is nearest with ties-to-even (`f64::round_ties_even`), which is
//! exact and platform-independent for a given `f64` input. Non-finite and
//! out-of-range inputs are rejected, never clamped: silent clamping would
//! hide upstream drift instead of surfacing it.
//!
//! The lattice set is versioned: [`LATTICE_REGISTRY_VERSION`] is written
//! into every export, and a reader that does not know the version must
//! reject the file instead of guessing.

use std::fmt;

/// Version of the lattice registry understood (and written) by this crate.
pub const LATTICE_REGISTRY_VERSION: &str = "lr-v1";

/// Errors produced by lattice conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuantError {
    /// The input value is NaN or infinite.
    NonFinite,
    /// The quantized value does not fit the lattice storage type.
    OutOfRange,
}

impl fmt::Display for QuantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuantError::NonFinite => write!(f, "cannot quantize a non-finite value"),
            QuantError::OutOfRange => write!(f, "quantized value does not fit the lattice type"),
        }
    }
}

impl std::error::Error for QuantError {}

macro_rules! lattice_newtype {
    ($(#[$doc:meta])* $name:ident($storage:ty), $quantum:expr) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub $storage);

        impl $name {
            /// Quantum of the lattice: one storage step expressed in the
            /// physical unit documented on the type.
            pub const QUANTUM: f64 = $quantum;

            /// Quantizes a physical value onto the lattice: nearest step,
            /// ties-to-even.
            pub fn try_from_value(value: f64) -> Result<Self, QuantError> {
                if !value.is_finite() {
                    return Err(QuantError::NonFinite);
                }
                let steps = value / Self::QUANTUM;
                let rounded = steps.round_ties_even();
                if rounded < (<$storage>::MIN as f64) || rounded > (<$storage>::MAX as f64) {
                    return Err(QuantError::OutOfRange);
                }
                Ok(Self(rounded as $storage))
            }

            /// The exact physical value of this lattice step.
            pub fn to_value(self) -> f64 {
                self.0 as f64 * Self::QUANTUM
            }
        }
    };
}

lattice_newtype! {
    /// Height above or below sea level, in whole meters.
    ///
    /// Sea surface is `0`; negative values are depths.
    HeightM(i32), 1.0
}

lattice_newtype! {
    /// Temperature in tenths of a degree Celsius.
    TempDeciC(i16), 0.1
}

lattice_newtype! {
    /// Relative humidity in tenths of a percent.
    HumidDeciPct(i16), 0.1
}

lattice_newtype! {
    /// Precipitation in millimeters per year.
    PrecipMmYr(i32), 1.0
}

lattice_newtype! {
    /// Natural potential of a territory in permille (scale `0..=1000`).
    ///
    /// The domain restriction to `0..=1000` is semantic and enforced by
    /// producers, not by the lattice type.
    NatPotential(i16), 0.001
}

lattice_newtype! {
    /// Weight used for canonical ordering of territory seeds, in permille.
    ///
    /// Sorting decisions consume this lattice, never the raw float weights.
    SeedWeight(i32), 0.001
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ties_round_to_even() {
        // Half cases are exactly representable on the unit-quantum lattice.
        assert_eq!(HeightM::try_from_value(0.5), Ok(HeightM(0)));
        assert_eq!(HeightM::try_from_value(1.5), Ok(HeightM(2)));
        assert_eq!(HeightM::try_from_value(2.5), Ok(HeightM(2)));
        assert_eq!(HeightM::try_from_value(-0.5), Ok(HeightM(0)));
        assert_eq!(HeightM::try_from_value(-1.5), Ok(HeightM(-2)));
    }

    #[test]
    fn nearest_rounding() {
        assert_eq!(HeightM::try_from_value(2.4), Ok(HeightM(2)));
        assert_eq!(HeightM::try_from_value(2.6), Ok(HeightM(3)));
        assert_eq!(HeightM::try_from_value(-2.4), Ok(HeightM(-2)));
        assert_eq!(HeightM::try_from_value(-2.6), Ok(HeightM(-3)));
    }

    #[test]
    fn non_unit_quantum() {
        // Decode direction is exact in the lattice unit only up to f64
        // representation of the decimal quantum; assert via the lattice
        // step count and an epsilon on the decoded value.
        assert_eq!(TempDeciC::try_from_value(23.2), Ok(TempDeciC(232)));
        assert!((TempDeciC(232).to_value() - 23.2).abs() < 1e-9);
        assert_eq!(NatPotential::try_from_value(0.741), Ok(NatPotential(741)));
    }

    #[test]
    fn rejects_non_finite_and_out_of_range() {
        assert_eq!(
            HeightM::try_from_value(f64::NAN),
            Err(QuantError::NonFinite)
        );
        assert_eq!(
            HeightM::try_from_value(f64::INFINITY),
            Err(QuantError::NonFinite)
        );
        assert_eq!(
            TempDeciC::try_from_value(f64::MAX),
            Err(QuantError::OutOfRange)
        );
        assert_eq!(
            NatPotential::try_from_value(-1.0e12),
            Err(QuantError::OutOfRange)
        );
    }

    #[test]
    fn registry_version_is_stable() {
        assert_eq!(LATTICE_REGISTRY_VERSION, "lr-v1");
    }
}
