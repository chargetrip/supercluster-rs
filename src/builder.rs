//! # Builder module
//!
//! This module contains the supercluster options and the builder that checks them.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{CoordinateSystem, SuperclusterError, MAX_ZOOM_LIMIT};

/// Supercluster configuration options.
/// The options are valid by construction: only `SuperclusterOptions::builder()`, `Default` and,
/// with the `serde` feature, deserialization create them, and the first and last check the values.
/// Their fields are private, so read them with the methods of the same names, and they cannot
/// change once they are checked.
#[derive(Clone, Debug, PartialEq)]
// Written as the builder they are read as, so a format that reads values in the order they were
// written, without their names, reads back the same types
#[cfg_attr(
    feature = "serde",
    derive(Deserialize, Serialize),
    serde(
        try_from = "SuperclusterOptionsBuilder",
        into = "SuperclusterOptionsBuilder"
    )
)]
#[non_exhaustive]
pub struct SuperclusterOptions {
    /// Minimal zoom level to generate clusters on.
    /// The default value is 0.
    pub(crate) min_zoom: u8,

    /// Maximal zoom level to cluster the points on.
    /// The default value is 16.
    pub(crate) max_zoom: u8,

    /// Minimum points to form a cluster.
    /// The default value is 2.
    pub(crate) min_points: u32,

    /// Cluster radius, in pixels.
    /// The default value is 40.0.
    pub(crate) radius: f64,

    /// Tile extent (radius is calculated relative to it).
    /// The default value is 512.
    pub(crate) extent: u32,

    /// Size of the KD-tree leaf node, affects performance.
    /// The default value is 64.
    pub(crate) node_size: usize,

    /// Type of coordinate system for clustering.
    /// The default value is `CoordinateSystem::LngLat`.
    pub(crate) coordinate_system: CoordinateSystem,
}

impl SuperclusterOptions {
    /// Start building options. A value that is not set keeps its default.
    ///
    /// # Examples
    ///
    /// ```
    /// use supercluster::SuperclusterOptions;
    ///
    /// let options = SuperclusterOptions::builder()
    ///     .radius(60.0)
    ///     .max_zoom(14)
    ///     .build()?;
    ///
    /// assert_eq!(options.radius(), 60.0);
    /// // The extent keeps its default
    /// assert_eq!(options.extent(), 512);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    #[must_use]
    pub fn builder() -> SuperclusterOptionsBuilder {
        SuperclusterOptionsBuilder::default()
    }

    /// Get the lowest zoom level to cluster on.
    #[must_use]
    pub fn min_zoom(&self) -> u8 {
        self.min_zoom
    }

    /// Get the highest zoom level to cluster on.
    #[must_use]
    pub fn max_zoom(&self) -> u8 {
        self.max_zoom
    }

    /// Get the fewest points that form a cluster.
    #[must_use]
    pub fn min_points(&self) -> u32 {
        self.min_points
    }

    /// Get the cluster radius, in pixels.
    #[must_use]
    pub fn radius(&self) -> f64 {
        self.radius
    }

    /// Get the size of a tile, in pixels, which the radius is relative to.
    #[must_use]
    pub fn extent(&self) -> u32 {
        self.extent
    }

    /// Get the number of points in a leaf of the KD-tree.
    #[must_use]
    pub fn node_size(&self) -> usize {
        self.node_size
    }

    /// Get the coordinate system of the points.
    #[must_use]
    pub fn coordinate_system(&self) -> &CoordinateSystem {
        &self.coordinate_system
    }
}

impl Default for SuperclusterOptions {
    /// Create the default options.
    fn default() -> Self {
        SuperclusterOptions {
            min_zoom: 0,
            max_zoom: 16,
            min_points: 2,
            radius: 40.0,
            extent: 512,
            node_size: 64,
            coordinate_system: CoordinateSystem::LngLat,
        }
    }
}

/// Builder of the supercluster options, which checks the values when it builds them. Serde reads
/// the options through it, and rejects a field it does not know, so a misspelled option gives an
/// error instead of its default.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct SuperclusterOptionsBuilder {
    /// Minimal zoom level to generate clusters on.
    min_zoom: Option<u8>,

    /// Maximal zoom level to cluster the points on.
    max_zoom: Option<u8>,

    /// Minimum points to form a cluster.
    min_points: Option<u32>,

    /// Cluster radius, in pixels.
    radius: Option<f64>,

    /// Tile extent (radius is calculated relative to it).
    extent: Option<u32>,

    /// Size of the KD-tree leaf node.
    node_size: Option<usize>,

    /// Type of coordinate system for clustering.
    coordinate_system: Option<CoordinateSystem>,
}

impl SuperclusterOptionsBuilder {
    /// Set the minimal zoom level to generate clusters on.
    #[must_use]
    pub fn min_zoom(mut self, min_zoom: u8) -> Self {
        self.min_zoom = Some(min_zoom);
        self
    }

    /// Set the maximal zoom level to cluster the points on.
    #[must_use]
    pub fn max_zoom(mut self, max_zoom: u8) -> Self {
        self.max_zoom = Some(max_zoom);
        self
    }

    /// Set the minimum points to form a cluster.
    #[must_use]
    pub fn min_points(mut self, min_points: u32) -> Self {
        self.min_points = Some(min_points);
        self
    }

    /// Set the cluster radius in pixels.
    #[must_use]
    pub fn radius(mut self, radius: f64) -> Self {
        self.radius = Some(radius);
        self
    }

    /// Set the tile extent (radius is calculated relative to it).
    #[must_use]
    pub fn extent(mut self, extent: u32) -> Self {
        self.extent = Some(extent);
        self
    }

    /// Set the size of the KD-tree leaf node, affects performance.
    #[must_use]
    pub fn node_size(mut self, node_size: usize) -> Self {
        self.node_size = Some(node_size);
        self
    }

    /// Set the type of coordinate system for clustering.
    #[must_use]
    pub fn coordinate_system(mut self, coordinate_system: CoordinateSystem) -> Self {
        self.coordinate_system = Some(coordinate_system);
        self
    }

    /// Build the supercluster options.
    ///
    /// # Errors
    ///
    /// - `InvalidMaxZoom` if the maximum zoom level is above `MAX_ZOOM_LIMIT`.
    /// - `InvalidMinZoom` if the minimum zoom level is above the maximum zoom level.
    /// - `InvalidRadius` if the radius is not a finite number above 0, or if the extent plus the
    ///   radius is above `i32::MAX`, which the positions in a tile could then not hold.
    /// - `InvalidExtent` if the extent is 0 or above `i32::MAX`.
    /// - `InvalidNodeSize` if the node size is 0.
    /// - `InvalidDataRange` if the `DataRange` of the `Cartesian` coordinate system cannot scale the
    ///   data.
    pub fn build(self) -> Result<SuperclusterOptions, SuperclusterError> {
        let defaults = SuperclusterOptions::default();
        let options = SuperclusterOptions {
            min_zoom: self.min_zoom.unwrap_or(defaults.min_zoom),
            max_zoom: self.max_zoom.unwrap_or(defaults.max_zoom),
            min_points: self.min_points.unwrap_or(defaults.min_points),
            radius: self.radius.unwrap_or(defaults.radius),
            extent: self.extent.unwrap_or(defaults.extent),
            node_size: self.node_size.unwrap_or(defaults.node_size),
            coordinate_system: self.coordinate_system.unwrap_or(defaults.coordinate_system),
        };

        if options.max_zoom > MAX_ZOOM_LIMIT {
            return Err(SuperclusterError::InvalidMaxZoom);
        }

        if options.min_zoom > options.max_zoom {
            return Err(SuperclusterError::InvalidMinZoom);
        }

        if !is_positive(options.radius) {
            return Err(SuperclusterError::InvalidRadius);
        }

        if options.extent == 0 || options.extent > i32::MAX as u32 {
            return Err(SuperclusterError::InvalidExtent);
        }

        // An item of a tile is at most the radius outside it, and its position is an `i32`, so the
        // extent plus the radius must fit one
        if f64::from(options.extent) + options.radius > f64::from(i32::MAX) {
            return Err(SuperclusterError::InvalidRadius);
        }

        if options.node_size == 0 {
            return Err(SuperclusterError::InvalidNodeSize);
        }

        if let CoordinateSystem::Cartesian { range } = &options.coordinate_system {
            if !range.is_valid() {
                return Err(SuperclusterError::InvalidDataRange);
            }
        }

        Ok(options)
    }
}

#[cfg(feature = "serde")]
impl TryFrom<SuperclusterOptionsBuilder> for SuperclusterOptions {
    type Error = SuperclusterError;

    fn try_from(builder: SuperclusterOptionsBuilder) -> Result<Self, Self::Error> {
        builder.build()
    }
}

// The builder with every value of the options set, which the options are written as
#[cfg(feature = "serde")]
impl From<SuperclusterOptions> for SuperclusterOptionsBuilder {
    fn from(options: SuperclusterOptions) -> Self {
        SuperclusterOptionsBuilder {
            min_zoom: Some(options.min_zoom),
            max_zoom: Some(options.max_zoom),
            min_points: Some(options.min_points),
            radius: Some(options.radius),
            extent: Some(options.extent),
            node_size: Some(options.node_size),
            coordinate_system: Some(options.coordinate_system),
        }
    }
}

/// Setting a field of the options does not compile outside the crate, because it would skip the
/// checks of the builder. This item only exists for the doc tests, so the documentation does not
/// show code that fails.
///
/// ```compile_fail,E0616
/// let mut options = supercluster::SuperclusterOptions::default();
///
/// options.node_size = 0;
/// ```
#[cfg(doctest)]
struct OptionsCannotChange;

/// Check whether a value is a finite number above 0, which `NaN` is not.
fn is_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::DataRange;

    #[test]
    fn test_options_are_equal_with_the_same_values() {
        let options = SuperclusterOptions::builder().radius(60.0).build().unwrap();

        assert_eq!(options, options.clone());
        assert_eq!(
            SuperclusterOptions::default(),
            SuperclusterOptions::default()
        );
        assert_ne!(options, SuperclusterOptions::default());
    }

    #[test]
    fn test_default() {
        let options = SuperclusterOptions::default();

        assert_eq!(options.min_zoom, 0);
        assert_eq!(options.max_zoom, 16);
        assert_eq!(options.min_points, 2);
        assert_eq!(options.radius, 40.0);
        assert_eq!(options.extent, 512);
        assert_eq!(options.node_size, 64);
        assert_eq!(options.coordinate_system, CoordinateSystem::LngLat);
    }

    #[test]
    fn test_build_without_values() {
        let options = SuperclusterOptions::builder().build().unwrap();
        let defaults = SuperclusterOptions::default();

        assert_eq!(options.max_zoom, defaults.max_zoom);
        assert_eq!(options.radius, defaults.radius);
    }

    #[test]
    fn test_build() {
        let options = SuperclusterOptions::builder()
            .min_zoom(1)
            .max_zoom(10)
            .min_points(5)
            .radius(50.0)
            .extent(1024)
            .node_size(128)
            .coordinate_system(CoordinateSystem::LngLat)
            .build()
            .unwrap();

        assert_eq!(options.min_zoom, 1);
        assert_eq!(options.max_zoom, 10);
        assert_eq!(options.min_points, 5);
        assert_eq!(options.radius, 50.0);
        assert_eq!(options.extent, 1024);
        assert_eq!(options.node_size, 128);
        assert_eq!(options.coordinate_system, CoordinateSystem::LngLat);
    }

    #[test]
    fn test_build_with_invalid_max_zoom() {
        assert_eq!(
            SuperclusterOptions::builder()
                .max_zoom(u8::MAX)
                .build()
                .err(),
            Some(SuperclusterError::InvalidMaxZoom)
        );
    }

    #[test]
    fn test_build_with_invalid_min_zoom() {
        assert_eq!(
            SuperclusterOptions::builder()
                .min_zoom(10)
                .max_zoom(5)
                .build()
                .err(),
            Some(SuperclusterError::InvalidMinZoom)
        );
    }

    #[test]
    fn test_build_with_invalid_radius() {
        for radius in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                SuperclusterOptions::builder().radius(radius).build().err(),
                Some(SuperclusterError::InvalidRadius)
            );
        }
    }

    #[test]
    fn test_build_with_invalid_extent() {
        for extent in [0, i32::MAX as u32 + 1, u32::MAX] {
            assert_eq!(
                SuperclusterOptions::builder().extent(extent).build().err(),
                Some(SuperclusterError::InvalidExtent)
            );
        }
    }

    #[test]
    fn test_build_with_positions_past_an_i32() {
        let build = |extent: u32, radius: f64| {
            SuperclusterOptions::builder()
                .extent(extent)
                .radius(radius)
                .build()
                .err()
        };

        // The positions of a tile reach the extent plus the radius
        assert_eq!(
            build(i32::MAX as u32, 1.0),
            Some(SuperclusterError::InvalidRadius)
        );
        assert_eq!(build(512, 3e9), Some(SuperclusterError::InvalidRadius));
        assert_eq!(build(i32::MAX as u32 - 40, 40.0), None);
    }

    #[test]
    fn test_build_with_invalid_node_size() {
        assert_eq!(
            SuperclusterOptions::builder().node_size(0).build().err(),
            Some(SuperclusterError::InvalidNodeSize)
        );
    }

    #[test]
    fn test_build_with_invalid_data_range() {
        let range = DataRange::new(5.0, 5.0, 5.0, 5.0);

        assert_eq!(
            SuperclusterOptions::builder()
                .coordinate_system(CoordinateSystem::Cartesian { range })
                .build()
                .err(),
            Some(SuperclusterError::InvalidDataRange)
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_options_from_json_with_invalid_node_size() {
        let options = serde_json::from_str::<SuperclusterOptions>(r#"{ "node_size": 0 }"#);

        assert!(options
            .unwrap_err()
            .to_string()
            .contains("The node size cannot be 0."));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_options_from_json_with_an_unknown_field() {
        let options = serde_json::from_str::<SuperclusterOptions>(r#"{ "raduis": 80.0 }"#);

        assert!(options
            .unwrap_err()
            .to_string()
            .contains("unknown field `raduis`"));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_options_from_json_with_an_unknown_field_in_the_coordinate_system() {
        let in_range = serde_json::from_str::<SuperclusterOptions>(
            r#"{ "coordinate_system": { "Cartesian": { "range": {
                "min_x": 0.0, "min_y": 0.0, "max_x": 1.0, "max_y": 1.0, "max_z": 5.0
            } } } }"#,
        );
        let in_variant = serde_json::from_str::<SuperclusterOptions>(
            r#"{ "coordinate_system": { "Cartesian": {
                "range": { "min_x": 0.0, "min_y": 0.0, "max_x": 1.0, "max_y": 1.0 },
                "ranges": 1
            } } }"#,
        );

        assert!(in_range
            .unwrap_err()
            .to_string()
            .contains("unknown field `max_z`"));
        assert!(in_variant
            .unwrap_err()
            .to_string()
            .contains("unknown field `ranges`"));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_options_from_json() {
        let options: SuperclusterOptions = serde_json::from_str(
            r#"{
                "radius": 40.0,
                "extent": 512,
                "min_points": 1,
                "node_size": 64,
                "min_zoom": 2,
                "max_zoom": 16,
                "coordinate_system": "LngLat"
            }"#,
        )
        .unwrap();

        assert_eq!(options.min_zoom, 2);
        assert_eq!(options.max_zoom, 16);
        assert_eq!(options.min_points, 1);
        assert_eq!(options.coordinate_system, CoordinateSystem::LngLat);
    }
}
