//! # Range module
//!
//! This module contains the range of the data for the `Cartesian` coordinate system.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// The range of the incoming data for the `Cartesian` coordinate system.
/// Applicable for non-geospatial data (i.e. microscopy, etc.).
///
/// Both axes are scaled by the same factor, from the lowest minimum to the highest maximum, so the
/// data keeps its proportions.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize))]
#[non_exhaustive]
pub struct DataRange {
    /// The minimum X value.
    pub min_x: f64,

    /// The minimum Y value.
    pub min_y: f64,

    /// The maximum X value.
    pub max_x: f64,

    /// The maximum Y value.
    pub max_y: f64,
}

impl DataRange {
    /// Create a data range from the bounds of the data. Building the options checks it.
    ///
    /// # Examples
    ///
    /// ```
    /// use supercluster::{CoordinateSystem, DataRange, SuperclusterOptions};
    ///
    /// // A microscope image of 2048 by 1024 pixels
    /// let range = DataRange::new(0.0, 0.0, 2048.0, 1024.0);
    /// let options = SuperclusterOptions::builder()
    ///     .coordinate_system(CoordinateSystem::Cartesian { range })
    ///     .build()?;
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Self {
        DataRange {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    /// Normalize the coordinate value to the range `[0, 1]`.
    /// The formula is `(v - offset) / scale`.
    pub fn normalize(&self, v: f64) -> f64 {
        (v - self.offset()) / self.scale()
    }

    /// Denormalize the coordinate value from the range `[0, 1]` to the original range.
    /// The formula is `v_scaled * scale + offset`.
    pub fn denormalize(&self, v_scaled: f64) -> f64 {
        v_scaled * self.scale() + self.offset()
    }

    /// Check whether the range can normalize coordinates: its bounds are finite, no maximum is below
    /// its minimum and the scale is above 0.
    pub(crate) fn is_valid(&self) -> bool {
        [self.min_x, self.min_y, self.max_x, self.max_y]
            .iter()
            .all(|value| value.is_finite())
            && self.max_x >= self.min_x
            && self.max_y >= self.min_y
            && self.scale() > 0.0
    }

    /// Get the lowest minimum of the two axes, which normalizes to 0.
    fn offset(&self) -> f64 {
        f64::min(self.min_x, self.min_y)
    }

    /// Get the distance from the lowest minimum to the highest maximum of the two axes, which
    /// normalizes to 1.
    fn scale(&self) -> f64 {
        f64::max(self.max_x, self.max_y) - self.offset()
    }
}

impl Default for DataRange {
    /// Create the range from 0 to 1 on both axes.
    fn default() -> Self {
        DataRange::new(0.0, 0.0, 1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_range_default() {
        let data_range = DataRange::default();

        assert_eq!(data_range.min_x, 0.0);
        assert_eq!(data_range.min_y, 0.0);
        assert_eq!(data_range.max_x, 1.0);
        assert_eq!(data_range.max_y, 1.0);
    }

    #[test]
    fn test_data_range() {
        let data_range = DataRange::new(-10.0, -20.0, 100.0, 50.0);

        assert_eq!(data_range.normalize(-20.0), 0.0);
        assert_eq!(data_range.normalize(40.0), 0.5);
        assert_eq!(data_range.normalize(100.0), 1.0);

        assert_eq!(data_range.denormalize(0.0), -20.0);
        assert_eq!(data_range.denormalize(0.5), 40.0);
        assert_eq!(data_range.denormalize(1.0), 100.0);
    }

    #[test]
    fn test_is_valid() {
        assert!(DataRange::default().is_valid());

        // A single line of points still has a scale
        assert!(DataRange::new(0.0, 5.0, 10.0, 5.0).is_valid());

        assert!(!DataRange::new(5.0, 5.0, 5.0, 5.0).is_valid());
        assert!(!DataRange::new(10.0, 0.0, 0.0, 10.0).is_valid());
        assert!(!DataRange::new(0.0, 0.0, f64::INFINITY, 10.0).is_valid());
        assert!(!DataRange::new(f64::NAN, 0.0, 10.0, 10.0).is_valid());
    }
}
