//! # Supercluster module
//!
//! The `supercluster` module provides a spatial clustering library for geographic points.
//! This is the core module that contains the `Supercluster` struct and its related functionality.
//!
//! The `Supercluster` struct is used to create a spatial clustering configuration and data structure.
//! It provides methods to load input points, retrieve clusters, children, leaves, and tiles, and
//! determine the zoom level at which a specific cluster expands.
//!
//! The module also contains the `CoordinateSystem` enum, which defines the coordinate system for clustering.
//!
//! The `CoordinateSystem` enum has two variants: `LatLng` for latitude and longitude coordinates and
//! `Cartesian` for Cartesian coordinates.

use std::{collections::HashMap, f64::consts::PI, hash::BuildHasherDefault};

#[cfg(feature = "cluster_metadata")]
use geojson::JsonObject;
use geojson::{feature::Id, Feature, FeatureCollection, Geometry, Value::Point};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "cluster_metadata")]
use serde_json::json;
use twox_hash::XxHash64;

use crate::{
    DataRange, FeatureBuilder, KDBush, SuperclusterBuilder, SuperclusterError, SuperclusterOptions,
};

/// An offset index used to access the zoom level value associated with a cluster in the data arrays.
const OFFSET_ZOOM: usize = 2;

/// An offset index used to access the ID associated with a cluster in the data arrays.
const OFFSET_ID: usize = 3;

/// An offset index used to access the identifier of the parent cluster of a point in the data arrays.
const OFFSET_PARENT: usize = 4;

/// An offset index used to access the number of points contained within a cluster at the given zoom level in the data arrays.
const OFFSET_NUM: usize = 5;

/// An offset index used to access the properties associated with a cluster in the data arrays.
#[cfg(feature = "cluster_metadata")]
const OFFSET_PROP: usize = 6;

/// Coordinate system for clustering.
/// The coordinate system is used to determine the range of the incoming data.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize))]
pub enum CoordinateSystem {
    /// Latitude and longitude coordinates. Used for geo-spatial data.
    LatLng,

    /// Cartesian coordinates. Used for non-geospatial (i.e. microscopy, etc.) data.
    Cartesian {
        /// The range of the incoming data if choosing the cartesian coordinate system.
        /// Applicable for non-geospatial data (i.e. microscopy, etc.).
        range: DataRange,
    },
}

/// A spatial clustering configuration and data structure.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize))]
pub struct Supercluster {
    /// Configuration settings.
    pub options: SuperclusterOptions,

    /// Map of KD-trees for each zoom level.
    /// The key is the zoom level, and the value is the KD-tree structure.
    /// The KD-tree structure is used for spatial indexing.
    pub trees: HashMap<usize, KDBush, BuildHasherDefault<XxHash64>>,

    /// Stride used for data access within the KD-tree.
    /// The stride is the number of elements in the flat numeric arrays representing point data.
    pub stride: usize,

    /// Input data points.
    /// A vector of GeoJSON features representing input points to be clustered.
    pub points: Vec<Feature>,

    /// Clusters metadata.
    /// A vector of JSON objects representing cluster properties.
    #[cfg(feature = "cluster_metadata")]
    pub metadata: Vec<JsonObject>,
}

impl Supercluster {
    /// Create a new supercluster builder instance.
    ///
    /// # Returns
    ///
    /// New supercluster builder.
    pub fn builder() -> SuperclusterBuilder {
        SuperclusterBuilder::new()
    }

    /// Create a new feature builder instance.
    pub fn feature_builder() -> FeatureBuilder {
        FeatureBuilder::new()
    }

    /// Create a new instance of `Supercluster` with the specified configuration settings.
    ///
    /// # Arguments
    ///
    /// - `options`: The configuration options for Supercluster.
    ///
    /// # Returns
    ///
    /// New `Supercluster` instance with the given configuration.
    pub fn new(options: SuperclusterOptions) -> Self {
        #[cfg(feature = "log")]
        log::debug!("Creating a new supercluster instance");

        Supercluster {
            options,
            stride: 6,
            points: vec![],
            trees: HashMap::default(),
            #[cfg(feature = "cluster_metadata")]
            metadata: vec![],
        }
    }

    /// Load the FeatureCollection Object into the Supercluster instance, performing clustering at various zoom levels.
    /// The input points are indexed into a KD-tree for spatial indexing.
    /// The points are clustered on the maximum zoom level, and the results are clustered on previous zoom levels.
    /// This results in a cluster hierarchy across zoom levels.
    ///
    /// # Arguments
    ///
    /// - `points`: A vector of GeoJSON features representing input points to be clustered.
    ///
    /// # Returns
    ///
    /// Supercluster instance with the input points loaded and clustered.
    pub fn load(&mut self, points: Vec<Feature>) -> Result<&mut Self, SuperclusterError> {
        #[cfg(feature = "log")]
        log::debug!("Loading input {} points into supercluster", points.len());

        let min_zoom = self.options.min_zoom as usize;
        let max_zoom = self.options.max_zoom as usize;

        self.points = points;

        // Generate a cluster object for each point and index input points into a KD-tree
        let mut data = vec![];

        #[cfg(feature = "log")]
        log::debug!("Coordinate system: {:?}", self.options.coordinate_system);

        for (i, feature) in self.points.iter().enumerate() {
            // Store internal point/cluster data in flat numeric arrays for performance
            let coordinates = match &feature.geometry {
                Some(geometry) => match &geometry.value {
                    Point(coords) => coords,
                    _ => continue,
                },
                None => continue,
            };

            match &self.options.coordinate_system {
                CoordinateSystem::Cartesian { range } => {
                    // X Coordinate
                    data.push(range.normalize(coordinates[0]));

                    // Y Coordinate
                    data.push(range.normalize(coordinates[1]));
                }
                CoordinateSystem::LatLng => {
                    // Longitude
                    data.push(convert_longitude_to_spherical_mercator(coordinates[0]));

                    // Latitude
                    data.push(convert_latitude_to_spherical_mercator(coordinates[1]));
                }
            };

            // The last zoom the point was processed at
            data.push(f64::INFINITY);

            // Index of the source feature in the original input array
            data.push(i as f64);

            // Parent cluster id
            data.push(-1.0);

            // Number of points in a cluster
            data.push(1.0);
        }

        let tree = self.create_tree(data);
        self.trees.insert((max_zoom) + 1, tree);

        // Cluster points on max zoom, then cluster the results on previous zoom, etc.;
        // Results in a cluster hierarchy across zoom levels
        for zoom in (min_zoom..=max_zoom).rev() {
            let next_zoom = zoom + 1;

            // Create a new set of clusters for the zoom and index them with a KD-tree
            let (previous, current) = self.cluster(
                self.trees
                    .get(&next_zoom)
                    .ok_or(SuperclusterError::TreeNotFound)?,
                zoom,
            );

            self.trees
                .get_mut(&next_zoom)
                .ok_or(SuperclusterError::TreeNotFound)?
                .data = previous;

            let tree = self.create_tree(current);
            self.trees.insert(zoom, tree);
        }

        Ok(self)
    }

    /// Retrieve clustered features within the specified bounding box and zoom level.
    ///
    /// # Arguments
    ///
    /// - `bbox`: The bounding box as an array of four coordinates [min_lng, min_lat, max_lng, max_lat].
    /// - `zoom`: The zoom level at which to retrieve clusters.
    ///
    /// # Returns
    ///
    /// List of GeoJSON features representing the clusters within the specified bounding box and zoom level.
    pub fn get_clusters(
        &self,
        bbox: [f64; 4],
        zoom: u8,
    ) -> Result<Vec<Feature>, SuperclusterError> {
        #[cfg(feature = "log")]
        log::debug!(
            "Retrieving clusters for zoom level {} and bounding box {:?}",
            zoom,
            bbox
        );

        let tree = &self
            .trees
            .get(&self.limit_zoom(zoom))
            .ok_or(SuperclusterError::TreeNotFound)?;

        let ids = match &self.options.coordinate_system {
            CoordinateSystem::Cartesian { range } => tree.range(
                range.normalize(bbox[0]),
                range.normalize(bbox[1]),
                range.normalize(bbox[2]),
                range.normalize(bbox[3]),
            ),
            CoordinateSystem::LatLng => {
                let mut min_lng = ((((bbox[0] + 180.0) % 360.0) + 360.0) % 360.0) - 180.0;
                let min_lat = bbox[1].clamp(-90.0, 90.0);
                let mut max_lng = if bbox[2] == 180.0 {
                    180.0
                } else {
                    ((((bbox[2] + 180.0) % 360.0) + 360.0) % 360.0) - 180.0
                };
                let max_lat = bbox[3].clamp(-90.0, 90.0);

                if bbox[2] - bbox[0] >= 360.0 {
                    min_lng = -180.0;
                    max_lng = 180.0;
                } else if min_lng > max_lng {
                    let mut eastern_hem = self
                        .get_clusters([min_lng, min_lat, 180.0, max_lat], zoom)
                        .unwrap_or_default();
                    let western_hem = self
                        .get_clusters([-180.0, min_lat, max_lng, max_lat], zoom)
                        .unwrap_or_default();

                    eastern_hem.extend(western_hem);

                    return Ok(eastern_hem);
                }

                tree.range(
                    convert_longitude_to_spherical_mercator(min_lng),
                    convert_latitude_to_spherical_mercator(max_lat),
                    convert_longitude_to_spherical_mercator(max_lng),
                    convert_latitude_to_spherical_mercator(min_lat),
                )
            }
        };

        let mut clusters = vec![];

        for id in ids {
            let k = self.stride * id;

            clusters.push(if tree.data[k + OFFSET_NUM] > 1.0 {
                get_cluster(
                    &tree.data,
                    k,
                    &self.options.coordinate_system,
                    #[cfg(feature = "cluster_metadata")]
                    &self.metadata,
                )
            } else {
                self.points[tree.data[k + OFFSET_ID] as usize].to_owned()
            });
        }

        #[cfg(feature = "log")]
        log::debug!("Retrieved {} clusters", clusters.len());

        Ok(clusters)
    }

    /// Retrieve the cluster features for a specified cluster ID.
    /// The cluster ID is the unique identifier of the cluster.
    ///
    /// # Arguments
    ///
    /// - `cluster_id`: The unique identifier of the cluster.
    ///
    /// # Returns
    ///
    /// Vector of GeoJSON features representing the cluster with the specified ID.
    pub fn get_children(&self, cluster_id: usize) -> Result<Vec<Feature>, SuperclusterError> {
        let origin_id = self.get_origin_id(cluster_id);
        let origin_zoom = self.get_origin_zoom(cluster_id);
        let tree = self
            .trees
            .get(&origin_zoom)
            .ok_or(SuperclusterError::TreeNotFound)?;
        let data = &tree.data;

        if origin_id * self.stride >= data.len() {
            #[cfg(feature = "log")]
            log::error!("Cluster not found for ID {}", cluster_id);

            return Err(SuperclusterError::ClusterNotFound);
        }

        let r = self.options.radius
            / (self.options.extent * f64::powf(2.0, (origin_zoom as f64) - 1.0));

        let x = data[origin_id * self.stride];
        let y = data[origin_id * self.stride + 1];

        let ids = tree.within(x, y, r);
        let mut children = vec![];

        for id in ids {
            let k = id * self.stride;

            if data[k + OFFSET_PARENT] == (cluster_id as f64) {
                if data[k + OFFSET_NUM] > 1.0 {
                    children.push(get_cluster(
                        data,
                        k,
                        &self.options.coordinate_system,
                        #[cfg(feature = "cluster_metadata")]
                        &self.metadata,
                    ));
                } else {
                    let point_id = data[k + OFFSET_ID] as usize;
                    children.push(self.points[point_id].to_owned());
                }
            }
        }

        if children.is_empty() {
            return Err(SuperclusterError::ClusterNotFound);
        }

        Ok(children)
    }

    /// Retrieve individual leaf features within a cluster.
    /// Leaf features are the individual points within a cluster.
    ///
    /// # Arguments
    ///
    /// - `cluster_id`: The unique identifier of the cluster.
    /// - `limit`: The maximum number of leaf features to retrieve.
    /// - `offset`: The offset to start retrieving leaf features.
    ///
    /// # Returns
    ///
    /// A vector of GeoJSON features representing the individual leaf features within the cluster.
    pub fn get_leaves(&self, cluster_id: usize, limit: usize, offset: usize) -> Vec<Feature> {
        let mut leaves = vec![];
        self.append_leaves(&mut leaves, cluster_id, limit, offset, 0);

        leaves
    }

    /// Retrieve a vector of features within a tile at the given zoom level and tile coordinates.
    /// The tile is a square area of the map that is rendered as an image.
    /// The zoom level determines the scale of the map.
    /// The X and Y coordinates determine the position of the tile on the map.
    /// The tile is represented by a GeoJSON FeatureCollection.
    ///
    /// # Arguments
    ///
    /// - `z`: The zoom level of the tile.
    /// - `x`: The X coordinate of the tile.
    /// - `y`: The Y coordinate of the tile.
    ///
    /// # Returns
    ///
    /// A list of GeoJSON features within the specified tile, otherwise an error if the tile is not found.
    pub fn get_tile(&self, z: u8, x: f64, y: f64) -> Result<FeatureCollection, SuperclusterError> {
        let zoom = self.limit_zoom(z);
        let tree = match self.trees.get(&zoom) {
            Some(tree) => tree,
            None => {
                #[cfg(feature = "log")]
                log::error!("Tree not found for zoom level {}", z);

                return Err(SuperclusterError::TreeNotFound);
            }
        };
        let z2: f64 = (2u32).pow(z as u32) as f64;
        let p = self.options.radius / self.options.extent;
        let top = (y - p) / z2;
        let bottom = (y + 1.0 + p) / z2;

        let mut tile = FeatureCollection {
            bbox: None,
            foreign_members: None,
            features: vec![],
        };

        let ids = tree.range((x - p) / z2, top, (x + 1.0 + p) / z2, bottom);
        self.add_tile_features(&ids, &tree.data, x, y, z2, &mut tile);

        if x == 0.0 {
            let ids = tree.range(1.0 - p / z2, top, 1.0, bottom);
            self.add_tile_features(&ids, &tree.data, z2, y, z2, &mut tile);
        }

        if x == z2 - 1.0 {
            let ids = tree.range(0.0, top, p / z2, bottom);
            self.add_tile_features(&ids, &tree.data, -1.0, y, z2, &mut tile);
        }

        if tile.features.is_empty() {
            #[cfg(feature = "log")]
            log::error!("Tile not found for zoom level {}, x: {}, y: {}", z, x, y);

            return Err(SuperclusterError::TileNotFound);
        }

        #[cfg(feature = "log")]
        log::debug!(
            "Retrieved {} features for tile at zoom level {}, x: {}, y: {}",
            tile.features.len(),
            z,
            x,
            y
        );

        Ok(tile)
    }

    /// Determine the zoom level at which a specific cluster expands.
    /// The cluster expands when it contains more than one child cluster.
    /// The cluster expands until it reaches the maximum zoom level or contains more than one child cluster.
    /// The zoom level at which the cluster expands is the zoom level at which the cluster contains more than one child cluster.
    /// The cluster does not expand if it contains only one child cluster.
    ///
    /// # Arguments
    ///
    /// - `cluster_id`: The unique identifier of the cluster.
    ///
    /// # Returns
    ///
    /// The zoom level at which the cluster expands.
    pub fn get_cluster_expansion_zoom(&self, mut cluster_id: usize) -> usize {
        let mut expansion_zoom = self.get_origin_zoom(cluster_id) - 1;

        while expansion_zoom <= (self.options.max_zoom as usize) {
            let children = match self.get_children(cluster_id) {
                Ok(children) => children,
                Err(_) => break,
            };

            expansion_zoom += 1;

            if children.len() != 1 {
                break;
            }

            cluster_id = match children[0].property("cluster_id") {
                Some(property) => match property.as_u64() {
                    Some(id) => id as usize,
                    None => break,
                },
                None => break,
            };
        }

        expansion_zoom
    }

    /// Appends leaves (features) to the result vector based on the specified criteria.
    /// The method is used to collect leaves within a cluster based on the specified limit and offset.
    /// The method is called recursively to collect leaves within child clusters.
    /// The method is used internally by the `get_leaves` method.
    ///
    /// # Arguments
    ///
    /// - `result`: A mutable reference to a vector where leaves will be appended.
    /// - `cluster_id`: The identifier of the cluster whose leaves are being collected.
    /// - `limit`: The maximum number of leaves to collect.
    /// - `offset`: The number of leaves to skip before starting to collect.
    /// - `skipped`: The current count of skipped leaves, used for tracking the progress.
    ///
    /// # Returns
    ///
    /// The updated count of skipped leaves after processing the current cluster.
    pub fn append_leaves(
        &self,
        result: &mut Vec<Feature>,
        cluster_id: usize,
        limit: usize,
        offset: usize,
        mut skipped: usize,
    ) -> usize {
        let cluster = match self.get_children(cluster_id) {
            Ok(cluster) => cluster,
            Err(_) => return skipped,
        };

        for child in cluster {
            if child.contains_property("cluster") {
                if let Some(point_count) = child.property("point_count").and_then(|p| p.as_i64()) {
                    if skipped + point_count as usize <= offset {
                        // Skip the whole cluster
                        skipped += point_count as usize;
                    } else {
                        // Enter the cluster
                        if let Some(cluster_id) =
                            child.property("cluster_id").and_then(|c| c.as_u64())
                        {
                            skipped = self.append_leaves(
                                result,
                                cluster_id as usize,
                                limit,
                                offset,
                                skipped,
                            );
                        }
                        // Exit the cluster
                    }
                }
            } else if skipped < offset {
                // Skip a single point
                skipped += 1;
            } else {
                // Add a single point
                result.push(child);
            }

            if result.len() == limit {
                break;
            }
        }

        skipped
    }

    /// Create a KD-tree using the specified data, which is used for spatial indexing.
    /// The KD-tree is used to index the input points for clustering.
    /// The KD-tree is created for the specified data and configured with the specified node size.
    /// The KD-tree is used to cluster points based on the specified node size.
    ///
    /// # Arguments
    ///
    /// - `data`: A vector of flat numeric arrays representing point data for the KD-tree.
    ///
    /// # Returns
    ///
    /// `KDBush` instance with the specified data.
    pub fn create_tree(&mut self, data: Vec<f64>) -> KDBush {
        let mut tree = KDBush::new(data.len() / self.stride, self.options.node_size);

        for i in (0..data.len()).step_by(self.stride) {
            tree.add_point(data[i], data[i + 1]);
        }

        tree.build_index();
        tree.data = data;

        tree
    }

    /// Populate a tile with features based on the specified point IDs, data, and tile parameters.
    /// The method is used to populate a tile with features based on the specified point IDs and data.
    ///
    /// # Arguments
    ///
    /// - `ids`: A vector of point IDs used for populating the tile.
    /// - `data`: A reference to the flat numeric arrays representing point data.
    /// - `x`: The X coordinate of the tile.
    /// - `y`: The Y coordinate of the tile.
    /// - `z2`: The zoom level multiplied by 2.
    /// - `tile`: A mutable reference to the `FeatureCollection` to be populated with features.
    pub fn add_tile_features(
        &self,
        ids: &Vec<usize>,
        data: &[f64],
        x: f64,
        y: f64,
        z2: f64,
        tile: &mut FeatureCollection,
    ) {
        for i in ids {
            let k = i * self.stride;
            let is_cluster = data[k + OFFSET_NUM] > 1.0;

            let cluster = if is_cluster {
                (
                    data[k],
                    data[k + 1],
                    #[cfg(feature = "cluster_metadata")]
                    get_cluster_metadata(data, k, &self.metadata),
                )
            } else {
                let p = &self.points[data[k + OFFSET_ID] as usize];

                #[cfg(feature = "cluster_metadata")]
                let properties = match p.properties.as_ref() {
                    Some(properties) => properties.to_owned(),
                    None => continue, // Handle the case where properties is None
                };

                let (px, py) = match p.geometry.as_ref() {
                    Some(geometry) => {
                        if let Point(coordinates) = &geometry.value {
                            match &self.options.coordinate_system {
                                CoordinateSystem::Cartesian { range } => (
                                    range.normalize(coordinates[0]),
                                    range.normalize(coordinates[1]),
                                ),
                                CoordinateSystem::LatLng => (
                                    convert_longitude_to_spherical_mercator(coordinates[0]),
                                    convert_latitude_to_spherical_mercator(coordinates[1]),
                                ),
                            }
                        } else {
                            continue;
                        }
                    }
                    None => continue, // Handle the case where geometry is None
                };

                (
                    px,
                    py,
                    #[cfg(feature = "cluster_metadata")]
                    properties,
                )
            };

            let id = if is_cluster {
                Some(Id::String(data[k + OFFSET_ID].to_string()))
            } else {
                self.points[data[k + OFFSET_ID] as usize].id.to_owned()
            };

            let geometry = Geometry::new(Point(vec![
                (self.options.extent * (cluster.0 * z2 - x)).round(),
                (self.options.extent * (cluster.1 * z2 - y)).round(),
            ]));

            tile.features.push(Feature {
                id,
                bbox: None,
                foreign_members: None,
                geometry: Some(geometry),
                #[cfg(feature = "cluster_metadata")]
                properties: Some(cluster.2),
                #[cfg(not(feature = "cluster_metadata"))]
                properties: None,
            });
        }
    }

    /// Calculate the effective zoom level that takes into account the configured minimum and maximum zoom levels.
    /// The effective zoom level is the zoom level that is within the configured minimum and maximum zoom levels.
    ///
    /// # Arguments
    ///
    /// - `zoom`: The initial zoom level.
    ///
    /// # Returns
    ///
    /// The effective zoom level considering the configured minimum and maximum zoom levels.
    pub fn limit_zoom(&self, zoom: u8) -> usize {
        #[cfg(feature = "log")]
        log::debug!("Limiting zoom level to {}", zoom);

        zoom.max(self.options.min_zoom)
            .min(self.options.max_zoom + 1) as usize
    }

    /// Cluster points on a given zoom level using a KD-tree and returns updated data arrays.
    ///
    /// # Arguments
    ///
    /// - `tree`: A reference to the KD-tree structure for spatial indexing.
    /// - `zoom`: The zoom level at which clustering is performed.
    ///
    /// # Returns
    ///
    /// A tuple of two vectors: the first one contains updated data arrays for the current zoom level,
    /// and the second one contains data arrays for the next zoom level.
    pub fn cluster(&self, tree: &KDBush, zoom: usize) -> (Vec<f64>, Vec<f64>) {
        let r = self.options.radius / (self.options.extent * (2.0_f64).powi(zoom as i32));

        #[cfg(feature = "log")]
        log::debug!("Clustering points at zoom level {}", zoom);

        let mut data = tree.data.to_owned();
        let mut next_data = vec![];

        // Loop through each point
        for i in (0..data.len()).step_by(self.stride) {
            // If we've already visited the point at this zoom level, skip it
            if data[i + OFFSET_ZOOM] <= (zoom as f64) {
                continue;
            }

            data[i + OFFSET_ZOOM] = zoom as f64;

            // Find all nearby points
            let x = data[i];
            let y = data[i + 1];

            let neighbor_ids = tree.within(x, y, r);

            let num_points_origin = data[i + OFFSET_NUM];
            let mut num_points = num_points_origin;

            // Count the number of points in a potential cluster
            for neighbor_id in &neighbor_ids {
                let k = neighbor_id * self.stride;

                // Filter out neighbors that are already processed
                if data[k + OFFSET_ZOOM] > (zoom as f64) {
                    num_points += data[k + OFFSET_NUM];
                }
            }

            // If there were neighbors to merge, and there are enough points to form a cluster
            if num_points > num_points_origin && num_points >= (self.options.min_points as f64) {
                let mut wx = x * num_points_origin;
                let mut wy = y * num_points_origin;

                // Encode both zoom and point index on which the cluster originated -- offset by total length of features
                let id = ((i / self.stride) << 5) + (zoom + 1) + self.points.len();

                for neighbor_id in neighbor_ids {
                    let k = neighbor_id * self.stride;

                    if data[k + OFFSET_ZOOM] <= (zoom as f64) {
                        continue;
                    }

                    // Save the zoom (so it doesn't get processed twice)
                    data[k + OFFSET_ZOOM] = zoom as f64;

                    let num_points2 = data[k + OFFSET_NUM];

                    // Accumulate coordinates for calculating weighted center
                    wx += data[k] * num_points2;
                    wy += data[k + 1] * num_points2;

                    data[k + OFFSET_PARENT] = id as f64;
                }

                data[i + OFFSET_PARENT] = id as f64;

                next_data.push(wx / num_points);
                next_data.push(wy / num_points);
                next_data.push(f64::INFINITY);
                next_data.push(id as f64);
                next_data.push(-1.0);
                next_data.push(num_points);
            } else {
                // Left points as unclustered
                for j in 0..self.stride {
                    next_data.push(data[i + j]);
                }

                if num_points > 1.0 {
                    for neighbor_id in neighbor_ids {
                        let k = neighbor_id * self.stride;

                        if data[k + OFFSET_ZOOM] <= (zoom as f64) {
                            continue;
                        }

                        data[k + OFFSET_ZOOM] = zoom as f64;

                        for j in 0..self.stride {
                            next_data.push(data[k + j]);
                        }
                    }
                }
            }
        }

        (data, next_data)
    }

    /// Get the index of the point from which the cluster originated.
    ///
    /// # Arguments
    ///
    /// - `cluster_id`: The unique identifier of the cluster.
    ///
    /// # Returns
    ///
    /// The index of the point from which the cluster originated.
    pub fn get_origin_id(&self, cluster_id: usize) -> usize {
        (cluster_id - self.points.len()) >> 5
    }

    /// Get the zoom of the point from which the cluster originated.
    /// The zoom level is encoded in the cluster ID.
    ///
    /// # Arguments
    ///
    /// - `cluster_id`: The unique identifier of the cluster.
    ///
    /// # Returns
    ///
    /// The zoom level of the point from which the cluster originated.
    pub fn get_origin_zoom(&self, cluster_id: usize) -> usize {
        (cluster_id - self.points.len()) % 32
    }
}

/// Convert clustered point data into a GeoJSON feature representing a cluster.
///
/// # Arguments
///
/// - `data`: A reference to the flat numeric arrays representing point data.
/// - `i`: The index in the data array for the cluster.
/// - `coordinate_system`: The coordinate system used for clustering.
/// - `metadata`: The cluster metadata.
///
/// # Returns
///
/// A GeoJSON feature representing a cluster.
fn get_cluster(
    data: &[f64],
    i: usize,
    coordinate_system: &CoordinateSystem,
    #[cfg(feature = "cluster_metadata")] metadata: &[JsonObject],
) -> Feature {
    let geometry = match coordinate_system {
        CoordinateSystem::Cartesian { range } => Geometry::new(Point(vec![
            range.denormalize(data[i]),
            range.denormalize(data[i + 1]),
        ])),
        CoordinateSystem::LatLng => Geometry::new(Point(vec![
            convert_spherical_mercator_to_longitude(data[i]),
            convert_spherical_mercator_to_latitude(data[i + 1]),
        ])),
    };

    Feature {
        id: Some(Id::String(data[i + OFFSET_ID].to_string())),
        bbox: None,
        foreign_members: None,
        geometry: Some(geometry),
        #[cfg(feature = "cluster_metadata")]
        properties: Some(get_cluster_metadata(data, i, metadata)),
        #[cfg(not(feature = "cluster_metadata"))]
        properties: None,
    }
}

/// Retrieve metadata for a cluster based on clustered point data.
///
/// # Arguments
///
/// - `data`: A reference to the flat numeric arrays representing point data.
/// - `i`: The index in the data array for the cluster.
/// - `metadata`: The cluster metadata.
///
/// # Returns
///
/// Metadata for the cluster based on the clustered point data.
#[cfg(feature = "cluster_metadata")]
fn get_cluster_metadata(data: &[f64], i: usize, metadata: &[JsonObject]) -> JsonObject {
    let count = data[i + OFFSET_NUM];
    let abbrev = if count >= 10000.0 {
        format!("{}k", count / 1000.0)
    } else if count >= 1000.0 {
        format!("{:}k", count / 100.0 / 10.0)
    } else {
        count.to_string()
    };

    let mut properties = if !metadata.is_empty() && data.get(i + OFFSET_PROP).is_some() {
        metadata[data[i + OFFSET_PROP] as usize].to_owned()
    } else {
        JsonObject::new()
    };

    properties.insert("cluster".to_string(), json!(true));
    properties.insert(
        "cluster_id".to_string(),
        json!(data[i + OFFSET_ID] as usize),
    );
    properties.insert("point_count".to_string(), json!(count as usize));
    properties.insert("point_count_abbreviated".to_string(), json!(abbrev));

    properties
}

/// Convert longitude to spherical mercator in the [0..1] range.
///
/// # Arguments
///
/// - `lng`: The longitude value to be converted.
///
/// # Returns
///
/// The converted value in the [0..1] range.
fn convert_longitude_to_spherical_mercator(lng: f64) -> f64 {
    lng / 360.0 + 0.5
}

/// Convert latitude to spherical mercator in the [0..1] range.
///
/// # Arguments
///
/// - `lat`: The latitude value to be converted.
///
/// # Returns
///
/// The converted value in the [0..1] range.
fn convert_latitude_to_spherical_mercator(lat: f64) -> f64 {
    let sin = lat.to_radians().sin();
    let y = 0.5 - (0.25 * ((1.0 + sin) / (1.0 - sin)).ln()) / PI;

    y.clamp(0.0, 1.0)
}

/// Convert spherical mercator to longitude.
///
/// # Arguments
///
/// - `x`: The spherical mercator value to be converted.
///
/// # Returns
///
/// The converted longitude value.
fn convert_spherical_mercator_to_longitude(x: f64) -> f64 {
    (x - 0.5) * 360.0
}

/// Convert spherical mercator to latitude.
///
/// # Arguments
///
/// - `y`: The spherical mercator value to be converted.
///
/// # Returns
///
/// The converted latitude value.
fn convert_spherical_mercator_to_latitude(y: f64) -> f64 {
    let y2 = ((180.0 - y * 360.0) * PI) / 180.0;
    (360.0 * y2.exp().atan()) / PI - 90.0
}

#[cfg(test)]
mod tests {
    use super::*;

    use geojson::JsonObject;

    fn setup() -> Supercluster {
        let options = Supercluster::builder().build();
        Supercluster::new(options)
    }

    #[test]
    fn test_builder() {
        let options = Supercluster::builder().build();
        let supercluster = Supercluster::new(options);

        assert_eq!(supercluster.options.min_zoom, 0);
        assert_eq!(supercluster.options.max_zoom, 16);
        assert_eq!(supercluster.options.radius, 40.0);
        assert_eq!(supercluster.options.extent, 512.0);
        assert_eq!(supercluster.options.node_size, 64);
        assert_eq!(supercluster.options.min_points, 2);
        assert_eq!(
            supercluster.options.coordinate_system,
            CoordinateSystem::LatLng
        );
    }

    #[test]
    fn test_feature_builder() {
        let features = Supercluster::feature_builder()
            .add_point(vec![0.0, 0.0])
            .build();
        let feature = features.first().unwrap();

        assert_eq!(feature.id, Some(Id::String("0".to_string())));
        assert_eq!(feature.geometry, Some(Geometry::new(Point(vec![0.0, 0.0]))));
    }

    #[test]
    fn test_limit_zoom() {
        let supercluster = setup();

        assert_eq!(supercluster.limit_zoom(5), 5);
    }

    #[test]
    fn test_get_origin_id() {
        let supercluster = setup();

        assert_eq!(supercluster.get_origin_id(100), 3);
    }

    #[test]
    fn test_get_origin_zoom() {
        let supercluster = setup();

        assert_eq!(supercluster.get_origin_zoom(100), 4);
    }

    #[test]
    #[cfg(feature = "cluster_metadata")]
    fn test_get_cluster_with_metadata() {
        let data = [0.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0];
        let i = 0;
        let mut metadata = JsonObject::new();

        metadata.insert("cluster".to_string(), serde_json::json!(false));
        metadata.insert("cluster_id".to_string(), serde_json::json!(0));
        metadata.insert("point_count".to_string(), serde_json::json!(0));
        metadata.insert("name".to_string(), serde_json::json!("name".to_string()));
        metadata.insert(
            "point_count_abbreviated".to_string(),
            serde_json::json!("0".to_string()),
        );

        let result = get_cluster(&data, i, &CoordinateSystem::LatLng, &[metadata]);

        assert_eq!(result.id, Some(Id::String("0".to_string())));

        assert!(result.property("cluster").unwrap().as_bool().unwrap());
        assert_eq!(result.property("cluster_id").unwrap().as_i64().unwrap(), 0);
        assert_eq!(result.property("point_count").unwrap().as_i64().unwrap(), 3);
        assert_eq!(
            result.property("name").unwrap().as_str().unwrap(),
            "name".to_string()
        );
        assert_eq!(
            result
                .property("point_count_abbreviated")
                .unwrap()
                .as_str()
                .unwrap(),
            "3".to_string()
        );

        let coordinates = match result.geometry {
            Some(geometry) => match geometry.value {
                Point(coords) => coords,
                _ => vec![],
            },
            None => vec![],
        };

        assert_eq!(coordinates, vec![-180.0, 85.05112877980659]);
    }

    #[test]
    #[cfg(feature = "cluster_metadata")]
    fn test_get_cluster_without_metadata() {
        let data = [0.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0];
        let i = 0;
        let metadata = vec![];

        let result = get_cluster(&data, i, &CoordinateSystem::LatLng, &metadata);

        assert_eq!(result.id, Some(Id::String("0".to_string())));

        assert!(result
            .property("cluster")
            .as_ref()
            .unwrap()
            .as_bool()
            .unwrap());
        assert_eq!(result.property("cluster_id").unwrap().as_i64().unwrap(), 0);
        assert_eq!(result.property("point_count").unwrap().as_i64().unwrap(), 3);
        assert!(result.property("name").is_none());
        assert_eq!(
            result
                .property("point_count_abbreviated")
                .unwrap()
                .as_str()
                .unwrap(),
            "3".to_string()
        );

        let coordinates = match result.geometry {
            Some(geometry) => match geometry.value {
                Point(coords) => coords,
                _ => vec![],
            },
            None => vec![],
        };

        assert_eq!(coordinates, vec![-180.0, 85.05112877980659]);
    }

    #[test]
    #[cfg(feature = "cluster_metadata")]
    fn test_get_cluster_metadata_with_metadata() {
        let data = [0.0, 0.0, 0.0, 0.0, 0.0, 10000.0, 0.0];
        let i = 0;
        let mut metadata = JsonObject::new();

        metadata.insert("cluster".to_string(), serde_json::json!(false));
        metadata.insert("cluster_id".to_string(), serde_json::json!(0));
        metadata.insert("point_count".to_string(), serde_json::json!(0));
        metadata.insert("name".to_string(), serde_json::json!("name".to_string()));
        metadata.insert(
            "point_count_abbreviated".to_string(),
            serde_json::json!("0".to_string()),
        );

        let result = get_cluster_metadata(&data, i, &[metadata]);

        assert!(result.get("cluster").unwrap().as_bool().unwrap());
        assert_eq!(result.get("cluster_id").unwrap().as_i64().unwrap(), 0);
        assert_eq!(result.get("point_count").unwrap().as_i64().unwrap(), 10000);
        assert_eq!(
            result.get("name").unwrap().as_str().unwrap(),
            "name".to_string()
        );
        assert_eq!(
            result
                .get("point_count_abbreviated")
                .unwrap()
                .as_str()
                .unwrap(),
            "10k".to_string()
        );
    }

    #[test]
    #[cfg(feature = "cluster_metadata")]
    fn test_get_cluster_metadata_without_metadata() {
        let data = [0.0, 0.0, 0.0, 0.0, 0.0, 1000.0, 0.0];
        let i = 0;
        let metadata = vec![];

        let result = get_cluster_metadata(&data, i, &metadata);

        assert!(result.get("cluster").unwrap().as_bool().unwrap());
        assert_eq!(result.get("cluster_id").unwrap().as_i64().unwrap(), 0);
        assert_eq!(result.get("point_count").unwrap().as_i64().unwrap(), 1000);
        assert!(result.get("name").is_none());
        assert_eq!(
            result
                .get("point_count_abbreviated")
                .unwrap()
                .as_str()
                .unwrap(),
            "1k".to_string()
        );
    }

    #[test]
    fn test_convert_longitude_to_spherical_mercator() {
        assert_eq!(convert_longitude_to_spherical_mercator(0.0), 0.5);
        assert_eq!(convert_longitude_to_spherical_mercator(180.0), 1.0);
        assert_eq!(convert_longitude_to_spherical_mercator(-180.0), 0.0);
        assert_eq!(convert_longitude_to_spherical_mercator(90.0), 0.75);
        assert_eq!(convert_longitude_to_spherical_mercator(-90.0), 0.25);
    }

    #[test]
    fn test_convert_latitude_to_spherical_mercator() {
        assert_eq!(convert_latitude_to_spherical_mercator(0.0), 0.5);
        assert_eq!(convert_latitude_to_spherical_mercator(90.0), 0.0);
        assert_eq!(convert_latitude_to_spherical_mercator(-90.0), 1.0);
        assert_eq!(
            convert_latitude_to_spherical_mercator(45.0),
            0.35972503691520497
        );
        assert_eq!(
            convert_latitude_to_spherical_mercator(-45.0),
            0.640274963084795
        );
    }

    #[test]
    fn test_convert_spherical_mercator_to_longitude() {
        assert_eq!(convert_spherical_mercator_to_longitude(0.5), 0.0);
        assert_eq!(convert_spherical_mercator_to_longitude(1.0), 180.0);
        assert_eq!(convert_spherical_mercator_to_longitude(0.0), -180.0);
        assert_eq!(convert_spherical_mercator_to_longitude(0.75), 90.0);
        assert_eq!(convert_spherical_mercator_to_longitude(0.25), -90.0);
    }

    #[test]
    fn test_convert_spherical_mercator_to_latitude() {
        assert_eq!(convert_spherical_mercator_to_latitude(0.5), 0.0);
        assert_eq!(
            convert_spherical_mercator_to_latitude(0.875),
            -79.17133464081944
        );
        assert_eq!(
            convert_spherical_mercator_to_latitude(0.125),
            79.17133464081945
        );
    }
}
