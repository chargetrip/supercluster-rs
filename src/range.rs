use std::cell::OnceCell;

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

    // Cached values for offset and scale
    pub offset: OnceCell<f64>,
    pub scale: OnceCell<f64>,
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
    fn offset(&self) -> f64 {
        *self.offset.get_or_init(|| f64::min(self.min_x, self.min_y))
    }

    /// Compute and cache the maximum range value.
    fn scale(&self) -> f64 {
        *self
            .scale
            .get_or_init(|| f64::max(self.max_x, self.max_y) - self.offset())
    }
}

impl Default for DataRange {
    fn default() -> Self {
        Self {
            min_x: 0.0,
            min_y: 0.0,
            max_x: 1.0,
            max_y: 1.0,
            offset: OnceCell::new(),
            scale: OnceCell::new(),
        }
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
