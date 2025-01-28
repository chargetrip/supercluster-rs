use serde::{Deserialize, Serialize};

/// The range of the incoming data if choosing the cartesian coordinate system.
/// Applicable for non-geospatial data (i.e. microscopy, etc.).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct DataRange {
    /// The minimum x-coordinate value.
    pub min_x: f64,

    /// The minimum y-coordinate value.
    pub min_y: f64,

    /// The maximum x-coordinate value.
    pub max_x: f64,

    /// The maximum y-coordinate value.
    pub max_y: f64,

    /// The cached value for offset.
    pub offset: Option<f64>,

    // The cached value for scale.
    pub scale: Option<f64>,
}

impl DataRange {
    /// Normalize the coordinate value to the range [0, 1].
    ///
    /// # Arguments
    ///
    /// - `v`: The coordinate value to be normalized.
    ///
    /// # Returns
    ///
    /// The normalized coordinate value.
    pub fn normalize(&self, v: f64) -> f64 {
        (v - self.offset()) / self.scale()
    }

    /// Denormalize the coordinate value from the range [0, 1] to the original range.
    ///
    /// # Arguments
    ///
    /// - `v_scaled`: The scaled coordinate value to be denormalized.
    ///
    /// # Returns
    ///
    /// The denormalized coordinate value.
    pub fn denormalize(&self, v_scaled: f64) -> f64 {
        v_scaled * self.scale() + self.offset()
    }

    /// Compute and cache the minimum range value.
    ///
    /// # Returns
    ///
    /// The minimum range value.
    fn offset(&self) -> f64 {
        self.offset
            .unwrap_or_else(|| f64::min(self.min_x, self.min_y))
    }

    /// Compute and cache the maximum range value.
    ///
    /// # Returns
    ///
    /// The maximum range value.
    fn scale(&self) -> f64 {
        self.scale
            .unwrap_or_else(|| f64::max(self.max_x, self.max_y) - self.offset())
    }
}

impl Default for DataRange {
    /// Create a new `DataRange` with default values.
    ///
    /// # Returns
    ///
    /// A new `DataRange` with default values.
    fn default() -> Self {
        Self {
            min_x: 0.0,
            min_y: 0.0,
            max_x: 1.0,
            max_y: 1.0,
            offset: None,
            scale: None,
        }
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
        let data_range = DataRange {
            min_x: -10.0,
            max_x: 100.0,
            min_y: -20.0,
            max_y: 50.0,
            ..Default::default()
        };

        assert_eq!(data_range.normalize(-20.0), 0.0);
        assert_eq!(data_range.normalize(40.0), 0.5);
        assert_eq!(data_range.normalize(100.0), 1.0);

        assert_eq!(data_range.denormalize(0.0), -20.0);
        assert_eq!(data_range.denormalize(0.5), 40.0);
        assert_eq!(data_range.denormalize(1.0), 100.0);
    }
}
