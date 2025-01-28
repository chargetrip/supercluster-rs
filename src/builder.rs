use serde::{Deserialize, Serialize};

use crate::CoordinateSystem;

/// Supercluster configuration options.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Options {
    /// Minimal zoom level to generate clusters on.
    pub min_zoom: u8,

    /// Maximal zoom level to cluster the points on.
    pub max_zoom: u8,

    /// Minimum points to form a cluster.
    pub min_points: u8,

    /// Cluster radius in pixels.
    pub radius: f64,

    /// Tile extent (radius is calculated relative to it).
    pub extent: f64,

    /// Size of the KD-tree leaf node, affects performance.
    pub node_size: usize,

    /// Type of coordinate system for clustering.
    pub coordinate_system: CoordinateSystem,
}

/// Supercluster configuration options builder.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct SuperclusterBuilder {
    /// Minimal zoom level to generate clusters on.
    min_zoom: Option<u8>,

    /// Maximal zoom level to cluster the points on.
    max_zoom: Option<u8>,

    /// Minimum points to form a cluster.
    min_points: Option<u8>,

    /// Cluster radius in pixels.
    radius: Option<f64>,

    /// Tile extent (radius is calculated relative to it).
    extent: Option<f64>,

    /// Size of the KD-tree leaf node, affects performance.
    node_size: Option<usize>,

    /// Type of coordinate system for clustering.
    coordinate_system: Option<CoordinateSystem>,
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
    /// - `node_size`: Size of the KD-tree leaf node, affects performance.
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
    /// The supercluster options.
    pub fn build(self) -> Options {
        Options {
            min_zoom: self.min_zoom.unwrap_or(0),
            max_zoom: self.max_zoom.unwrap_or(16),
            min_points: self.min_points.unwrap_or(2),
            radius: self.radius.unwrap_or(40.0),
            extent: self.extent.unwrap_or(512.0),
            node_size: self.node_size.unwrap_or(64),
            coordinate_system: self.coordinate_system.unwrap_or(CoordinateSystem::LatLng),
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_supercluster_builder_default() {
        let options = SuperclusterBuilder::default().build();

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
            .build();

        assert_eq!(options.min_zoom, 1);
        assert_eq!(options.max_zoom, 10);
        assert_eq!(options.min_points, 5);
        assert_eq!(options.radius, 50.0);
        assert_eq!(options.extent, 1024.0);
        assert_eq!(options.node_size, 128);
        assert_eq!(options.coordinate_system, CoordinateSystem::LatLng);
    }
}
