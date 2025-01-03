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
    /// Normalize the x-coordinate value to the range [0, 1].
    ///
    /// # Arguments
    ///
    /// - `x`: The x-coordinate value to be normalized.
    ///
    /// # Returns
    ///
    /// The normalized x-coordinate value.
    pub fn normalize_x(&self, x: f64) -> f64 {
        (x - self.min_x) / (self.max_x - self.min_x)
    }

    /// Normalize the y-coordinate value to the range [0, 1].
    ///
    /// # Arguments
    ///
    /// - `y`: The y-coordinate value to be normalized.
    ///
    /// # Returns
    ///
    /// The normalized y-coordinate value.
    pub fn normalize_y(&self, y: f64) -> f64 {
        (y - self.min_y) / (self.max_y - self.min_y)
    }

    /// Denormalize the x-coordinate value from the range [0, 1] to the original range.
    ///
    /// # Arguments
    ///
    /// - `x_scaled`: The scaled x-coordinate value to be denormalized.
    ///
    /// # Returns
    ///
    /// The denormalized x-coordinate value.
    pub fn denormalize_x(&self, x_scaled: f64) -> f64 {
        x_scaled * (self.max_x - self.min_x) + self.min_x
    }

    /// Denormalize the y-coordinate value from the range [0, 1] to the original range.
    ///
    /// # Arguments
    ///
    /// - `y_scaled`: The scaled y-coordinate value to be denormalized.
    ///
    /// # Returns
    ///
    /// The denormalized y-coordinate value.
    pub fn denormalize_y(&self, y_scaled: f64) -> f64 {
        y_scaled * (self.max_y - self.min_y) + self.min_y
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

        assert_eq!(data_range.normalize_x(-10.0), 0.0);
        assert_eq!(data_range.normalize_x(45.0), 0.5);
        assert_eq!(data_range.normalize_x(100.0), 1.0);
        assert_eq!(data_range.normalize_y(-20.0), 0.0);
        assert_eq!(data_range.normalize_y(15.0), 0.5);
        assert_eq!(data_range.normalize_y(50.0), 1.0);

        assert_eq!(data_range.denormalize_x(0.0), -10.0);
        assert_eq!(data_range.denormalize_x(0.5), 45.0);
        assert_eq!(data_range.denormalize_x(1.0), 100.0);
        assert_eq!(data_range.denormalize_y(0.0), -20.0);
        assert_eq!(data_range.denormalize_y(0.5), 15.0);
        assert_eq!(data_range.denormalize_y(1.0), 50.0);
    }
}
