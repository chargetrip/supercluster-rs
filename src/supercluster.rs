//! # Supercluster module
//!
//! The `supercluster` module provides a spatial clustering library for points.
//! This is the core module that contains the `Supercluster` struct and its related functionality.
//!
//! The `Supercluster` struct is generic over the type of the points. Any type can be clustered, as
//! long as a closure tells where a point is when the points are loaded.
//! It provides methods to load input points, retrieve clusters, children, leaves, and tiles, and
//! determine the zoom level at which a specific cluster expands.
//!
//! The module also contains the `CoordinateSystem` enum, which defines the coordinate system for clustering.
//!
//! The `CoordinateSystem` enum has two variants: `LatLng` for latitude and longitude coordinates and
//! `Cartesian` for Cartesian coordinates.

use std::{cmp::Ordering, collections::HashMap, f64::consts::PI, hash::BuildHasherDefault};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use twox_hash::XxHash64;

use crate::{kdbush::KDBush, DataRange, SuperclusterError, SuperclusterOptions};

/// The number of values stored per point or cluster in the data arrays.
const STRIDE: usize = 6;

/// An offset index used to access the zoom level value associated with a cluster in the data arrays.
const OFFSET_ZOOM: usize = 2;

/// An offset index used to access the ID associated with a cluster in the data arrays.
const OFFSET_ID: usize = 3;

/// An offset index used to access the identifier of the parent cluster of a point in the data arrays.
const OFFSET_PARENT: usize = 4;

/// An offset index used to access the number of points contained within a cluster at the given zoom level in the data arrays.
const OFFSET_NUM: usize = 5;

/// The zoom level of a point or cluster that no zoom level has processed yet. It is the largest
/// finite number, so a supercluster can be written as JSON, which has no infinity.
const NOT_PROCESSED: f64 = f64::MAX;

/// The highest maximum zoom level a supercluster supports, because a cluster ID holds the zoom
/// level in 5 bits. Loading points into a supercluster with a higher `max_zoom` is an error.
pub const MAX_ZOOM_LIMIT: u8 = 30;

/// Start and length in the leaves of a supercluster, per cluster ID.
type LeafRanges = HashMap<usize, (u32, u32), BuildHasherDefault<XxHash64>>;

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

/// A cluster of points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Cluster {
    /// The unique identifier of the cluster.
    pub id: usize,

    /// The number of points in the cluster.
    pub point_count: usize,

    /// The X coordinate of the center of the cluster, in the coordinate system of the supercluster.
    /// This is the longitude for the `LatLng` coordinate system.
    pub x: f64,

    /// The Y coordinate of the center of the cluster, in the coordinate system of the supercluster.
    /// This is the latitude for the `LatLng` coordinate system.
    pub y: f64,
}

impl Cluster {
    /// Get the number of points as a short text, for example `1.5k` for 1500 points.
    ///
    /// # Returns
    ///
    /// The number of points, abbreviated from 1000 points on.
    pub fn point_count_abbreviated(&self) -> String {
        let count = self.point_count as f64;

        if count >= 10000.0 {
            format!("{}k", count / 1000.0)
        } else if count >= 1000.0 {
            format!("{}k", count / 100.0 / 10.0)
        } else {
            self.point_count.to_string()
        }
    }
}

/// A cluster, or a point that is not part of a cluster at the queried zoom level.
#[derive(Debug, PartialEq)]
pub enum Item<'a, P> {
    /// A cluster of points.
    Cluster(Cluster),

    /// A point that is not clustered.
    Point {
        /// The index of the point in the list that was loaded.
        index: usize,

        /// The point.
        point: &'a P,
    },
}

// Written by hand, because a derive would also require the point type to be `Clone` and `Copy`,
// and an item only holds a reference to a point.
impl<P> Clone for Item<'_, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Copy for Item<'_, P> {}

/// An item of a tile with its position in the tile.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub struct TileItem<'a, P> {
    /// The X position in the tile, in the units of the tile extent.
    pub x: f64,

    /// The Y position in the tile, in the units of the tile extent.
    pub y: f64,

    /// The cluster or point at the position.
    pub item: Item<'a, P>,
}

impl<P> Clone for TileItem<'_, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Copy for TileItem<'_, P> {}

/// The items within a tile.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub struct Tile<'a, P> {
    /// The clusters and points of the tile, which also holds the ones of the buffer around it.
    pub items: Vec<TileItem<'a, P>>,
}

impl<P> Clone for Tile<'_, P> {
    fn clone(&self) -> Self {
        Tile {
            items: self.items.clone(),
        }
    }
}

impl<P> Tile<'_, P> {
    /// Check whether the tile has no items.
    ///
    /// # Returns
    ///
    /// `true` if the tile has no items, otherwise `false`.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// A spatial clustering configuration and data structure.
///
/// The points are kept as they are loaded, so a point type that shares its data, such as an
/// `Arc`, is not copied.
///
/// A supercluster is `Send` and `Sync` when its points are, so queries can run on many threads at
/// once. Loading is CPU-heavy and blocks the thread it runs on, so an application with an async
/// runtime should run it on a thread of its own. Dropping a large supercluster is slow too, so drop
/// it away from threads that serve requests.
///
/// With the `serde` feature, a supercluster can be written and read back. The data is not checked
/// when it is read, so data that a supercluster did not write can make queries panic.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize))]
pub struct Supercluster<P> {
    /// Configuration settings.
    options: SuperclusterOptions,

    /// KD-trees for each zoom level, from 0 to the maximum zoom level plus one.
    /// The last one holds the points that are not clustered. The ones below the minimum zoom level
    /// are empty.
    trees: Vec<KDBush>,

    /// Input points.
    points: Vec<P>,

    /// The indexes of the points of every cluster, one cluster after the other.
    /// The points of a cluster are next to each other, which makes a range enough to find them.
    leaves: Vec<u32>,

    /// The range of every cluster in the leaves.
    leaf_ranges: LeafRanges,
}

impl<P> Supercluster<P> {
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
        let trees = (0..options.max_zoom as usize + 2)
            .map(|_| KDBush::new(0, options.node_size))
            .collect();

        Supercluster {
            options,
            trees,
            points: vec![],
            leaves: vec![],
            leaf_ranges: HashMap::default(),
        }
    }

    /// Get the configuration settings.
    ///
    /// # Returns
    ///
    /// The configuration options of the supercluster.
    pub fn options(&self) -> &SuperclusterOptions {
        &self.options
    }

    /// Get the points that were loaded.
    ///
    /// # Returns
    ///
    /// The points, in the order they were loaded.
    pub fn points(&self) -> &[P] {
        &self.points
    }

    /// Retrieve clustered items within the specified bounding box and zoom level.
    ///
    /// # Arguments
    ///
    /// - `bbox`: The bounding box as an array of four coordinates `[min_lng, min_lat, max_lng, max_lat]`,
    ///   or `[min_x, min_y, max_x, max_y]` for the `Cartesian` coordinate system.
    /// - `zoom`: The zoom level at which to retrieve clusters.
    ///
    /// # Returns
    ///
    /// List of the clusters and the points that are not clustered within the specified bounding box and zoom level.
    pub fn get_clusters(&self, bbox: [f64; 4], zoom: u8) -> Vec<Item<'_, P>> {
        let tree = &self.trees[self.tree_index(zoom, self.options.max_zoom)];

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
                    let mut eastern_hem =
                        self.get_clusters([min_lng, min_lat, 180.0, max_lat], zoom);
                    let western_hem = self.get_clusters([-180.0, min_lat, max_lng, max_lat], zoom);

                    eastern_hem.extend(western_hem);

                    return eastern_hem;
                }

                tree.range(
                    convert_longitude_to_spherical_mercator(min_lng),
                    convert_latitude_to_spherical_mercator(max_lat),
                    convert_longitude_to_spherical_mercator(max_lng),
                    convert_latitude_to_spherical_mercator(min_lat),
                )
            }
        };

        ids.into_iter()
            .map(|id| self.get_item(&tree.data, id * STRIDE))
            .collect()
    }

    /// Retrieve the children of a cluster.
    /// The cluster ID is the unique identifier of the cluster.
    ///
    /// # Arguments
    ///
    /// - `cluster_id`: The unique identifier of the cluster.
    ///
    /// # Returns
    ///
    /// The clusters and points the cluster is made of, otherwise an error if there is no such cluster.
    pub fn get_children(&self, cluster_id: usize) -> Result<Vec<Item<'_, P>>, SuperclusterError> {
        let (zoom, offsets) = self.find_children(cluster_id)?;
        let data = &self.trees[zoom].data;

        Ok(offsets
            .into_iter()
            .map(|offset| self.get_item(data, offset))
            .collect())
    }

    /// Retrieve the individual points within a cluster.
    /// The points of a cluster always come in the same order.
    ///
    /// # Arguments
    ///
    /// - `cluster_id`: The unique identifier of the cluster.
    /// - `limit`: The maximum number of points to retrieve.
    /// - `offset`: The number of points to skip before the first one to retrieve.
    ///
    /// # Returns
    ///
    /// The points within the cluster, otherwise an error if there is no such cluster.
    pub fn get_leaves(
        &self,
        cluster_id: usize,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<&P>, SuperclusterError> {
        Ok(self
            .iter_leaf_indices(cluster_id)?
            .skip(offset)
            .take(limit)
            .map(|index| &self.points[index])
            .collect())
    }

    /// Retrieve the items within a tile at the given zoom level and tile coordinates.
    /// The tile is a square area of the map. The items also include the ones of a buffer around
    /// the tile, so clusters and points on its edge are not cut off.
    ///
    /// # Arguments
    ///
    /// - `z`: The zoom level of the tile.
    /// - `x`: The X coordinate of the tile, counted from the left.
    /// - `y`: The Y coordinate of the tile, counted from the top.
    ///
    /// # Returns
    ///
    /// The items within the specified tile, which has none if there is nothing in the tile.
    pub fn get_tile(&self, z: u8, x: u32, y: u32) -> Tile<'_, P> {
        self.get_tile_with_cluster_max_zoom(z, x, y, None)
    }

    /// Retrieve the items within a tile, like `get_tile`, but let the caller lower the zoom
    /// level up to which points are clustered.
    /// Above that zoom level the tile holds the individual points instead of clusters.
    ///
    /// # Arguments
    ///
    /// - `z`: The zoom level of the tile.
    /// - `x`: The X coordinate of the tile, counted from the left.
    /// - `y`: The Y coordinate of the tile, counted from the top.
    /// - `cluster_max_zoom`: Maximum zoom level to cluster on. It can lower the configured
    ///   `max_zoom` but not raise it. `None` keeps the configured `max_zoom`.
    ///
    /// # Returns
    ///
    /// The items within the specified tile, which has none if there is nothing in the tile.
    pub fn get_tile_with_cluster_max_zoom(
        &self,
        z: u8,
        x: u32,
        y: u32,
        cluster_max_zoom: Option<u8>,
    ) -> Tile<'_, P> {
        let tree = &self.trees[self.tree_index(z, self.cluster_max_zoom(cluster_max_zoom))];
        let (x, y) = (f64::from(x), f64::from(y));
        let z2 = 2f64.powi(z as i32);
        let p = self.options.radius / self.options.extent;
        let top = (y - p) / z2;
        let bottom = (y + 1.0 + p) / z2;

        let mut items = vec![];

        let ids = tree.range((x - p) / z2, top, (x + 1.0 + p) / z2, bottom);
        self.add_tile_items(&ids, &tree.data, x, y, z2, &mut items);

        if x == 0.0 {
            let ids = tree.range(1.0 - p / z2, top, 1.0, bottom);
            self.add_tile_items(&ids, &tree.data, z2, y, z2, &mut items);
        }

        if x == z2 - 1.0 {
            let ids = tree.range(0.0, top, p / z2, bottom);
            self.add_tile_items(&ids, &tree.data, -1.0, y, z2, &mut items);
        }

        Tile { items }
    }

    /// Determine the zoom level at which a specific cluster expands.
    /// The cluster expands at the zoom level where it is split into more than one cluster or point.
    ///
    /// # Arguments
    ///
    /// - `cluster_id`: The unique identifier of the cluster.
    ///
    /// # Returns
    ///
    /// The zoom level at which the cluster expands, otherwise an error if there is no such cluster.
    pub fn get_cluster_expansion_zoom(
        &self,
        cluster_id: usize,
    ) -> Result<usize, SuperclusterError> {
        self.get_cluster_expansion_zoom_with(cluster_id, None, |_| true)
    }

    /// Determine the zoom level at which a specific cluster expands, like
    /// `get_cluster_expansion_zoom`, but let the caller lower the zoom level up to which points are
    /// clustered and leave points out.
    /// A cluster counts when one of its points passes the filter. When a single cluster or point
    /// is left after the filter, the cluster does not look different on the next zoom level, so the
    /// search goes on with what is left.
    ///
    /// # Arguments
    ///
    /// - `cluster_id`: The unique identifier of the cluster.
    /// - `cluster_max_zoom`: Maximum zoom level to cluster on. It can lower the configured
    ///   `max_zoom` but not raise it. `None` keeps the configured `max_zoom`.
    /// - `filter`: A closure that takes a point and returns whether it counts.
    ///
    /// # Returns
    ///
    /// The zoom level at which the cluster expands, otherwise an error if there is no such cluster.
    /// Above the zoom level to cluster on, this is that zoom level plus one.
    pub fn get_cluster_expansion_zoom_with<F>(
        &self,
        mut cluster_id: usize,
        cluster_max_zoom: Option<u8>,
        filter: F,
    ) -> Result<usize, SuperclusterError>
    where
        F: Fn(&P) -> bool,
    {
        let max_zoom = self.cluster_max_zoom(cluster_max_zoom) as usize;

        loop {
            let (origin_zoom, offsets) = self.find_children(cluster_id)?;

            // Above the maximum zoom level every cluster is split into its points.
            if origin_zoom > max_zoom {
                return Ok(max_zoom + 1);
            }

            let data = &self.trees[origin_zoom].data;

            // Two available children are enough to know the cluster expands here
            let mut available = vec![];

            for offset in offsets {
                if self.is_available(data, offset, &filter)? {
                    available.push(offset);
                }

                if available.len() > 1 {
                    break;
                }
            }

            match available[..] {
                [offset] if data[offset + OFFSET_NUM] > 1.0 => {
                    cluster_id = data[offset + OFFSET_ID] as usize;
                }
                _ => return Ok(origin_zoom),
            }
        }
    }

    /// Check whether a child of a cluster has a point that passes the filter.
    ///
    /// # Arguments
    ///
    /// - `data`: A reference to the flat numeric arrays representing the children.
    /// - `offset`: The offset of the child in the data.
    /// - `filter`: A closure that takes a point and returns whether it counts.
    ///
    /// # Returns
    ///
    /// `true` if the child is a point that passes, or a cluster with a point that passes.
    /// It is an error if the points of a cluster are not indexed.
    fn is_available<F>(
        &self,
        data: &[f64],
        offset: usize,
        filter: &F,
    ) -> Result<bool, SuperclusterError>
    where
        F: Fn(&P) -> bool,
    {
        let id = data[offset + OFFSET_ID] as usize;

        if data[offset + OFFSET_NUM] > 1.0 {
            Ok(self
                .iter_leaf_indices(id)?
                .any(|index| filter(&self.points[index])))
        } else {
            Ok(filter(&self.points[id]))
        }
    }

    /// Iterate over the indexes of the points of a cluster.
    ///
    /// # Arguments
    ///
    /// - `cluster_id`: The unique identifier of the cluster.
    ///
    /// # Returns
    ///
    /// The indexes of the points of the cluster, otherwise an error if there is no such cluster.
    pub(crate) fn iter_leaf_indices(
        &self,
        cluster_id: usize,
    ) -> Result<impl Iterator<Item = usize> + '_, SuperclusterError> {
        let &(start, len) = self
            .leaf_ranges
            .get(&cluster_id)
            .ok_or(SuperclusterError::ClusterNotFound)?;
        let leaves = &self.leaves[start as usize..(start + len) as usize];

        Ok(leaves.iter().map(|&index| index as usize))
    }

    /// Get the coordinates of a point that was loaded, from the index of the supercluster.
    ///
    /// # Arguments
    ///
    /// - `index`: The index of the point in the list that was loaded.
    ///
    /// # Returns
    ///
    /// The coordinates in the coordinate system of the supercluster, or `None` if the point was left
    /// out of the clustering.
    pub(crate) fn get_point_coordinates(&self, index: usize) -> Option<[f64; 2]> {
        let data = &self.trees[self.options.max_zoom as usize + 1].data;

        // The points that are not clustered are in the order they were loaded
        let mut low = 0;
        let mut high = data.len() / STRIDE;

        while low < high {
            let middle = (low + high) / 2;
            let offset = middle * STRIDE;

            match (data[offset + OFFSET_ID] as usize).cmp(&index) {
                Ordering::Less => low = middle + 1,
                Ordering::Greater => high = middle,
                Ordering::Equal => {
                    let (x, y) = self.denormalize(data[offset], data[offset + 1]);

                    return Some([x, y]);
                }
            }
        }

        None
    }

    /// Find the children of a cluster in the KD-tree they are in.
    ///
    /// # Arguments
    ///
    /// - `cluster_id`: The unique identifier of the cluster.
    ///
    /// # Returns
    ///
    /// The zoom level of the KD-tree, which is also the zoom level the cluster originates from,
    /// and the offsets of the children in its data. It is an error if there is no such cluster.
    fn find_children(&self, cluster_id: usize) -> Result<(usize, Vec<usize>), SuperclusterError> {
        let relative_id = cluster_id
            .checked_sub(self.points.len())
            .ok_or(SuperclusterError::ClusterNotFound)?;
        let origin_id = relative_id >> 5;
        let origin_zoom = relative_id % 32;

        let tree = self
            .trees
            .get(origin_zoom)
            .ok_or(SuperclusterError::ClusterNotFound)?;
        let data = &tree.data;

        if origin_id * STRIDE >= data.len() {
            return Err(SuperclusterError::ClusterNotFound);
        }

        let r = self.options.radius
            / (self.options.extent * f64::powf(2.0, (origin_zoom as f64) - 1.0));

        let x = data[origin_id * STRIDE];
        let y = data[origin_id * STRIDE + 1];

        let offsets: Vec<usize> = tree
            .within(x, y, r)
            .into_iter()
            .map(|id| id * STRIDE)
            .filter(|&offset| data[offset + OFFSET_PARENT] == (cluster_id as f64))
            .collect();

        if offsets.is_empty() {
            return Err(SuperclusterError::ClusterNotFound);
        }

        Ok((origin_zoom, offsets))
    }

    /// Convert an entry of the data arrays into a cluster or a point.
    ///
    /// # Arguments
    ///
    /// - `data`: A reference to the flat numeric arrays representing point data.
    /// - `offset`: The offset of the entry in the data.
    ///
    /// # Returns
    ///
    /// The cluster or point of the entry.
    fn get_item(&self, data: &[f64], offset: usize) -> Item<'_, P> {
        let id = data[offset + OFFSET_ID] as usize;

        if data[offset + OFFSET_NUM] > 1.0 {
            let (x, y) = self.denormalize(data[offset], data[offset + 1]);

            Item::Cluster(Cluster {
                id,
                point_count: data[offset + OFFSET_NUM] as usize,
                x,
                y,
            })
        } else {
            Item::Point {
                index: id,
                point: &self.points[id],
            }
        }
    }

    /// Populate a tile with items based on the specified point IDs, data, and tile parameters.
    ///
    /// # Arguments
    ///
    /// - `ids`: A list of point IDs used for populating the tile.
    /// - `data`: A reference to the flat numeric arrays representing point data.
    /// - `x`: The X coordinate of the tile.
    /// - `y`: The Y coordinate of the tile.
    /// - `z2`: The number of tiles per axis on the zoom level of the tile.
    /// - `items`: A mutable reference to the list to add the items to.
    fn add_tile_items<'a>(
        &'a self,
        ids: &[usize],
        data: &[f64],
        x: f64,
        y: f64,
        z2: f64,
        items: &mut Vec<TileItem<'a, P>>,
    ) {
        for id in ids {
            let offset = id * STRIDE;
            let [tile_x, tile_y] = self.get_tile_position(data[offset], data[offset + 1], z2, x, y);

            items.push(TileItem {
                x: tile_x,
                y: tile_y,
                item: self.get_item(data, offset),
            });
        }
    }

    /// Get the position in a tile of a point that is normalized to the [0..1] range.
    ///
    /// # Arguments
    ///
    /// - `x`: The normalized X value of the point.
    /// - `y`: The normalized Y value of the point.
    /// - `z2`: The number of tiles per axis on the zoom level of the tile.
    /// - `tile_x`: The X coordinate of the tile.
    /// - `tile_y`: The Y coordinate of the tile.
    ///
    /// # Returns
    ///
    /// The X and Y position in the tile, in the units of the tile extent.
    pub(crate) fn get_tile_position(
        &self,
        x: f64,
        y: f64,
        z2: f64,
        tile_x: f64,
        tile_y: f64,
    ) -> [f64; 2] {
        [
            (self.options.extent * (x * z2 - tile_x)).round(),
            (self.options.extent * (y * z2 - tile_y)).round(),
        ]
    }

    /// Get the highest zoom level to cluster on.
    ///
    /// # Arguments
    ///
    /// - `cluster_max_zoom`: Maximum zoom level to cluster on, `None` for the configured one.
    ///
    /// # Returns
    ///
    /// The lower of the configured `max_zoom` and the given zoom level.
    fn cluster_max_zoom(&self, cluster_max_zoom: Option<u8>) -> u8 {
        cluster_max_zoom.map_or(self.options.max_zoom, |zoom| {
            zoom.min(self.options.max_zoom)
        })
    }

    /// Get the index of the KD-tree to query for a zoom level.
    /// Above the highest zoom level to cluster on, this is the KD-tree of the points that are not clustered.
    ///
    /// # Arguments
    ///
    /// - `zoom`: The requested zoom level.
    /// - `cluster_max_zoom`: The highest zoom level to cluster on.
    ///
    /// # Returns
    ///
    /// The index of the KD-tree.
    fn tree_index(&self, zoom: u8, cluster_max_zoom: u8) -> usize {
        let unclustered = self.options.max_zoom as usize + 1;

        if zoom > cluster_max_zoom {
            unclustered
        } else {
            (zoom.max(self.options.min_zoom) as usize).min(unclustered)
        }
    }

    /// Convert input coordinates to the [0..1] range the KD-trees use.
    ///
    /// # Arguments
    ///
    /// - `coordinates`: The coordinates of a point in the configured coordinate system.
    ///
    /// # Returns
    ///
    /// The X and Y values in the [0..1] range.
    pub(crate) fn normalize(&self, coordinates: [f64; 2]) -> (f64, f64) {
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
    }

    /// Convert values from the [0..1] range the KD-trees use to coordinates.
    ///
    /// # Arguments
    ///
    /// - `x`: The X value in the [0..1] range.
    /// - `y`: The Y value in the [0..1] range.
    ///
    /// # Returns
    ///
    /// The X and Y coordinates in the configured coordinate system.
    fn denormalize(&self, x: f64, y: f64) -> (f64, f64) {
        match &self.options.coordinate_system {
            CoordinateSystem::Cartesian { range } => (range.denormalize(x), range.denormalize(y)),
            CoordinateSystem::LatLng => (
                convert_spherical_mercator_to_longitude(x),
                convert_spherical_mercator_to_latitude(y),
            ),
        }
    }

    /// Create a KD-tree using the specified data, which is used for spatial indexing.
    ///
    /// # Arguments
    ///
    /// - `node_size`: The size of the KD-tree leaf node.
    /// - `data`: A vector of flat numeric arrays representing point data for the KD-tree.
    ///
    /// # Returns
    ///
    /// `KDBush` instance with the specified data.
    fn create_tree(node_size: usize, data: Vec<f64>) -> KDBush {
        let mut tree = KDBush::new(data.len() / STRIDE, node_size);

        for i in (0..data.len()).step_by(STRIDE) {
            tree.add_point(data[i], data[i + 1]);
        }

        tree.build_index();
        tree.data = data;

        tree
    }

    /// Cluster points on a given zoom level using a KD-tree and returns updated data arrays.
    ///
    /// # Arguments
    ///
    /// - `tree`: A reference to the KD-tree structure for spatial indexing.
    /// - `data`: The flat numeric arrays of the points of the KD-tree, which are updated in place.
    /// - `zoom`: The zoom level at which clustering is performed.
    ///
    /// # Returns
    ///
    /// A tuple of two vectors: the first one contains updated data arrays for the current zoom level,
    /// and the second one contains data arrays for the next zoom level.
    fn cluster(&self, tree: &KDBush, mut data: Vec<f64>, zoom: u8) -> (Vec<f64>, Vec<f64>) {
        let r = self.options.radius / (self.options.extent * (2.0_f64).powi(zoom as i32));

        let mut next_data = vec![];

        // Buffers for the neighbor search, reused for every point
        let mut stack = vec![];
        let mut neighbor_ids = vec![];

        // Loop through each point
        for i in (0..data.len()).step_by(STRIDE) {
            // If we've already visited the point at this zoom level, skip it
            if data[i + OFFSET_ZOOM] <= (zoom as f64) {
                continue;
            }

            data[i + OFFSET_ZOOM] = zoom as f64;

            // Find all nearby points
            let x = data[i];
            let y = data[i + 1];

            tree.within_into(x, y, r, &mut stack, &mut neighbor_ids);

            let num_points_origin = data[i + OFFSET_NUM];
            let mut num_points = num_points_origin;

            // Count the number of points in a potential cluster
            for neighbor_id in &neighbor_ids {
                let k = neighbor_id * STRIDE;

                // Filter out neighbors that are already processed
                if data[k + OFFSET_ZOOM] > (zoom as f64) {
                    num_points += data[k + OFFSET_NUM];
                }
            }

            // If there were neighbors to merge, and there are enough points to form a cluster
            if num_points > num_points_origin && num_points >= (self.options.min_points as f64) {
                let mut wx = x * num_points_origin;
                let mut wy = y * num_points_origin;

                // Encode the zoom level and the index of the point the cluster originated from, offset by the number of points
                let id = ((i / STRIDE) << 5) + (zoom as usize + 1) + self.points.len();

                for neighbor_id in &neighbor_ids {
                    let k = neighbor_id * STRIDE;

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
                next_data.push(NOT_PROCESSED);
                next_data.push(id as f64);
                next_data.push(-1.0);
                next_data.push(num_points);
            } else {
                // Left points as unclustered
                for j in 0..STRIDE {
                    next_data.push(data[i + j]);
                }

                if num_points > 1.0 {
                    for neighbor_id in &neighbor_ids {
                        let k = neighbor_id * STRIDE;

                        if data[k + OFFSET_ZOOM] <= (zoom as f64) {
                            continue;
                        }

                        data[k + OFFSET_ZOOM] = zoom as f64;

                        for j in 0..STRIDE {
                            next_data.push(data[k + j]);
                        }
                    }
                }
            }
        }

        (data, next_data)
    }

    /// Index the points of every cluster, so they are found without walking the hierarchy of clusters.
    /// The points of a cluster are next to each other, and so are the points of the clusters in it.
    ///
    /// # Returns
    ///
    /// An error if the hierarchy of clusters is not consistent.
    fn index_leaves(&mut self) -> Result<(), SuperclusterError> {
        let mut leaves = vec![];
        let mut leaf_ranges = LeafRanges::default();

        // The clusters of the lowest zoom level hold all the others
        let data = &self.trees[self.tree_index(0, self.options.max_zoom)].data;

        for offset in (0..data.len()).step_by(STRIDE) {
            if data[offset + OFFSET_NUM] > 1.0 {
                let cluster_id = data[offset + OFFSET_ID] as usize;

                self.collect_leaves(cluster_id, &mut leaves, &mut leaf_ranges)?;
            }
        }

        leaves.shrink_to_fit();
        leaf_ranges.shrink_to_fit();

        self.leaves = leaves;
        self.leaf_ranges = leaf_ranges;

        Ok(())
    }

    /// Collect the points of a cluster and of the clusters in it.
    ///
    /// # Arguments
    ///
    /// - `cluster_id`: The unique identifier of the cluster.
    /// - `leaves`: The list to add the indexes of the points to.
    /// - `leaf_ranges`: The map to add the range of every cluster to.
    ///
    /// # Returns
    ///
    /// An error if the cluster or a cluster in it cannot be found.
    fn collect_leaves(
        &self,
        cluster_id: usize,
        leaves: &mut Vec<u32>,
        leaf_ranges: &mut LeafRanges,
    ) -> Result<(), SuperclusterError> {
        let start = leaves.len();
        let (zoom, offsets) = self.find_children(cluster_id)?;
        let data = &self.trees[zoom].data;

        for offset in offsets {
            let id = data[offset + OFFSET_ID] as usize;

            if data[offset + OFFSET_NUM] > 1.0 {
                self.collect_leaves(id, leaves, leaf_ranges)?;
            } else {
                leaves.push(id as u32);
            }
        }

        leaf_ranges.insert(cluster_id, (start as u32, (leaves.len() - start) as u32));

        Ok(())
    }
    /// Load the points into the Supercluster instance, performing clustering at various zoom levels.
    /// The input points are indexed into a KD-tree for spatial indexing.
    /// The points are clustered on the maximum zoom level, and the results are clustered on previous zoom levels.
    /// This results in a cluster hierarchy across zoom levels.
    /// Points without coordinates, and points whose coordinates are not finite numbers, are not clustered.
    /// Loading again replaces the points that were loaded before.
    /// This is CPU-heavy and blocks the thread it runs on.
    ///
    /// # Arguments
    ///
    /// - `points`: A vector of points to be clustered.
    /// - `get_coordinates`: A closure that takes a point and returns its coordinates in the
    ///   coordinate system of the supercluster, the longitude and latitude for `LatLng`. A point
    ///   that gets `None`, or coordinates that are not finite numbers, is left out of the
    ///   clustering. It is called once for every point.
    ///
    /// # Returns
    ///
    /// Supercluster instance with the input points loaded and clustered, otherwise an error if the
    /// maximum zoom level is above `MAX_ZOOM_LIMIT`, the node size is 0 or there are more than
    /// `u32::MAX` points.
    pub fn load_with<F>(
        &mut self,
        points: Vec<P>,
        get_coordinates: F,
    ) -> Result<&mut Self, SuperclusterError>
    where
        F: Fn(&P) -> Option<[f64; 2]>,
    {
        if self.options.max_zoom > MAX_ZOOM_LIMIT {
            return Err(SuperclusterError::InvalidMaxZoom);
        }

        if self.options.node_size == 0 {
            return Err(SuperclusterError::InvalidNodeSize);
        }

        if u32::try_from(points.len()).is_err() {
            return Err(SuperclusterError::TooManyPoints);
        }

        let min_zoom = self.options.min_zoom;
        let max_zoom = self.options.max_zoom;
        let node_size = self.options.node_size;

        // Free what an earlier load built first, so it does not peak together with the new one
        self.trees.fill_with(|| KDBush::new(0, node_size));
        self.leaves = vec![];
        self.leaf_ranges = LeafRanges::default();
        self.points = points;

        // Generate a cluster object for each point and index input points into a KD-tree.
        // The room is for every point, so it is exact unless points are left out.
        let mut data = Vec::with_capacity(self.points.len() * STRIDE);

        for (i, point) in self.points.iter().enumerate() {
            let Some(coordinates) = get_coordinates(point) else {
                continue;
            };

            // Store internal point/cluster data in flat numeric arrays for performance
            let (x, y) = self.normalize(coordinates);

            // A value that is not a finite number would break the sorting of the KD-tree
            if !x.is_finite() || !y.is_finite() {
                continue;
            }

            data.push(x);
            data.push(y);

            // The last zoom the point was processed at
            data.push(NOT_PROCESSED);

            // Index of the source point in the original input list
            data.push(i as f64);

            // Parent cluster id
            data.push(-1.0);

            // Number of points in a cluster
            data.push(1.0);
        }

        data.shrink_to_fit();

        #[cfg(feature = "tracing")]
        Self::warn_left_out_points(self.points.len(), data.len() / STRIDE);

        self.trees[max_zoom as usize + 1] = Self::create_tree(node_size, data);

        // Cluster points on max zoom, then cluster the results on previous zoom, etc.;
        // Results in a cluster hierarchy across zoom levels
        for zoom in (min_zoom..=max_zoom).rev() {
            let next_zoom = zoom as usize + 1;

            // Create a new set of clusters for the zoom and index them with a KD-tree
            let data = std::mem::take(&mut self.trees[next_zoom].data);
            let (previous, current) = self.cluster(&self.trees[next_zoom], data, zoom);

            self.trees[next_zoom].data = previous;

            #[cfg(feature = "tracing")]
            tracing::trace!(zoom, items = current.len() / STRIDE, "Clustered zoom level");

            self.trees[zoom as usize] = Self::create_tree(node_size, current);
        }

        self.index_leaves()?;

        #[cfg(feature = "tracing")]
        tracing::debug!(
            points = self.points.len(),
            clusters = self.leaf_ranges.len(),
            min_zoom,
            max_zoom,
            "Loaded points into the supercluster"
        );

        Ok(self)
    }

    /// Warn when points are left out of the clustering because their coordinates are missing or
    /// not finite numbers.
    ///
    /// # Arguments
    ///
    /// - `total`: The number of points that were given.
    /// - `clustered`: The number of points that are not left out.
    #[cfg(feature = "tracing")]
    fn warn_left_out_points(total: usize, clustered: usize) {
        if clustered < total {
            tracing::warn!(
                total,
                left_out = total - clustered,
                "Points without finite coordinates are left out"
            );
        }
    }
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

    use crate::SuperclusterBuilder;

    /// A point that tells whether a filter should keep it.
    #[derive(Clone, Debug, PartialEq)]
    struct TestPoint {
        /// The longitude and latitude, if the point has any.
        coordinates: Option<[f64; 2]>,

        /// Whether a filter keeps the point.
        keep: bool,
    }

    impl TestPoint {
        /// Create a point at a longitude and latitude.
        fn new(lng: f64, lat: f64, keep: bool) -> Self {
            TestPoint {
                coordinates: Some([lng, lat]),
                keep,
            }
        }

        /// Create a point without coordinates.
        fn without_coordinates() -> Self {
            TestPoint {
                coordinates: None,
                keep: true,
            }
        }
    }

    /// Create a supercluster with the points loaded and the default options.
    /// A point is a longitude, a latitude and whether a filter keeps it.
    fn load(max_zoom: u8, points: &[(f64, f64, bool)]) -> Supercluster<TestPoint> {
        let mut supercluster =
            Supercluster::new(SuperclusterBuilder::new().max_zoom(max_zoom).build());
        let points: Vec<TestPoint> = points
            .iter()
            .map(|&(lng, lat, keep)| TestPoint::new(lng, lat, keep))
            .collect();

        supercluster
            .load_with(points, |point| point.coordinates)
            .unwrap();

        supercluster
    }

    /// Get the tile that holds the point at 10, 50 (longitude, latitude).
    fn get_tile_of_point(
        supercluster: &Supercluster<TestPoint>,
        z: u8,
        cluster_max_zoom: Option<u8>,
    ) -> Tile<'_, TestPoint> {
        let z2 = 2f64.powi(z as i32);
        let x = (convert_longitude_to_spherical_mercator(10.0) * z2).floor() as u32;
        let y = (convert_latitude_to_spherical_mercator(50.0) * z2).floor() as u32;

        supercluster.get_tile_with_cluster_max_zoom(z, x, y, cluster_max_zoom)
    }

    /// Get the ID of the cluster that holds all the points at zoom level 0.
    fn get_top_cluster_id(supercluster: &Supercluster<TestPoint>) -> usize {
        match &supercluster.get_tile(0, 0, 0).items[0].item {
            Item::Cluster(cluster) => cluster.id,
            Item::Point { .. } => panic!("expected a cluster"),
        }
    }

    /// Load two close points and a farther one that does not pass the filter.
    /// The close ones cluster until zoom level 16, the three of them until zoom level 1.
    fn load_nested_cluster() -> Supercluster<TestPoint> {
        load(
            16,
            &[
                (10.0, 50.0, true),
                (10.0001, 50.0, true),
                (20.0, 50.0, false),
            ],
        )
    }

    /// Check at compile time that a type can be sent to and shared between threads.
    fn assert_send_and_sync<T: Send + Sync>() {}

    #[test]
    fn test_supercluster_is_send_and_sync() {
        assert_send_and_sync::<Supercluster<TestPoint>>();
    }

    #[test]
    fn test_options() {
        let supercluster = load(16, &[]);

        assert_eq!(supercluster.options().max_zoom, 16);
    }

    #[test]
    fn test_points() {
        let supercluster = load(16, &[(10.0, 50.0, true)]);

        assert_eq!(supercluster.points(), [TestPoint::new(10.0, 50.0, true)]);
    }

    #[test]
    fn test_load_with_invalid_max_zoom() {
        let options = SuperclusterBuilder::new().max_zoom(u8::MAX).build();
        let mut supercluster = Supercluster::<[f64; 2]>::new(options);

        assert_eq!(
            supercluster
                .load_with(vec![[0.0, 0.0]], |point| Some(*point))
                .err(),
            Some(SuperclusterError::InvalidMaxZoom)
        );
    }

    #[test]
    fn test_load_with_invalid_node_size() {
        let options = SuperclusterBuilder::new().node_size(0).build();
        let mut supercluster = Supercluster::<[f64; 2]>::new(options);

        assert_eq!(
            supercluster
                .load_with(vec![[0.0, 0.0], [1.0, 1.0]], |point| Some(*point))
                .err(),
            Some(SuperclusterError::InvalidNodeSize)
        );
    }

    /// Create points spread over the world with a fixed formula, so every run uses the same points.
    fn create_points(count: usize) -> Vec<[f64; 2]> {
        (0..count)
            .map(|i| {
                let lng = (i as f64 * 0.618_033_988_749_895) % 1.0;
                let lat = (i as f64 * 0.754_877_666_246_693) % 1.0;

                [-180.0 + 360.0 * lng, -80.0 + 160.0 * lat]
            })
            .collect()
    }

    /// Get the number of points an item stands for.
    fn get_point_count(item: &Item<'_, [f64; 2]>) -> usize {
        match item {
            Item::Cluster(cluster) => cluster.point_count,
            Item::Point { .. } => 1,
        }
    }

    #[test]
    fn test_get_clusters_counts_every_point_on_every_zoom_level() {
        let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build());

        supercluster
            .load_with(create_points(500), |point| Some(*point))
            .unwrap();

        // The highest zoom level holds the points that are not clustered
        for zoom in 0..=17 {
            let count: usize = supercluster
                .get_clusters([-180.0, -90.0, 180.0, 90.0], zoom)
                .iter()
                .map(get_point_count)
                .sum();

            assert_eq!(count, 500, "zoom level {zoom}");
        }
    }

    #[test]
    fn test_get_leaves_of_a_cluster_are_the_leaves_of_its_children() {
        let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build());

        supercluster
            .load_with(create_points(500), |point| Some(*point))
            .unwrap();

        let mut clusters = 0;

        for zoom in 0..=6 {
            for item in supercluster.get_clusters([-180.0, -90.0, 180.0, 90.0], zoom) {
                let Item::Cluster(cluster) = item else {
                    continue;
                };

                let leaves = supercluster.get_leaves(cluster.id, usize::MAX, 0).unwrap();
                let mut leaves_of_children: Vec<&[f64; 2]> = vec![];

                for child in supercluster.get_children(cluster.id).unwrap() {
                    match child {
                        Item::Cluster(child) => leaves_of_children
                            .extend(supercluster.get_leaves(child.id, usize::MAX, 0).unwrap()),
                        Item::Point { point, .. } => leaves_of_children.push(point),
                    }
                }

                assert_eq!(leaves.len(), cluster.point_count);
                assert_eq!(leaves, leaves_of_children);

                clusters += 1;
            }
        }

        assert!(clusters > 0);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_supercluster_round_trips_through_json() {
        let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build());

        supercluster
            .load_with(create_points(300), |point| Some(*point))
            .unwrap();

        let json = serde_json::to_string(&supercluster).unwrap();
        let restored: Supercluster<[f64; 2]> = serde_json::from_str(&json).unwrap();
        let bbox = [-180.0, -90.0, 180.0, 90.0];

        for zoom in [0, 4, 9, 17] {
            assert_eq!(
                restored.get_clusters(bbox, zoom),
                supercluster.get_clusters(bbox, zoom)
            );
        }

        assert_eq!(restored.get_tile(2, 1, 1), supercluster.get_tile(2, 1, 1));
    }

    #[test]
    fn test_load_without_points() {
        let supercluster = load(16, &[]);

        assert!(supercluster.get_tile(0, 0, 0).is_empty());
        assert!(supercluster
            .get_clusters([-180.0, -90.0, 180.0, 90.0], 3)
            .is_empty());
        assert_eq!(
            supercluster.get_children(5).err(),
            Some(SuperclusterError::ClusterNotFound)
        );
    }

    #[test]
    fn test_load_replaces_the_points() {
        let mut supercluster = load(16, &[(10.0, 50.0, true), (12.0, 50.0, true)]);

        supercluster
            .load_with(vec![TestPoint::new(-20.0, 10.0, true)], |point| {
                point.coordinates
            })
            .unwrap();

        let items = supercluster.get_clusters([-180.0, -90.0, 180.0, 90.0], 0);

        assert_eq!(items.len(), 1);
        assert_eq!(supercluster.points().len(), 1);
    }

    /// Get the indexes of the points that are not clustered at the highest zoom level, which are
    /// the points that were not left out.
    fn get_point_indexes(supercluster: &Supercluster<TestPoint>) -> Vec<usize> {
        let mut indexes: Vec<usize> = supercluster
            .get_clusters([-180.0, -90.0, 180.0, 90.0], 17)
            .iter()
            .filter_map(|item| match item {
                Item::Point { index, .. } => Some(*index),
                Item::Cluster(_) => None,
            })
            .collect();

        indexes.sort();

        indexes
    }

    #[test]
    fn test_load_skips_points_without_coordinates() {
        let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build());

        supercluster
            .load_with(
                vec![
                    TestPoint::new(10.0, 50.0, true),
                    TestPoint::without_coordinates(),
                    TestPoint::new(12.0, 50.0, true),
                ],
                |point| point.coordinates,
            )
            .unwrap();

        assert_eq!(get_point_indexes(&supercluster), vec![0, 2]);
    }

    #[test]
    fn test_load_skips_points_with_coordinates_that_are_not_finite() {
        let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build());

        supercluster
            .load_with(
                vec![
                    TestPoint::new(10.0, 50.0, true),
                    TestPoint::new(f64::NAN, 50.0, true),
                    TestPoint::new(10.0, f64::INFINITY, true),
                    TestPoint::new(f64::NEG_INFINITY, 50.0, true),
                    TestPoint::new(12.0, 50.0, true),
                ],
                |point| point.coordinates,
            )
            .unwrap();

        assert_eq!(get_point_indexes(&supercluster), vec![0, 4]);
    }

    #[test]
    fn test_get_tile_without_cluster_max_zoom() {
        let supercluster = load(16, &[(10.0, 50.0, true), (12.0, 50.0, true)]);
        let tile = get_tile_of_point(&supercluster, 1, None);

        assert_eq!(tile.items.len(), 1);
        assert!(matches!(&tile.items[0].item, Item::Cluster(cluster) if cluster.point_count == 2));
    }

    #[test]
    fn test_get_tile_at_cluster_max_zoom() {
        let supercluster = load(16, &[(10.0, 50.0, true), (12.0, 50.0, true)]);

        assert_eq!(get_tile_of_point(&supercluster, 1, Some(1)).items.len(), 1);
    }

    #[test]
    fn test_get_tile_above_cluster_max_zoom() {
        let supercluster = load(16, &[(10.0, 50.0, true), (12.0, 50.0, true)]);
        let tile = get_tile_of_point(&supercluster, 1, Some(0));

        assert_eq!(tile.items.len(), 2);
        assert!(tile
            .items
            .iter()
            .all(|tile_item| matches!(tile_item.item, Item::Point { .. })));
    }

    #[test]
    fn test_get_tile_cluster_max_zoom_above_max_zoom() {
        let supercluster = load(4, &[(10.0, 50.0, true), (12.0, 50.0, true)]);

        // The given zoom level cannot raise the maximum zoom level, so zoom level 6 reads the points that are not clustered.
        assert_eq!(get_tile_of_point(&supercluster, 6, Some(8)).items.len(), 1);
    }

    #[test]
    fn test_get_tile_position_of_items() {
        let supercluster = load(16, &[(0.0, 0.0, true)]);
        let tile = supercluster.get_tile(0, 0, 0);

        assert_eq!(tile.items.len(), 1);
        assert_eq!((tile.items[0].x, tile.items[0].y), (256.0, 256.0));
    }

    #[test]
    fn test_get_tile_without_loaded_points() {
        let supercluster = Supercluster::<TestPoint>::new(SuperclusterBuilder::new().build());

        assert!(supercluster.get_tile(0, 0, 0).is_empty());
    }

    #[test]
    fn test_get_children() {
        let supercluster = load_nested_cluster();
        let mut point_counts: Vec<usize> = supercluster
            .get_children(get_top_cluster_id(&supercluster))
            .unwrap()
            .iter()
            .map(|item| match item {
                Item::Cluster(cluster) => cluster.point_count,
                Item::Point { .. } => 1,
            })
            .collect();

        point_counts.sort();

        assert_eq!(point_counts, vec![1, 2]);
    }

    #[test]
    fn test_get_children_of_unknown_cluster() {
        let supercluster = load_nested_cluster();

        // Smaller than the number of points, so it is a point and not a cluster.
        assert_eq!(
            supercluster.get_children(1).err(),
            Some(SuperclusterError::ClusterNotFound)
        );
        // The zoom level in the ID does not exist.
        assert_eq!(
            supercluster.get_children(100000).err(),
            Some(SuperclusterError::ClusterNotFound)
        );
    }

    #[test]
    fn test_get_leaves() {
        let supercluster = load_nested_cluster();
        let cluster_id = get_top_cluster_id(&supercluster);

        let all = supercluster.get_leaves(cluster_id, usize::MAX, 0).unwrap();

        assert_eq!(all.len(), 3);

        // Every point is there exactly once.
        let mut keeps: Vec<bool> = all.iter().map(|point| point.keep).collect();

        keeps.sort();

        assert_eq!(keeps, vec![false, true, true]);

        // The limit and offset select a window of the same order.
        assert_eq!(supercluster.get_leaves(cluster_id, 2, 0).unwrap(), all[..2]);
        assert_eq!(supercluster.get_leaves(cluster_id, 2, 1).unwrap(), all[1..]);
        assert_eq!(
            supercluster.get_leaves(cluster_id, 10, 2).unwrap(),
            all[2..]
        );
        assert!(supercluster
            .get_leaves(cluster_id, 10, 3)
            .unwrap()
            .is_empty());
        assert!(supercluster
            .get_leaves(cluster_id, 0, 0)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_get_leaves_of_nested_cluster() {
        let supercluster = load_nested_cluster();
        let nested_id = supercluster
            .get_children(get_top_cluster_id(&supercluster))
            .unwrap()
            .iter()
            .find_map(|item| match item {
                Item::Cluster(cluster) => Some(cluster.id),
                Item::Point { .. } => None,
            })
            .unwrap();

        let leaves = supercluster.get_leaves(nested_id, usize::MAX, 0).unwrap();

        assert_eq!(leaves.len(), 2);
        assert!(leaves.iter().all(|point| point.keep));
    }

    #[test]
    fn test_get_leaves_of_unknown_cluster() {
        let supercluster = load_nested_cluster();

        assert_eq!(
            supercluster.get_leaves(100000, 10, 0).err(),
            Some(SuperclusterError::ClusterNotFound)
        );
    }

    #[test]
    fn test_get_cluster_expansion_zoom() {
        let supercluster = load_nested_cluster();
        let cluster_id = get_top_cluster_id(&supercluster);

        assert_eq!(supercluster.get_cluster_expansion_zoom(cluster_id), Ok(2));
    }

    #[test]
    fn test_get_cluster_expansion_zoom_of_unknown_cluster() {
        let supercluster = load_nested_cluster();

        assert_eq!(
            supercluster.get_cluster_expansion_zoom(100000),
            Err(SuperclusterError::ClusterNotFound)
        );
    }

    #[test]
    fn test_get_cluster_expansion_zoom_with_filter_accepting_all() {
        let supercluster = load_nested_cluster();
        let cluster_id = get_top_cluster_id(&supercluster);

        assert_eq!(
            supercluster.get_cluster_expansion_zoom_with(cluster_id, None, |_| true),
            Ok(2)
        );
    }

    #[test]
    fn test_get_cluster_expansion_zoom_with_filter_rejecting_a_child() {
        let supercluster = load_nested_cluster();
        let cluster_id = get_top_cluster_id(&supercluster);

        // Without the farther point the cluster looks the same until the close points split on zoom level 17.
        assert_eq!(
            supercluster.get_cluster_expansion_zoom_with(cluster_id, None, |point| point.keep),
            Ok(17)
        );
    }

    #[test]
    fn test_get_cluster_expansion_zoom_with_filter_rejecting_all() {
        let supercluster = load_nested_cluster();
        let cluster_id = get_top_cluster_id(&supercluster);

        assert_eq!(
            supercluster.get_cluster_expansion_zoom_with(cluster_id, None, |_| false),
            Ok(2)
        );
    }

    #[test]
    fn test_get_cluster_expansion_zoom_with_cluster_max_zoom() {
        let supercluster = load_nested_cluster();
        let cluster_id = get_top_cluster_id(&supercluster);

        // Above the zoom level to cluster on there is nothing left to expand.
        assert_eq!(
            supercluster.get_cluster_expansion_zoom_with(cluster_id, Some(0), |_| true),
            Ok(1)
        );
        assert_eq!(
            supercluster.get_cluster_expansion_zoom_with(cluster_id, Some(5), |_| true),
            Ok(2)
        );
        assert_eq!(
            supercluster.get_cluster_expansion_zoom_with(cluster_id, Some(5), |point| point.keep),
            Ok(6)
        );
        assert_eq!(
            supercluster.get_cluster_expansion_zoom_with(cluster_id, Some(20), |point| point.keep),
            Ok(17)
        );
    }

    #[test]
    fn test_get_clusters_below_min_zoom() {
        let options = SuperclusterBuilder::new().min_zoom(3).build();
        let mut supercluster = Supercluster::new(options);

        supercluster
            .load_with(
                vec![
                    TestPoint::new(10.0, 50.0, true),
                    TestPoint::new(12.0, 50.0, true),
                ],
                |point| point.coordinates,
            )
            .unwrap();

        let bbox = [-180.0, -90.0, 180.0, 90.0];

        // A zoom level below the minimum one reads the minimum one
        assert_eq!(
            supercluster.get_clusters(bbox, 1),
            supercluster.get_clusters(bbox, 3)
        );
    }

    #[test]
    fn test_get_clusters_has_the_center_of_the_cluster() {
        let supercluster = load(16, &[(10.0, 50.0, true), (12.0, 50.0, true)]);
        let items = supercluster.get_clusters([-180.0, -90.0, 180.0, 90.0], 0);

        let [Item::Cluster(cluster)] = &items[..] else {
            panic!("expected a single cluster");
        };

        assert!((cluster.x - 11.0).abs() < 1e-9);
        assert!((cluster.y - 50.0).abs() < 1e-9);
    }

    #[test]
    fn test_point_count_abbreviated() {
        let abbreviate = |point_count| {
            Cluster {
                id: 0,
                point_count,
                x: 0.0,
                y: 0.0,
            }
            .point_count_abbreviated()
        };

        assert_eq!(abbreviate(3), "3");
        assert_eq!(abbreviate(999), "999");
        assert_eq!(abbreviate(1000), "1k");
        assert_eq!(abbreviate(1500), "1.5k");
        assert_eq!(abbreviate(10000), "10k");
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
