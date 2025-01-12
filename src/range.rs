/// The range of the incoming data if choosing the cartesian coordinate system.
/// Applicable for non-geospatial data (i.e. microscopy, etc.).
#[derive(Clone, Debug)]
pub struct DataRange {
    /// The minimum x-coordinate value.
    pub min_x: f64,

    /// The minimum y-coordinate value.
    pub min_y: f64,

    /// The maximum x-coordinate value.
    pub max_x: f64,

    /// The maximum y-coordinate value.
    pub max_y: f64,
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
        let range_min = f64::min(self.min_x, self.min_y);
        let range_max = f64::max(self.max_x, self.max_y);
        (v - range_min) / (range_max - range_min)
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
        let range_min = f64::min(self.min_x, self.min_y);
        let range_max = f64::max(self.max_x, self.max_y);
        v_scaled * (range_max - range_min) + range_min
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_range() {
        let data_range = DataRange {
            min_x: -10.0,
            max_x: 100.0,
            min_y: -20.0,
            max_y: 50.0,
        };

        assert_eq!(data_range.normalize(-20.0), 0.0);
        assert_eq!(data_range.normalize(40.0), 0.5);
        assert_eq!(data_range.normalize(100.0), 1.0);

        assert_eq!(data_range.denormalize(0.0), -20.0);
        assert_eq!(data_range.denormalize(0.5), 40.0);
        assert_eq!(data_range.denormalize(1.0), 100.0);
    }
}
