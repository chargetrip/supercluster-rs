//! # Builder module
//!
//! This module contains the builder pattern for the supercluster configuration settings.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{CoordinateSystem, SuperclusterError, MAX_ZOOM_LIMIT};

/// Supercluster configuration options.
/// The options are valid by construction: only `SuperclusterBuilder::build` and, with the `serde`
/// feature, deserialization create them, and both check the values.
#[derive(Clone, Debug)]
#[cfg_attr(
    feature = "serde",
    derive(Deserialize, Serialize),
    serde(try_from = "SuperclusterBuilder")
)]
#[non_exhaustive]
pub struct SuperclusterOptions {
    /// Minimal zoom level to generate clusters on.
    /// The default value is 0.
    pub min_zoom: u8,

    /// Maximal zoom level to cluster the points on.
    /// The default value is 16.
    pub max_zoom: u8,

    /// Minimum points to form a cluster.
    /// The default value is 2.
    pub min_points: u8,

    /// Cluster radius, in pixels.
    /// The default value is 40.0.
    pub radius: f64,

    /// Tile extent (radius is calculated relative to it).
    /// The default value is 512.0.
    pub extent: f64,

    /// Size of the KD-tree leaf node, affects performance.
    /// It cannot be 0. The default value is 64.
    pub node_size: usize,

    /// Type of coordinate system for clustering.
    /// The default value is `CoordinateSystem::LatLng`.
    pub coordinate_system: CoordinateSystem,
}

/// Supercluster configuration options builder.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize))]
pub struct SuperclusterBuilder {
    /// Minimal zoom level to generate clusters on.
    /// The default value is 0.
    pub min_zoom: Option<u8>,

    /// Maximal zoom level to cluster the points on.
    /// The default value is 16.
    pub max_zoom: Option<u8>,

    /// Minimum points to form a cluster.
    /// The default value is 2.
    pub min_points: Option<u8>,

    /// Cluster radius, in pixels.
    /// The default value is 40.0.
    pub radius: Option<f64>,

    /// Tile extent (radius is calculated relative to it).
    /// The default value is 512.0.
    pub extent: Option<f64>,

    /// Size of the KD-tree leaf node, affects performance.
    /// It cannot be 0. The default value is 64.
    pub node_size: Option<usize>,

    /// Type of coordinate system for clustering.
    /// The default value is `CoordinateSystem::LatLng`.
    pub coordinate_system: Option<CoordinateSystem>,
}

impl SuperclusterBuilder {
    /// Create a new supercluster options builder to set the options.
    ///
    /// # Returns
    ///
    /// New supercluster options builder.
    pub fn new() -> Self {
        SuperclusterBuilder::default()
    }

    /// Set the minimal zoom level to generate clusters on.
    ///
    /// # Arguments
    ///
    /// - `min_zoom`: Minimal zoom level to generate clusters on.
    ///
    /// # Returns
    ///
    /// The supercluster options builder.
    pub fn min_zoom(mut self, min_zoom: u8) -> Self {
        self.min_zoom = Some(min_zoom);
        self
    }

    /// Set the maximal zoom level to cluster the points on.
    ///
    /// # Arguments
    ///
    /// - `max_zoom`: Maximal zoom level to cluster the points on.
    ///
    /// # Returns
    ///
    /// The supercluster options builder.
    pub fn max_zoom(mut self, max_zoom: u8) -> Self {
        self.max_zoom = Some(max_zoom);
        self
    }

    /// Set the minimum points to form a cluster.
    ///
    /// # Arguments
    ///
    /// - `min_points`: Minimum points to form a cluster.
    ///
    /// # Returns
    ///
    /// The supercluster options builder.
    pub fn min_points(mut self, min_points: u8) -> Self {
        self.min_points = Some(min_points);
        self
    }

    /// Set the cluster radius in pixels.
    ///
    /// # Arguments
    ///
    /// - `radius`: Cluster radius in pixels.
    ///
    /// # Returns
    ///
    /// The supercluster options builder.
    pub fn radius(mut self, radius: f64) -> Self {
        self.radius = Some(radius);
        self
    }

    /// Set the tile extent (radius is calculated relative to it).
    ///
    /// # Arguments
    ///
    /// - `extent`: Tile extent (radius is calculated relative to it).
    ///
    /// # Returns
    ///
    /// The supercluster options builder.
    pub fn extent(mut self, extent: f64) -> Self {
        self.extent = Some(extent);
        self
    }

    /// Set the size of the KD-tree leaf node, affects performance.
    ///
    /// # Arguments
    ///
    /// - `node_size`: Size of the KD-tree leaf node, affects performance. It cannot be 0.
    ///
    /// # Returns
    ///
    /// The supercluster options builder.
    pub fn node_size(mut self, node_size: usize) -> Self {
        self.node_size = Some(node_size);
        self
    }

    /// Set the type of coordinate system for clustering.
    ///
    /// # Arguments
    ///
    /// - `coordinate_system`: Type of coordinate system for clustering.
    ///
    /// # Returns
    ///
    /// The supercluster options builder.
    pub fn coordinate_system(mut self, coordinate_system: CoordinateSystem) -> Self {
        self.coordinate_system = Some(coordinate_system);
        self
    }

    /// Build the supercluster options.
    ///
    /// # Returns
    ///
    /// The supercluster options, otherwise an error if the maximum zoom level is above
    /// `MAX_ZOOM_LIMIT`, the minimum zoom level is above the maximum zoom level or the node size is
    /// 0.
    pub fn build(self) -> Result<SuperclusterOptions, SuperclusterError> {
        let options = SuperclusterOptions {
            min_zoom: self.min_zoom.unwrap_or(0),
            max_zoom: self.max_zoom.unwrap_or(16),
            min_points: self.min_points.unwrap_or(2),
            radius: self.radius.unwrap_or(40.0),
            extent: self.extent.unwrap_or(512.0),
            node_size: self.node_size.unwrap_or(64),
            coordinate_system: self.coordinate_system.unwrap_or(CoordinateSystem::LatLng),
        };

        if options.max_zoom > MAX_ZOOM_LIMIT {
            return Err(SuperclusterError::InvalidMaxZoom);
        }

        if options.min_zoom > options.max_zoom {
            return Err(SuperclusterError::InvalidMinZoom);
        }

        if options.node_size == 0 {
            return Err(SuperclusterError::InvalidNodeSize);
        }

        Ok(options)
    }
}

#[cfg(feature = "serde")]
impl TryFrom<SuperclusterBuilder> for SuperclusterOptions {
    type Error = SuperclusterError;

    fn try_from(builder: SuperclusterBuilder) -> Result<Self, Self::Error> {
        builder.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supercluster_builder_default() {
        let options = SuperclusterBuilder::default().build().unwrap();

        assert_eq!(options.min_zoom, 0);
        assert_eq!(options.max_zoom, 16);
        assert_eq!(options.min_points, 2);
        assert_eq!(options.radius, 40.0);
        assert_eq!(options.extent, 512.0);
        assert_eq!(options.node_size, 64);
        assert_eq!(options.coordinate_system, CoordinateSystem::LatLng);
    }

    #[test]
    fn test_supercluster_builder() {
        let options = SuperclusterBuilder::new()
            .min_zoom(1)
            .max_zoom(10)
            .min_points(5)
            .radius(50.0)
            .extent(1024.0)
            .node_size(128)
            .coordinate_system(CoordinateSystem::LatLng)
            .build()
            .unwrap();

        assert_eq!(options.min_zoom, 1);
        assert_eq!(options.max_zoom, 10);
        assert_eq!(options.min_points, 5);
        assert_eq!(options.radius, 50.0);
        assert_eq!(options.extent, 1024.0);
        assert_eq!(options.node_size, 128);
        assert_eq!(options.coordinate_system, CoordinateSystem::LatLng);
    }

    #[test]
    fn test_supercluster_builder_with_invalid_max_zoom() {
        assert_eq!(
            SuperclusterBuilder::new().max_zoom(u8::MAX).build().err(),
            Some(SuperclusterError::InvalidMaxZoom)
        );
    }

    #[test]
    fn test_supercluster_builder_with_invalid_min_zoom() {
        assert_eq!(
            SuperclusterBuilder::new()
                .min_zoom(10)
                .max_zoom(5)
                .build()
                .err(),
            Some(SuperclusterError::InvalidMinZoom)
        );
    }

    #[test]
    fn test_supercluster_builder_with_invalid_node_size() {
        assert_eq!(
            SuperclusterBuilder::new().node_size(0).build().err(),
            Some(SuperclusterError::InvalidNodeSize)
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
    fn test_options_from_json() {
        let options: SuperclusterOptions = serde_json::from_str(
            r#"{
                "radius": 40.0,
                "extent": 512.0,
                "min_points": 1,
                "node_size": 64,
                "min_zoom": 2,
                "max_zoom": 16,
                "coordinate_system": "LatLng"
            }"#,
        )
        .unwrap();

        assert_eq!(options.min_zoom, 2);
        assert_eq!(options.max_zoom, 16);
        assert_eq!(options.min_points, 1);
        assert_eq!(options.coordinate_system, CoordinateSystem::LatLng);
    }
}
