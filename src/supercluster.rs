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
//!
//! Every zoom level has a KD-tree of its items, and an item of a tree is a `u32` handle: a point
//! by its position in the points, or a cluster by its ID. The points are in leaf order, where the
//! points of every cluster are next to each other, so the points of a cluster are a range of them.
//! What a query needs to know about a cluster is in one table, by the number of the cluster.

use std::{
    f64::consts::PI,
    fmt::{self, Display, Formatter},
    ops::Range,
};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{kdbush::KDBush, DataRange, SuperclusterError, SuperclusterOptions};

/// The highest maximum zoom level a supercluster supports. Building options with a higher
/// `max_zoom` is an error.
pub const MAX_ZOOM_LIMIT: u8 = 30;

/// The highest number of points a supercluster holds. A KD-tree refers to a point or a cluster with
/// a `u32`, and there are fewer clusters than points, so twice this number still fits.
pub const MAX_POINTS: usize = 1 << 31;

/// An item of a zoom level while the points are clustered.
#[derive(Clone, Copy)]
struct Entry {
    /// The position, normalized to the [0..1] range the KD-trees use.
    position: [f64; 2],

    /// The handle of the item: a point by its index in the loaded list, until the points move into
    /// leaf order, or a cluster by its ID.
    handle: u32,

    /// The number of points of the item, 1 for a point.
    point_count: u32,
}

/// The clusters of every zoom level, in the order loading formed them. The cluster with the number
/// `c` has the ID `c` plus the number of points, so an ID is never the position of a point.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize))]
struct Clusters {
    /// The center of every cluster, normalized to the [0..1] range the KD-trees use.
    positions: Vec<[f64; 2]>,

    /// The number of points of every cluster.
    point_counts: Vec<u32>,

    /// The zoom level every cluster formed on. Its children are on the zoom level after it.
    zooms: Vec<u8>,

    /// Where the points of every cluster start in the points of the supercluster.
    leaf_starts: Vec<u32>,

    /// Where the children of every cluster start in `children`, followed by the end of the
    /// children of the last cluster.
    child_starts: Vec<u32>,

    /// The handles of the children of every cluster, one cluster after the other, in the order the
    /// neighbor search found them.
    children: Vec<u32>,
}

impl Clusters {
    /// Create a table without clusters.
    fn new() -> Self {
        Clusters {
            positions: vec![],
            point_counts: vec![],
            zooms: vec![],
            leaf_starts: vec![],
            child_starts: vec![0],
            children: vec![],
        }
    }

    /// Get the number of clusters.
    fn len(&self) -> usize {
        self.point_counts.len()
    }

    /// Get the handles of the children of a cluster.
    fn children_of(&self, cluster: usize) -> &[u32] {
        &self.children[self.child_starts[cluster] as usize..self.child_starts[cluster + 1] as usize]
    }

    /// Add a cluster that formed on `zoom`, whose children are the ones added to `children` since
    /// the last cluster.
    fn push(&mut self, position: [f64; 2], point_count: u32, zoom: u8) {
        self.positions.push(position);
        self.point_counts.push(point_count);
        self.zooms.push(zoom);
        self.leaf_starts.push(0);
        self.child_starts.push(self.children.len() as u32);
    }

    /// Give the points of a cluster, and of the clusters in it, the next positions in leaf order,
    /// in the order of its children. `leaf_of` gets the position of every point by its index in the
    /// loaded list, and `point_count` is the number of points, which tells a cluster from a point.
    fn collect_leaves(
        &mut self,
        cluster: usize,
        point_count: usize,
        leaf_of: &mut [u32],
        next_leaf: &mut u32,
    ) {
        self.leaf_starts[cluster] = *next_leaf;

        for i in self.child_starts[cluster] as usize..self.child_starts[cluster + 1] as usize {
            let child = self.children[i] as usize;

            match child.checked_sub(point_count) {
                Some(child) => self.collect_leaves(child, point_count, leaf_of, next_leaf),
                None => {
                    leaf_of[child] = *next_leaf;
                    *next_leaf += 1;
                }
            }
        }
    }

    /// Free the room the vectors grew into but do not use.
    fn shrink_to_fit(&mut self) {
        self.positions.shrink_to_fit();
        self.point_counts.shrink_to_fit();
        self.zooms.shrink_to_fit();
        self.leaf_starts.shrink_to_fit();
        self.child_starts.shrink_to_fit();
        self.children.shrink_to_fit();
    }
}

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

impl CoordinateSystem {
    /// Convert coordinates to the [0..1] range the KD-trees use.
    pub(crate) fn normalize(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        match self {
            CoordinateSystem::Cartesian { range } => [range.normalize(x), range.normalize(y)],
            CoordinateSystem::LatLng => [
                convert_longitude_to_spherical_mercator(x),
                convert_latitude_to_spherical_mercator(y),
            ],
        }
    }

    /// Convert a position from the [0..1] range the KD-trees use to coordinates.
    pub(crate) fn denormalize(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        match self {
            CoordinateSystem::Cartesian { range } => [range.denormalize(x), range.denormalize(y)],
            CoordinateSystem::LatLng => [
                convert_spherical_mercator_to_longitude(x),
                convert_spherical_mercator_to_latitude(y),
            ],
        }
    }
}

/// The identifier of a cluster.
///
/// A client can send the number back, for example after reading it from the GeoJSON property
/// `cluster_id`, and `ClusterId::new` or `From` turns it into an identifier again. A number that
/// is not the identifier of a cluster of the supercluster gives a `ClusterNotFound` error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize), serde(transparent))]
pub struct ClusterId(pub(crate) u32);

impl ClusterId {
    /// Create a cluster identifier from its number, as `get` returns it.
    pub fn new(id: u32) -> Self {
        ClusterId(id)
    }

    /// Get the number of the cluster identifier.
    pub fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for ClusterId {
    fn from(id: u32) -> Self {
        ClusterId(id)
    }
}

impl From<ClusterId> for u32 {
    fn from(id: ClusterId) -> Self {
        id.0
    }
}

/// A cluster of points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Cluster<'a> {
    /// The unique identifier of the cluster.
    pub id: ClusterId,

    /// The number of points in the cluster.
    pub point_count: u32,

    /// The center of the cluster, normalized to the [0..1] range the KD-trees use.
    position: [f64; 2],

    /// The coordinate system that converts the center into coordinates.
    coordinate_system: &'a CoordinateSystem,
}

impl Cluster<'_> {
    /// Get the coordinates of the center of the cluster, in the coordinate system of the
    /// supercluster: the longitude and latitude for `LatLng`. They are computed on each call, so a
    /// query that does not need them does not pay for them.
    pub fn coordinates(&self) -> [f64; 2] {
        self.coordinate_system.denormalize(self.position)
    }

    /// Get the number of points as a short text, for example `1.5k` for 1500 points.
    pub fn point_count_abbreviated(&self) -> String {
        // `to_string` of an integer skips the formatting machinery that `Display` goes through
        if self.point_count < 1000 {
            self.point_count.to_string()
        } else {
            AbbreviatedCount(self.point_count).to_string()
        }
    }
}

/// The number of points of a cluster as a short text, for example `1.5k` for 1500 points, which
/// `Display` writes without allocating.
pub(crate) struct AbbreviatedCount(pub(crate) u32);

impl Display for AbbreviatedCount {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let count = f64::from(self.0);

        if count >= 10000.0 {
            write!(formatter, "{}k", (count / 1000.0).round())
        } else if count >= 1000.0 {
            write!(formatter, "{}k", (count / 100.0).round() / 10.0)
        } else {
            write!(formatter, "{}", self.0)
        }
    }
}

/// A cluster, or a point that is not part of a cluster at the queried zoom level.
#[derive(Debug, PartialEq)]
pub enum Item<'a, P> {
    /// A cluster of points.
    Cluster(Cluster<'a>),

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
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// A spatial clustering configuration and data structure, built once by `load`.
///
/// The points are moved into leaf order when they are loaded, and never cloned, so a point type
/// that shares its data, such as an `Arc`, is not copied.
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

    /// The points, in leaf order: the points of every cluster are next to each other, and so are
    /// the points of the clusters in it. The points that no cluster holds follow, then the points
    /// that were left out of the clustering.
    points: Vec<P>,

    /// The index of every point in the list that was loaded, in the order of `points`.
    indexes: Vec<u32>,

    /// The position of every point that was clustered, normalized to the [0..1] range the KD-trees
    /// use, in the order of `points`. The points that were left out have none.
    positions: Vec<[f64; 2]>,

    /// KD-trees for each zoom level, from 0 to the maximum zoom level plus one, whose IDs are the
    /// handles of their items. The last one holds the points that are not clustered. The ones
    /// below the minimum zoom level are empty.
    trees: Vec<KDBush>,

    /// The clusters of every zoom level.
    clusters: Clusters,
}

impl<P> Supercluster<P> {
    /// Load the points into a new supercluster, which clusters them on every zoom level.
    /// The points are clustered on the maximum zoom level, and the results are clustered on previous
    /// zoom levels, which results in a cluster hierarchy across zoom levels.
    ///
    /// `get_coordinates` returns the coordinates of a point in the coordinate system of the options,
    /// the longitude and latitude for `LatLng`. It runs once for every point, in order. A point that
    /// gets `None`, or coordinates that are not finite numbers, is left out of the clustering.
    ///
    /// The points move into leaf order, where the points of a cluster are next to each other.
    /// `Item::Point` keeps the index of a point in the list that was loaded.
    ///
    /// This is CPU-heavy and blocks the thread it runs on. To load other points, load a new
    /// supercluster, and drop the old one first if memory is tight.
    ///
    /// # Errors
    ///
    /// Returns `SuperclusterError::TooManyPoints` if there are more than `MAX_POINTS` points.
    ///
    /// # Examples
    ///
    /// ```
    /// use supercluster::{Supercluster, SuperclusterOptions};
    ///
    /// struct Station {
    ///     lng: f64,
    ///     lat: f64,
    /// }
    ///
    /// let stations = vec![Station { lng: 4.9, lat: 52.37 }, Station { lng: 4.91, lat: 52.37 }];
    /// let supercluster = Supercluster::load(SuperclusterOptions::default(), stations, |station| {
    ///     Some([station.lng, station.lat])
    /// })?;
    ///
    /// assert_eq!(supercluster.points().len(), 2);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn load<F>(
        options: SuperclusterOptions,
        points: Vec<P>,
        get_coordinates: F,
    ) -> Result<Self, SuperclusterError>
    where
        F: FnMut(&P) -> Option<[f64; 2]>,
    {
        if points.len() > MAX_POINTS {
            return Err(SuperclusterError::TooManyPoints);
        }

        let mut supercluster = Supercluster {
            options,
            points,
            indexes: vec![],
            positions: vec![],
            trees: vec![],
            clusters: Clusters::new(),
        };

        supercluster.index(get_coordinates);

        Ok(supercluster)
    }

    /// Get the configuration settings.
    pub fn options(&self) -> &SuperclusterOptions {
        &self.options
    }

    /// Get the points that were loaded, in leaf order: the points of every cluster are next to
    /// each other. The points that no cluster holds follow, then the points that were left out of
    /// the clustering. `Item::Point` has the index of a point in the list that was loaded.
    pub fn points(&self) -> &[P] {
        &self.points
    }

    /// Get a view whose queries cluster up to `cluster_max_zoom` instead of the `max_zoom` of the
    /// options. It can lower the `max_zoom` but not raise it. Above that zoom level, the view
    /// returns the individual points instead of clusters.
    ///
    /// # Examples
    ///
    /// ```
    /// use supercluster::{Item, Supercluster, SuperclusterOptions};
    ///
    /// let points = vec![[10.0, 50.0], [12.0, 50.0]];
    /// let supercluster =
    ///     Supercluster::load(SuperclusterOptions::default(), points, |point| Some(*point))?;
    ///
    /// // The two points form a cluster on zoom level 1, unless the view stops clustering at 0
    /// let items = supercluster.with_cluster_max_zoom(0).clusters([-180.0, -90.0, 180.0, 90.0], 1);
    ///
    /// assert!(items.iter().all(|item| matches!(item, Item::Point { .. })));
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn with_cluster_max_zoom(&self, cluster_max_zoom: u8) -> SuperclusterView<'_, P> {
        SuperclusterView {
            supercluster: self,
            cluster_max_zoom: cluster_max_zoom.min(self.options.max_zoom),
        }
    }

    /// Get a view that clusters up to the `max_zoom` of the options, which the queries of the
    /// supercluster itself use.
    pub(crate) fn view(&self) -> SuperclusterView<'_, P> {
        self.with_cluster_max_zoom(self.options.max_zoom)
    }

    /// Retrieve the clusters, and the points that are not clustered, within a bounding box at a
    /// zoom level. `bbox` is `[min_lng, min_lat, max_lng, max_lat]`, or `[min_x, min_y, max_x,
    /// max_y]` for the `Cartesian` coordinate system. A box that crosses the antimeridian, with
    /// `min_lng` above `max_lng`, holds both sides.
    pub fn clusters(&self, bbox: [f64; 4], zoom: u8) -> Vec<Item<'_, P>> {
        self.view().clusters(bbox, zoom)
    }

    /// Retrieve the clusters and points a cluster is made of, on the zoom level after the one it
    /// comes from. Loading recorded them, so this does not search for them.
    ///
    /// # Errors
    ///
    /// Returns `SuperclusterError::ClusterNotFound` if there is no such cluster.
    pub fn children(
        &self,
        cluster_id: ClusterId,
    ) -> Result<impl ExactSizeIterator<Item = Item<'_, P>> + '_, SuperclusterError> {
        let cluster = self.cluster_index(cluster_id)?;

        Ok(self
            .clusters
            .children_of(cluster)
            .iter()
            .map(|&handle| self.item(handle, self.position(handle))))
    }

    /// Retrieve the points of a cluster, as they were loaded. They are next to each other in
    /// `points`, so this is a slice of them, and they always come in the same order.
    ///
    /// # Errors
    ///
    /// Returns `SuperclusterError::ClusterNotFound` if there is no such cluster.
    ///
    /// # Examples
    ///
    /// ```
    /// use supercluster::{Item, Supercluster, SuperclusterOptions};
    ///
    /// let points = vec![[10.0, 50.0], [10.001, 50.0], [10.002, 50.0]];
    /// let supercluster =
    ///     Supercluster::load(SuperclusterOptions::default(), points, |point| Some(*point))?;
    ///
    /// if let Item::Cluster(cluster) = supercluster.tile(0, 0, 0).items[0].item {
    ///     // The second page of two points holds the last point
    ///     let page: Vec<_> = supercluster.leaves(cluster.id)?.iter().skip(2).take(2).collect();
    ///
    ///     assert_eq!(page.len(), 1);
    /// }
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn leaves(&self, cluster_id: ClusterId) -> Result<&[P], SuperclusterError> {
        self.cluster_index(cluster_id)?;

        Ok(&self.points[self.leaf_range(cluster_id.0)])
    }

    /// Retrieve the items of the tile at zoom level `z`, `x` tiles from the left and `y` tiles from
    /// the top. The items also include the ones of a buffer around the tile, so clusters and points
    /// on its edge are not cut off. For `LatLng`, the first and last tile also hold the items of the
    /// opposite edge, across the antimeridian. `Cartesian` does not wrap, because the two ends of
    /// its `DataRange` are not next to each other. A tile with nothing in it has no items.
    ///
    /// # Examples
    ///
    /// ```
    /// use supercluster::{Item, Supercluster, SuperclusterOptions};
    ///
    /// let points = vec![[10.0, 50.0], [12.0, 50.0], [-100.0, -30.0]];
    /// let supercluster =
    ///     Supercluster::load(SuperclusterOptions::default(), points, |point| Some(*point))?;
    ///
    /// for tile_item in &supercluster.tile(0, 0, 0).items {
    ///     match tile_item.item {
    ///         Item::Cluster(cluster) => println!("{} points", cluster.point_count),
    ///         Item::Point { index, .. } => println!("point {index}"),
    ///     }
    /// }
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn tile(&self, z: u8, x: u32, y: u32) -> Tile<'_, P> {
        self.view().tile(z, x, y)
    }

    /// Determine the zoom level at which a cluster expands, where it splits into more than one
    /// cluster or point. A map zooms in to it when the user clicks the cluster.
    ///
    /// # Errors
    ///
    /// Returns `SuperclusterError::ClusterNotFound` if there is no such cluster.
    pub fn cluster_expansion_zoom(&self, cluster_id: ClusterId) -> Result<u8, SuperclusterError> {
        self.view().cluster_expansion_zoom(cluster_id)
    }

    /// Determine the zoom level at which a specific cluster expands, like
    /// `cluster_expansion_zoom`, but count only the points that pass a filter.
    /// A cluster counts when one of its points passes the filter. When a single cluster or point
    /// is left after the filter, the cluster does not look different on the next zoom level, so the
    /// search goes on with what is left.
    ///
    /// # Errors
    ///
    /// Returns `SuperclusterError::ClusterNotFound` if there is no such cluster.
    pub fn cluster_expansion_zoom_filtered<F>(
        &self,
        cluster_id: ClusterId,
        filter: F,
    ) -> Result<u8, SuperclusterError>
    where
        F: Fn(&P) -> bool,
    {
        self.view()
            .cluster_expansion_zoom_filtered(cluster_id, filter)
    }

    /// Get the number of a cluster in the table of clusters from its ID, or an error if no cluster
    /// has the ID.
    fn cluster_index(&self, cluster_id: ClusterId) -> Result<usize, SuperclusterError> {
        (cluster_id.0 as usize)
            .checked_sub(self.points.len())
            .filter(|&cluster| cluster < self.clusters.len())
            .ok_or(SuperclusterError::ClusterNotFound)
    }

    /// Get the positions in `points` of the points of an item of a KD-tree: the point itself, or
    /// the points of a cluster.
    pub(crate) fn leaf_range(&self, handle: u32) -> Range<usize> {
        let handle = handle as usize;

        match handle.checked_sub(self.points.len()) {
            Some(cluster) => {
                let start = self.clusters.leaf_starts[cluster] as usize;

                start..start + self.clusters.point_counts[cluster] as usize
            }
            None => handle..handle + 1,
        }
    }

    /// Get the normalized position of an item of a KD-tree, from the table of clusters or of the
    /// points.
    fn position(&self, handle: u32) -> [f64; 2] {
        let handle = handle as usize;

        match handle.checked_sub(self.points.len()) {
            Some(cluster) => self.clusters.positions[cluster],
            None => self.positions[handle],
        }
    }

    /// Get the normalized position of the point at `leaf` in `points`. Only the points that were
    /// left out of the clustering have none.
    pub(crate) fn point_position(&self, leaf: usize) -> [f64; 2] {
        self.positions[leaf]
    }

    /// Get the number of points that were clustered, which come before the ones left out in
    /// `points`.
    pub(crate) fn clustered_points(&self) -> usize {
        self.positions.len()
    }

    /// Convert an item of a KD-tree, at its normalized `position`, into a cluster or a point.
    pub(crate) fn item(&self, handle: u32, position: [f64; 2]) -> Item<'_, P> {
        let leaf = handle as usize;

        match leaf.checked_sub(self.points.len()) {
            Some(cluster) => Item::Cluster(Cluster {
                id: ClusterId(handle),
                point_count: self.clusters.point_counts[cluster],
                position,
                coordinate_system: &self.options.coordinate_system,
            }),
            None => Item::Point {
                index: self.indexes[leaf] as usize,
                point: &self.points[leaf],
            },
        }
    }

    /// Determine the zoom level at which a cluster expands, up to `cluster_max_zoom`, counting only
    /// the children for which `has_match` returns `true`. `has_match` gets the positions in
    /// `points` of the points of a child. It is an error if there is no such cluster.
    pub(crate) fn expansion_zoom_where<F>(
        &self,
        cluster_id: ClusterId,
        cluster_max_zoom: u8,
        mut has_match: F,
    ) -> Result<u8, SuperclusterError>
    where
        F: FnMut(Range<usize>) -> bool,
    {
        let mut cluster = self.cluster_index(cluster_id)?;

        loop {
            // The children of a cluster are on the zoom level after the one it formed on
            let origin_zoom = self.clusters.zooms[cluster] + 1;

            // Above the maximum zoom level every cluster is split into its points.
            if origin_zoom > cluster_max_zoom {
                return Ok(cluster_max_zoom + 1);
            }

            // Two children with a match are enough to know the cluster expands here
            let mut matches = self
                .clusters
                .children_of(cluster)
                .iter()
                .filter(|&&child| has_match(self.leaf_range(child)));

            match (matches.next(), matches.next()) {
                (Some(&child), None) => match (child as usize).checked_sub(self.points.len()) {
                    Some(child) => cluster = child,
                    None => return Ok(origin_zoom),
                },
                _ => return Ok(origin_zoom),
            }
        }
    }

    /// Call `visit` with the handle and the normalized position of every item of the tile at zoom
    /// level `z`, `x` tiles from the left and `y` tiles from the top, and with the X of the tile the
    /// position is relative to. That X is a world width away from `x` for the items that the first
    /// and last tile show from across the antimeridian.
    pub(crate) fn for_each_in_tile(
        &self,
        cluster_max_zoom: u8,
        (z, x, y): (u8, u32, u32),
        mut visit: impl FnMut(u32, [f64; 2], f64),
    ) {
        let tree = &self.trees[self.tree_index(z, cluster_max_zoom)];
        let (x, y) = (f64::from(x), f64::from(y));
        let z2 = tiles_per_axis(z);
        let p = self.options.radius / self.options.extent;
        let top = (y - p) / z2;
        let bottom = (y + 1.0 + p) / z2;

        let mut visit_range = |min_x: f64, max_x: f64, origin_x: f64| {
            tree.range(min_x, top, max_x, bottom, |position| {
                visit(tree.ids[position], tree.coords[position], origin_x)
            });
        };

        visit_range((x - p) / z2, (x + 1.0 + p) / z2, x);

        // For `LatLng`, the first and last tile also show the other edge.
        // The two ends of a `Cartesian` range are not next to each other, so they do not wrap.
        if self.options.coordinate_system == CoordinateSystem::LatLng {
            if x == 0.0 {
                visit_range(1.0 - p / z2, 1.0, z2);
            }

            if x == z2 - 1.0 {
                visit_range(0.0, p / z2, -1.0);
            }
        }
    }

    /// Get the position, in the units of the tile extent, of a point that is normalized to the
    /// [0..1] range, in the tile at `tile_x`, `tile_y`, where `z2` is the number of tiles per axis.
    pub(crate) fn tile_position(
        &self,
        [x, y]: [f64; 2],
        z2: f64,
        tile_x: f64,
        tile_y: f64,
    ) -> [f64; 2] {
        [
            (self.options.extent * (x * z2 - tile_x)).round(),
            (self.options.extent * (y * z2 - tile_y)).round(),
        ]
    }

    /// Get the index of the KD-tree to query for a zoom level.
    /// Above the highest zoom level to cluster on, this is the KD-tree of the points that are not clustered.
    fn tree_index(&self, zoom: u8, cluster_max_zoom: u8) -> usize {
        let unclustered = self.options.max_zoom as usize + 1;

        if zoom > cluster_max_zoom {
            unclustered
        } else {
            (zoom.max(self.options.min_zoom) as usize).min(unclustered)
        }
    }

    /// Index the points that were moved into the supercluster: normalize them, cluster them from
    /// the maximum zoom level down to the minimum one, then move them into leaf order.
    /// `get_coordinates` returns the coordinates of a point, or `None` to leave it out.
    fn index<F>(&mut self, mut get_coordinates: F)
    where
        F: FnMut(&P) -> Option<[f64; 2]>,
    {
        let min_zoom = self.options.min_zoom;
        let max_zoom = self.options.max_zoom;
        let node_size = self.options.node_size;

        // The points that are clustered, in the order they were loaded
        let mut entries = Vec::with_capacity(self.points.len());

        for (index, point) in self.points.iter().enumerate() {
            let Some(coordinates) = get_coordinates(point) else {
                continue;
            };

            let position = self.options.coordinate_system.normalize(coordinates);

            // A value that is not a finite number would break the sorting of the KD-tree
            if position.iter().all(|value| value.is_finite()) {
                entries.push(Entry {
                    position,
                    handle: index as u32,
                    point_count: 1,
                });
            }
        }

        #[cfg(feature = "tracing")]
        Self::warn_left_out_points(self.points.len(), entries.len());

        // The KD-trees from the highest zoom level down, the one of the points first
        let mut trees = Vec::with_capacity(max_zoom as usize + 2);
        let mut tree = Self::create_tree(&entries, node_size);
        let mut clusters = Clusters::new();

        // Cluster points on max zoom, then cluster the results on previous zoom, etc.;
        // Results in a cluster hierarchy across zoom levels
        for zoom in (min_zoom..=max_zoom).rev() {
            let current = self.cluster(&tree, &entries, zoom, &mut clusters);

            set_handles(&mut tree, &entries);
            trees.push(tree);

            #[cfg(feature = "tracing")]
            tracing::trace!(zoom, items = current.len(), "Clustered zoom level");

            tree = Self::create_tree(&current, node_size);
            entries = current;
        }

        set_handles(&mut tree, &entries);
        trees.push(tree);

        // The zoom levels below the minimum one have no items
        trees.extend((0..min_zoom).map(|_| KDBush::default()));
        trees.reverse();

        self.move_into_leaf_order(&entries, &mut trees, &mut clusters);

        clusters.shrink_to_fit();

        self.trees = trees;
        self.clusters = clusters;

        #[cfg(feature = "tracing")]
        tracing::debug!(
            points = self.points.len(),
            clusters = self.clusters.len(),
            min_zoom,
            max_zoom,
            "Loaded points into the supercluster"
        );
    }

    /// Create a KD-tree over the items of a zoom level, whose IDs are their indexes in `entries`.
    fn create_tree(entries: &[Entry], node_size: usize) -> KDBush {
        KDBush::new(
            entries.iter().map(|entry| entry.position).collect(),
            node_size,
        )
    }

    /// Cluster the items of a zoom level on the zoom level `zoom` before it. `tree` indexes
    /// `entries`, the items of the zoom level after `zoom`, by their index in `entries`. Every
    /// cluster that forms goes into `clusters`, with its children.
    ///
    /// Returns the items of `zoom`, in the order they form.
    fn cluster(
        &self,
        tree: &KDBush,
        entries: &[Entry],
        zoom: u8,
        clusters: &mut Clusters,
    ) -> Vec<Entry> {
        let r = self.options.radius / (self.options.extent * (2.0_f64).powi(zoom as i32));
        let min_points = u32::from(self.options.min_points);

        let mut processed = vec![false; entries.len()];
        let mut next = Vec::with_capacity(entries.len());

        // Buffers for the neighbor search, reused for every item
        let mut stack = vec![];
        let mut neighbors = vec![];

        for (i, &entry) in entries.iter().enumerate() {
            // If we've already visited the item at this zoom level, skip it
            if processed[i] {
                continue;
            }

            processed[i] = true;

            let [x, y] = entry.position;

            tree.within_into(x, y, r, &mut stack, &mut neighbors);

            // Count the points of the neighbors that no other item took yet
            let point_count = entry.point_count
                + neighbors
                    .iter()
                    .filter(|&&k| !processed[k])
                    .map(|&k| entries[k].point_count)
                    .sum::<u32>();

            // If there were neighbors to merge, and there are enough points to form a cluster
            if point_count > entry.point_count && point_count >= min_points {
                let mut wx = x * f64::from(entry.point_count);
                let mut wy = y * f64::from(entry.point_count);

                for &k in &neighbors {
                    // The item the cluster originates from is a child too, though it was marked as
                    // processed before the search
                    if k != i {
                        if processed[k] {
                            continue;
                        }

                        processed[k] = true;

                        // Accumulate coordinates for calculating weighted center
                        let [kx, ky] = entries[k].position;
                        let weight = f64::from(entries[k].point_count);

                        wx += kx * weight;
                        wy += ky * weight;
                    }

                    clusters.children.push(entries[k].handle);
                }

                let id = (self.points.len() + clusters.len()) as u32;
                let position = [wx / f64::from(point_count), wy / f64::from(point_count)];

                clusters.push(position, point_count, zoom);

                next.push(Entry {
                    position,
                    handle: id,
                    point_count,
                });
            } else {
                // Leave the item and its neighbors as they are
                next.push(entry);

                for &k in &neighbors {
                    if !processed[k] {
                        processed[k] = true;
                        next.push(entries[k]);
                    }
                }
            }
        }

        next
    }

    /// Move the points into leaf order, where the points of every cluster are next to each other,
    /// and refer to every point by its position in that order. `top` holds the items of the lowest
    /// zoom level, whose clusters hold all the others.
    fn move_into_leaf_order(
        &mut self,
        top: &[Entry],
        trees: &mut [KDBush],
        clusters: &mut Clusters,
    ) {
        let point_count = self.points.len();

        // The position of every point in leaf order, by its index in the list that was loaded
        let mut leaf_of = vec![u32::MAX; point_count];
        let mut next_leaf = 0;

        for entry in top {
            match (entry.handle as usize).checked_sub(point_count) {
                Some(cluster) => {
                    clusters.collect_leaves(cluster, point_count, &mut leaf_of, &mut next_leaf)
                }
                None => {
                    leaf_of[entry.handle as usize] = next_leaf;
                    next_leaf += 1;
                }
            }
        }

        let clustered = next_leaf as usize;

        // The points left out of the clustering come last, in the order they were loaded
        for leaf in leaf_of.iter_mut().filter(|leaf| **leaf == u32::MAX) {
            *leaf = next_leaf;
            next_leaf += 1;
        }

        let handles = trees
            .iter_mut()
            .flat_map(|tree| tree.ids.iter_mut())
            .chain(clusters.children.iter_mut());

        for handle in handles {
            if let Some(&leaf) = leaf_of.get(*handle as usize) {
                *handle = leaf;
            }
        }

        // The tree of the points holds every point that was clustered
        let points = &trees[self.options.max_zoom as usize + 1];
        let mut positions = vec![[0.0, 0.0]; clustered];

        for (&leaf, &position) in points.ids.iter().zip(&points.coords) {
            positions[leaf as usize] = position;
        }

        let mut indexes = vec![0; point_count];

        for (index, &leaf) in leaf_of.iter().enumerate() {
            indexes[leaf as usize] = index as u32;
        }

        permute(&mut self.points, &indexes);

        self.indexes = indexes;
        self.positions = positions;
    }

    /// Warn when points are left out of the clustering because their coordinates are missing or
    /// not finite numbers.
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

/// Replace the IDs of a KD-tree, which are the indexes of its items in `entries`, with the handles
/// of the items.
fn set_handles(tree: &mut KDBush, entries: &[Entry]) {
    for id in &mut tree.ids {
        *id = entries[*id as usize].handle;
    }
}

/// Move every point to its position in leaf order, where the point at position `leaf` is the one
/// that was at `indexes[leaf]`. It swaps the points along each cycle of the order, so it needs no
/// room for a second list of points.
fn permute<P>(points: &mut [P], indexes: &[u32]) {
    let mut done = vec![false; points.len()];

    for start in 0..points.len() {
        let mut leaf = start;

        while !done[leaf] {
            done[leaf] = true;

            let from = indexes[leaf] as usize;

            if from == start {
                break;
            }

            points.swap(leaf, from);
            leaf = from;
        }
    }
}

/// A view of a supercluster whose queries cluster up to a lower zoom level than the `max_zoom` of
/// its options. Above that zoom level, the queries return the individual points instead of
/// clusters. Create one with `Supercluster::with_cluster_max_zoom`.
#[derive(Debug)]
pub struct SuperclusterView<'a, P> {
    /// The supercluster to query.
    pub(crate) supercluster: &'a Supercluster<P>,

    /// The highest zoom level to cluster on, which is not above the `max_zoom` of the options.
    pub(crate) cluster_max_zoom: u8,
}

// Written by hand, because a derive would also require the point type to be `Clone` and `Copy`,
// and a view only holds a reference to the supercluster.
impl<P> Clone for SuperclusterView<'_, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Copy for SuperclusterView<'_, P> {}

impl<'a, P> SuperclusterView<'a, P> {
    /// Retrieve clustered items within the specified bounding box and zoom level, like
    /// `Supercluster::clusters`.
    pub fn clusters(&self, bbox: [f64; 4], zoom: u8) -> Vec<Item<'a, P>> {
        let mut items = vec![];

        self.clusters_into(bbox, zoom, &mut items);

        items
    }

    /// Add the clustered items within the bounding box at the zoom level to `items`.
    fn clusters_into(&self, bbox: [f64; 4], zoom: u8, items: &mut Vec<Item<'a, P>>) {
        let supercluster = self.supercluster;
        let tree = &supercluster.trees[supercluster.tree_index(zoom, self.cluster_max_zoom)];
        let mut add = |position: usize| {
            items.push(supercluster.item(tree.ids[position], tree.coords[position]))
        };

        match &supercluster.options.coordinate_system {
            CoordinateSystem::Cartesian { range } => tree.range(
                range.normalize(bbox[0]),
                range.normalize(bbox[1]),
                range.normalize(bbox[2]),
                range.normalize(bbox[3]),
                add,
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
                    self.clusters_into([min_lng, min_lat, 180.0, max_lat], zoom, items);
                    self.clusters_into([-180.0, min_lat, max_lng, max_lat], zoom, items);

                    return;
                }

                tree.range(
                    convert_longitude_to_spherical_mercator(min_lng),
                    convert_latitude_to_spherical_mercator(max_lat),
                    convert_longitude_to_spherical_mercator(max_lng),
                    convert_latitude_to_spherical_mercator(min_lat),
                    &mut add,
                )
            }
        }
    }

    /// Retrieve the items of a tile, like `Supercluster::tile`.
    pub fn tile(&self, z: u8, x: u32, y: u32) -> Tile<'a, P> {
        let supercluster = self.supercluster;
        let z2 = tiles_per_axis(z);
        let tile_y = f64::from(y);
        let mut items = vec![];

        supercluster.for_each_in_tile(
            self.cluster_max_zoom,
            (z, x, y),
            |handle, position, origin_x| {
                let [x, y] = supercluster.tile_position(position, z2, origin_x, tile_y);

                items.push(TileItem {
                    x,
                    y,
                    item: supercluster.item(handle, position),
                });
            },
        );

        Tile { items }
    }

    /// Determine the zoom level at which a cluster expands, like
    /// `Supercluster::cluster_expansion_zoom`. Above the zoom level to cluster on, this is that
    /// zoom level plus one.
    ///
    /// # Errors
    ///
    /// Returns `SuperclusterError::ClusterNotFound` if there is no such cluster.
    pub fn cluster_expansion_zoom(&self, cluster_id: ClusterId) -> Result<u8, SuperclusterError> {
        let supercluster = self.supercluster;
        let cluster = supercluster.cluster_index(cluster_id)?;

        // A cluster has at least two children, so it splits on the zoom level after it formed
        Ok((supercluster.clusters.zooms[cluster] + 1).min(self.cluster_max_zoom + 1))
    }

    /// Determine the zoom level at which a cluster expands, counting only the points that pass a
    /// filter, like `Supercluster::cluster_expansion_zoom_filtered`. Above the zoom level to
    /// cluster on, this is that zoom level plus one.
    ///
    /// # Errors
    ///
    /// Returns `SuperclusterError::ClusterNotFound` if there is no such cluster.
    pub fn cluster_expansion_zoom_filtered<F>(
        &self,
        cluster_id: ClusterId,
        filter: F,
    ) -> Result<u8, SuperclusterError>
    where
        F: Fn(&P) -> bool,
    {
        let points = self.supercluster.points();

        self.supercluster
            .expansion_zoom_where(cluster_id, self.cluster_max_zoom, |leaves| {
                points[leaves].iter().any(&filter)
            })
    }
}

/// Get the number of tiles per axis at a zoom level.
pub(crate) fn tiles_per_axis(z: u8) -> f64 {
    2f64.powi(z as i32)
}

/// Convert longitude to spherical mercator in the [0..1] range.
fn convert_longitude_to_spherical_mercator(lng: f64) -> f64 {
    lng / 360.0 + 0.5
}

/// Convert latitude to spherical mercator in the [0..1] range.
fn convert_latitude_to_spherical_mercator(lat: f64) -> f64 {
    let sin = lat.to_radians().sin();
    let y = 0.5 - (0.25 * ((1.0 + sin) / (1.0 - sin)).ln()) / PI;

    y.clamp(0.0, 1.0)
}

/// Convert spherical mercator to longitude.
fn convert_spherical_mercator_to_longitude(x: f64) -> f64 {
    (x - 0.5) * 360.0
}

/// Convert spherical mercator to latitude.
fn convert_spherical_mercator_to_latitude(y: f64) -> f64 {
    let y2 = ((180.0 - y * 360.0) * PI) / 180.0;
    (360.0 * y2.exp().atan()) / PI - 90.0
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let options = SuperclusterOptions::builder()
            .max_zoom(max_zoom)
            .build()
            .unwrap();
        let points: Vec<TestPoint> = points
            .iter()
            .map(|&(lng, lat, keep)| TestPoint::new(lng, lat, keep))
            .collect();

        Supercluster::load(options, points, |point| point.coordinates).unwrap()
    }

    /// Load test points with the default options.
    fn load_points(points: Vec<TestPoint>) -> Supercluster<TestPoint> {
        Supercluster::load(SuperclusterOptions::default(), points, |point| {
            point.coordinates
        })
        .unwrap()
    }

    /// Load the points spread over the world with the default options.
    fn load_spread_points(count: usize) -> Supercluster<[f64; 2]> {
        Supercluster::load(
            SuperclusterOptions::default(),
            create_points(count),
            |point| Some(*point),
        )
        .unwrap()
    }

    /// Get the tile that holds the point at 10, 50 (longitude, latitude).
    fn get_tile_of_point(
        supercluster: &Supercluster<TestPoint>,
        z: u8,
        cluster_max_zoom: Option<u8>,
    ) -> Tile<'_, TestPoint> {
        let z2 = tiles_per_axis(z);
        let x = (convert_longitude_to_spherical_mercator(10.0) * z2).floor() as u32;
        let y = (convert_latitude_to_spherical_mercator(50.0) * z2).floor() as u32;

        match cluster_max_zoom {
            Some(cluster_max_zoom) => supercluster
                .with_cluster_max_zoom(cluster_max_zoom)
                .tile(z, x, y),
            None => supercluster.tile(z, x, y),
        }
    }

    /// Get the ID of the cluster that holds all the points at zoom level 0.
    fn get_top_cluster_id(supercluster: &Supercluster<TestPoint>) -> ClusterId {
        match &supercluster.tile(0, 0, 0).items[0].item {
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

    /// A point type that does not implement `Clone`, which the items and the view must not require.
    #[derive(Debug, PartialEq)]
    struct Unclonable([f64; 2]);

    /// Load two close points that do not implement `Clone`.
    fn load_unclonable_points() -> Supercluster<Unclonable> {
        Supercluster::load(
            SuperclusterOptions::default(),
            vec![Unclonable([10.0, 50.0]), Unclonable([12.0, 50.0])],
            |point| Some(point.0),
        )
        .unwrap()
    }

    /// Clone a value through the `Clone` trait. A call on a `Copy` type would make clippy ask for a
    /// copy instead, and these tests check the `Clone` impls that the types write by hand.
    fn clone_of<T: Clone>(value: &T) -> T {
        value.clone()
    }

    /// Check at compile time that a type can be sent to and shared between threads.
    fn assert_send_and_sync<T: Send + Sync>() {}

    #[test]
    fn test_supercluster_is_send_and_sync() {
        assert_send_and_sync::<Supercluster<TestPoint>>();
    }

    #[test]
    fn test_load_calls_get_coordinates_once_per_point_in_order() {
        let mut seen = vec![];

        Supercluster::load(SuperclusterOptions::default(), vec![1, 2, 3], |point| {
            seen.push(*point);

            Some([0.0, 0.0])
        })
        .unwrap();

        assert_eq!(seen, vec![1, 2, 3]);
    }

    #[test]
    fn test_cluster_id_conversions() {
        let id = ClusterId::new(42);

        assert_eq!(id.get(), 42);
        assert_eq!(ClusterId::from(42), id);
        assert_eq!(u32::from(id), 42);
    }

    #[test]
    fn test_clone_of_a_tile_and_its_items_without_a_clone_point() {
        let supercluster = load_unclonable_points();
        let tile = supercluster.tile(0, 0, 0);

        assert_eq!(tile.items.len(), 1);
        assert_eq!(tile.clone(), tile);
        assert_eq!(clone_of(&tile.items[0]), tile.items[0]);
        assert_eq!(clone_of(&tile.items[0].item), tile.items[0].item);
    }

    #[test]
    fn test_clone_of_a_view_without_a_clone_point() {
        let supercluster = load_unclonable_points();
        let view = supercluster.with_cluster_max_zoom(0);

        assert_eq!(clone_of(&view).tile(1, 1, 0), view.tile(1, 1, 0));
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn test_load_with_too_many_points() {
        // A vector of a zero-sized type has a length without taking any memory
        let points = vec![(); MAX_POINTS + 1];

        assert_eq!(
            Supercluster::load(SuperclusterOptions::default(), points, |_| None).err(),
            Some(SuperclusterError::TooManyPoints)
        );
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
    fn test_points_are_in_leaf_order() {
        // The first and last point cluster, the middle one is far away
        let supercluster = load(
            16,
            &[
                (10.0, 50.0, true),
                (-100.0, -30.0, true),
                (10.001, 50.0, false),
            ],
        );
        let cluster_id = supercluster
            .clusters([-180.0, -90.0, 180.0, 90.0], 0)
            .iter()
            .find_map(|item| match item {
                Item::Cluster(cluster) => Some(cluster.id),
                Item::Point { .. } => None,
            })
            .unwrap();

        // The points of the cluster come first, next to each other, then the point on its own
        assert_eq!(
            supercluster.leaves(cluster_id).unwrap(),
            &supercluster.points()[..2]
        );
        assert_eq!(
            supercluster.points()[2],
            TestPoint::new(-100.0, -30.0, true)
        );
    }

    #[test]
    fn test_points_left_out_come_last() {
        let supercluster = load_points(vec![
            TestPoint::without_coordinates(),
            TestPoint::new(10.0, 50.0, true),
            TestPoint::new(12.0, 50.0, true),
        ]);

        assert_eq!(supercluster.points().len(), 3);
        assert_eq!(supercluster.points()[2], TestPoint::without_coordinates());
    }

    #[test]
    fn test_clusters_gives_points_their_loaded_index() {
        let points = create_points(500);
        let supercluster =
            Supercluster::load(SuperclusterOptions::default(), points.clone(), |point| {
                Some(*point)
            })
            .unwrap();

        let items = supercluster.clusters([-180.0, -90.0, 180.0, 90.0], 17);

        assert_eq!(items.len(), 500);

        for item in items {
            let Item::Point { index, point } = item else {
                panic!("expected a point");
            };

            assert_eq!(*point, points[index]);
        }
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
            Item::Cluster(cluster) => cluster.point_count as usize,
            Item::Point { .. } => 1,
        }
    }

    #[test]
    fn test_clusters_counts_every_point_on_every_zoom_level() {
        let supercluster = load_spread_points(500);

        // The highest zoom level holds the points that are not clustered
        for zoom in 0..=17 {
            let count: usize = supercluster
                .clusters([-180.0, -90.0, 180.0, 90.0], zoom)
                .iter()
                .map(get_point_count)
                .sum();

            assert_eq!(count, 500, "zoom level {zoom}");
        }
    }

    #[test]
    fn test_leaves_of_a_cluster_are_the_leaves_of_its_children() {
        let supercluster = load_spread_points(500);

        let mut clusters = 0;

        for zoom in 0..=6 {
            for item in supercluster.clusters([-180.0, -90.0, 180.0, 90.0], zoom) {
                let Item::Cluster(cluster) = item else {
                    continue;
                };

                let leaves: Vec<&[f64; 2]> =
                    supercluster.leaves(cluster.id).unwrap().iter().collect();
                let mut leaves_of_children: Vec<&[f64; 2]> = vec![];

                for child in supercluster.children(cluster.id).unwrap() {
                    match child {
                        Item::Cluster(child) => {
                            leaves_of_children.extend(supercluster.leaves(child.id).unwrap())
                        }
                        Item::Point { point, .. } => leaves_of_children.push(point),
                    }
                }

                assert_eq!(leaves.len(), cluster.point_count as usize);
                assert_eq!(leaves, leaves_of_children);

                clusters += 1;
            }
        }

        assert!(clusters > 0);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_supercluster_round_trips_through_json() {
        let supercluster = load_spread_points(300);

        let json = serde_json::to_string(&supercluster).unwrap();
        let restored: Supercluster<[f64; 2]> = serde_json::from_str(&json).unwrap();
        let bbox = [-180.0, -90.0, 180.0, 90.0];

        for zoom in [0, 4, 9, 17] {
            assert_eq!(
                restored.clusters(bbox, zoom),
                supercluster.clusters(bbox, zoom)
            );
        }

        assert_eq!(restored.tile(2, 1, 1), supercluster.tile(2, 1, 1));
    }

    #[test]
    fn test_load_without_points() {
        let supercluster = load(16, &[]);

        assert!(supercluster.tile(0, 0, 0).is_empty());
        assert!(supercluster
            .clusters([-180.0, -90.0, 180.0, 90.0], 3)
            .is_empty());
        assert!(matches!(
            supercluster.children(ClusterId::new(5)),
            Err(SuperclusterError::ClusterNotFound)
        ));
    }

    /// Get the indexes of the points that are not clustered at the highest zoom level, which are
    /// the points that were not left out.
    fn get_point_indexes(supercluster: &Supercluster<TestPoint>) -> Vec<usize> {
        let mut indexes: Vec<usize> = supercluster
            .clusters([-180.0, -90.0, 180.0, 90.0], 17)
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
        let supercluster = load_points(vec![
            TestPoint::new(10.0, 50.0, true),
            TestPoint::without_coordinates(),
            TestPoint::new(12.0, 50.0, true),
        ]);

        assert_eq!(get_point_indexes(&supercluster), vec![0, 2]);
    }

    #[test]
    fn test_load_skips_points_with_coordinates_that_are_not_finite() {
        let supercluster = load_points(vec![
            TestPoint::new(10.0, 50.0, true),
            TestPoint::new(f64::NAN, 50.0, true),
            TestPoint::new(10.0, f64::INFINITY, true),
            TestPoint::new(f64::NEG_INFINITY, 50.0, true),
            TestPoint::new(12.0, 50.0, true),
        ]);

        assert_eq!(get_point_indexes(&supercluster), vec![0, 4]);
    }

    #[test]
    fn test_tile_without_cluster_max_zoom() {
        let supercluster = load(16, &[(10.0, 50.0, true), (12.0, 50.0, true)]);
        let tile = get_tile_of_point(&supercluster, 1, None);

        assert_eq!(tile.items.len(), 1);
        assert!(matches!(&tile.items[0].item, Item::Cluster(cluster) if cluster.point_count == 2));
    }

    #[test]
    fn test_tile_at_cluster_max_zoom() {
        let supercluster = load(16, &[(10.0, 50.0, true), (12.0, 50.0, true)]);

        assert_eq!(get_tile_of_point(&supercluster, 1, Some(1)).items.len(), 1);
    }

    #[test]
    fn test_tile_above_cluster_max_zoom() {
        let supercluster = load(16, &[(10.0, 50.0, true), (12.0, 50.0, true)]);
        let tile = get_tile_of_point(&supercluster, 1, Some(0));

        assert_eq!(tile.items.len(), 2);
        assert!(tile
            .items
            .iter()
            .all(|tile_item| matches!(tile_item.item, Item::Point { .. })));
    }

    #[test]
    fn test_tile_cluster_max_zoom_above_max_zoom() {
        let supercluster = load(4, &[(10.0, 50.0, true), (12.0, 50.0, true)]);

        // The given zoom level cannot raise the maximum zoom level, so zoom level 6 reads the points that are not clustered.
        assert_eq!(get_tile_of_point(&supercluster, 6, Some(8)).items.len(), 1);
    }

    #[test]
    fn test_tile_with_cartesian_coordinates_does_not_wrap() {
        let range = DataRange::new(0.0, 0.0, 100.0, 100.0);
        let options = SuperclusterOptions::builder()
            .coordinate_system(CoordinateSystem::Cartesian { range })
            .build()
            .unwrap();
        let supercluster = Supercluster::load(
            options,
            vec![
                TestPoint::new(1.0, 1.0, true),
                TestPoint::new(99.0, 1.0, true),
            ],
            |point| point.coordinates,
        )
        .unwrap();

        // The point at 99 is near the other end of the range, so tile 0 does not show it.
        assert_eq!(supercluster.tile(1, 0, 0).items.len(), 1);
    }

    #[test]
    fn test_tile_with_lat_lng_wraps_around_the_antimeridian() {
        let supercluster = load(16, &[(1.0, 0.0, true), (179.0, 0.0, true)]);

        // Tile 0 also shows the point at 179, which sits left of the tile.
        let tile = supercluster.tile(1, 0, 0);

        assert_eq!(tile.items.len(), 2);
        assert!(tile.items.iter().any(|tile_item| tile_item.x < 0.0));
    }

    #[test]
    fn test_tile_position_of_items() {
        let supercluster = load(16, &[(0.0, 0.0, true)]);
        let tile = supercluster.tile(0, 0, 0);

        assert_eq!(tile.items.len(), 1);
        assert_eq!((tile.items[0].x, tile.items[0].y), (256.0, 256.0));
    }

    #[test]
    fn test_tile_without_loaded_points() {
        let supercluster = load_points(vec![]);

        assert!(supercluster.tile(0, 0, 0).is_empty());
    }

    #[test]
    fn test_children() {
        let supercluster = load_nested_cluster();
        let mut point_counts: Vec<u32> = supercluster
            .children(get_top_cluster_id(&supercluster))
            .unwrap()
            .map(|item| match item {
                Item::Cluster(cluster) => cluster.point_count,
                Item::Point { .. } => 1,
            })
            .collect();

        point_counts.sort();

        assert_eq!(point_counts, vec![1, 2]);
    }

    #[test]
    fn test_children_of_unknown_cluster() {
        let supercluster = load_nested_cluster();

        // Smaller than the number of points, so it is a point and not a cluster.
        assert!(matches!(
            supercluster.children(ClusterId::new(1)),
            Err(SuperclusterError::ClusterNotFound)
        ));
        // Above the IDs of the clusters.
        assert!(matches!(
            supercluster.children(ClusterId::new(100000)),
            Err(SuperclusterError::ClusterNotFound)
        ));
    }

    #[test]
    fn test_leaves() {
        let supercluster = load_nested_cluster();
        let cluster_id = get_top_cluster_id(&supercluster);

        let all: Vec<&TestPoint> = supercluster.leaves(cluster_id).unwrap().iter().collect();

        assert_eq!(supercluster.leaves(cluster_id).unwrap().len(), 3);

        // Every point is there exactly once.
        let mut keeps: Vec<bool> = all.iter().map(|point| point.keep).collect();

        keeps.sort();

        assert_eq!(keeps, vec![false, true, true]);

        // Skipping and taking select a window of the same order.
        let page = |skip, take| -> Vec<&TestPoint> {
            supercluster
                .leaves(cluster_id)
                .unwrap()
                .iter()
                .skip(skip)
                .take(take)
                .collect()
        };

        assert_eq!(page(0, 2), all[..2]);
        assert_eq!(page(1, 2), all[1..]);
        assert_eq!(page(2, 10), all[2..]);
        assert!(page(3, 10).is_empty());
    }

    #[test]
    fn test_leaves_of_nested_cluster() {
        let supercluster = load_nested_cluster();
        let nested_id = supercluster
            .children(get_top_cluster_id(&supercluster))
            .unwrap()
            .find_map(|item| match item {
                Item::Cluster(cluster) => Some(cluster.id),
                Item::Point { .. } => None,
            })
            .unwrap();

        let leaves = supercluster.leaves(nested_id).unwrap();

        assert_eq!(leaves.len(), 2);
        assert!(leaves.iter().all(|point| point.keep));
    }

    #[test]
    fn test_leaves_of_unknown_cluster() {
        let supercluster = load_nested_cluster();

        assert_eq!(
            supercluster.leaves(ClusterId::new(100000)).err(),
            Some(SuperclusterError::ClusterNotFound)
        );
    }

    #[test]
    fn test_cluster_expansion_zoom() {
        let supercluster = load_nested_cluster();
        let cluster_id = get_top_cluster_id(&supercluster);

        assert_eq!(supercluster.cluster_expansion_zoom(cluster_id), Ok(2));
    }

    #[test]
    fn test_cluster_expansion_zoom_of_unknown_cluster() {
        let supercluster = load_nested_cluster();

        assert_eq!(
            supercluster.cluster_expansion_zoom(ClusterId::new(100000)),
            Err(SuperclusterError::ClusterNotFound)
        );
    }

    #[test]
    fn test_cluster_expansion_zoom_filtered_accepting_all() {
        let supercluster = load_nested_cluster();
        let cluster_id = get_top_cluster_id(&supercluster);

        assert_eq!(
            supercluster.cluster_expansion_zoom_filtered(cluster_id, |_| true),
            Ok(2)
        );
    }

    #[test]
    fn test_cluster_expansion_zoom_filtered_rejecting_a_child() {
        let supercluster = load_nested_cluster();
        let cluster_id = get_top_cluster_id(&supercluster);

        // Without the farther point the cluster looks the same until the close points split on zoom level 17.
        assert_eq!(
            supercluster.cluster_expansion_zoom_filtered(cluster_id, |point| point.keep),
            Ok(17)
        );
    }

    #[test]
    fn test_cluster_expansion_zoom_filtered_rejecting_all() {
        let supercluster = load_nested_cluster();
        let cluster_id = get_top_cluster_id(&supercluster);

        assert_eq!(
            supercluster.cluster_expansion_zoom_filtered(cluster_id, |_| false),
            Ok(2)
        );
    }

    #[test]
    fn test_cluster_expansion_zoom_with_cluster_max_zoom() {
        let supercluster = load_nested_cluster();
        let cluster_id = get_top_cluster_id(&supercluster);

        // Above the zoom level to cluster on there is nothing left to expand.
        assert_eq!(
            supercluster
                .with_cluster_max_zoom(0)
                .cluster_expansion_zoom_filtered(cluster_id, |_| true),
            Ok(1)
        );
        assert_eq!(
            supercluster
                .with_cluster_max_zoom(5)
                .cluster_expansion_zoom_filtered(cluster_id, |_| true),
            Ok(2)
        );
        assert_eq!(
            supercluster
                .with_cluster_max_zoom(5)
                .cluster_expansion_zoom_filtered(cluster_id, |point| point.keep),
            Ok(6)
        );
        assert_eq!(
            supercluster
                .with_cluster_max_zoom(20)
                .cluster_expansion_zoom_filtered(cluster_id, |point| point.keep),
            Ok(17)
        );
    }

    #[test]
    fn test_clusters_below_min_zoom() {
        let options = SuperclusterOptions::builder().min_zoom(3).build().unwrap();
        let supercluster = Supercluster::load(
            options,
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
            supercluster.clusters(bbox, 1),
            supercluster.clusters(bbox, 3)
        );
    }

    #[test]
    fn test_clusters_with_cluster_max_zoom() {
        let supercluster = load(16, &[(10.0, 50.0, true), (12.0, 50.0, true)]);
        let bbox = [-180.0, -90.0, 180.0, 90.0];

        assert_eq!(supercluster.clusters(bbox, 1).len(), 1);

        // Above the zoom level to cluster on, the view returns the points
        let items = supercluster.with_cluster_max_zoom(0).clusters(bbox, 1);

        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| matches!(item, Item::Point { .. })));
    }

    #[test]
    fn test_clusters_has_the_center_of_the_cluster() {
        let supercluster = load(16, &[(10.0, 50.0, true), (12.0, 50.0, true)]);
        let items = supercluster.clusters([-180.0, -90.0, 180.0, 90.0], 0);

        let [Item::Cluster(cluster)] = &items[..] else {
            panic!("expected a single cluster");
        };

        let [lng, lat] = cluster.coordinates();

        assert!((lng - 11.0).abs() < 1e-9);
        assert!((lat - 50.0).abs() < 1e-9);
    }

    #[test]
    fn test_point_count_abbreviated() {
        let abbreviate = |point_count| {
            Cluster {
                id: ClusterId::new(0),
                point_count,
                position: [0.0, 0.0],
                coordinate_system: &CoordinateSystem::LatLng,
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
