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
//! The `CoordinateSystem` enum has two variants: `LngLat` for longitude and latitude coordinates and
//! `Cartesian` for Cartesian coordinates.
//!
//! Every zoom level has a KD-tree of its clusters, and one KD-tree holds the points for all zoom
//! levels, with the zoom level from which each point shows on its own. An item of a tree is a
//! `u32` handle: a point by its position in the points, or a cluster by its ID. The points are in
//! leaf order, where the points of every cluster are next to each other, so the points of a
//! cluster are a range of them. What a query needs to know about a cluster is in one table, by the
//! number of the cluster.

use std::{f64::consts::PI, fmt::Debug, ops::Range, sync::Arc, vec};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{
    kdbush::KDBush, sums::Positions, tile::every_point_tile, DataRange, SuperclusterError,
    SuperclusterOptions, Tile,
};

/// The highest maximum zoom level a supercluster supports. Building options with a higher
/// `max_zoom` is an error.
pub const MAX_ZOOM_LIMIT: u8 = 30;

/// The highest number of points a supercluster holds. A KD-tree refers to a point or a cluster with
/// a `u32`, and there are fewer clusters than points, so twice this number still fits.
pub const MAX_POINTS: usize = 1 << 31;

/// What a handle of a KD-tree refers to. The clusters come after the points, so a handle below the
/// number of points is a point, and one above it is a cluster.
#[derive(Clone, Copy)]
enum Handle {
    /// A point, by its position in the points: in the order they were loaded until loading moves
    /// them into leaf order, and in leaf order after.
    Point(usize),

    /// A cluster, by its number in the table of clusters.
    Cluster(usize),
}

impl Handle {
    /// Decode `handle`, where the clusters come after `point_count` points.
    fn decode(handle: u32, point_count: usize) -> Self {
        let handle = handle as usize;

        match handle.checked_sub(point_count) {
            Some(cluster) => Handle::Cluster(cluster),
            None => Handle::Point(handle),
        }
    }
}

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

    /// How far the points of every cluster can be from its center, in steps of `spread_step` of
    /// the zoom level it formed on, or `UNKNOWN_SPREAD`. The center of any of its points is as
    /// close, so a filtered tile skips a cluster that is further from the tile. Reading a
    /// supercluster finds them again from the children.
    #[cfg_attr(feature = "serde", serde(skip))]
    spreads: Vec<u8>,
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
            spreads: vec![],
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
            match Handle::decode(self.children[i], point_count) {
                Handle::Cluster(child) => {
                    self.collect_leaves(child, point_count, leaf_of, next_leaf)
                }
                Handle::Point(point) => {
                    leaf_of[point] = *next_leaf;
                    *next_leaf += 1;
                }
            }
        }
    }

    /// Find how far the points of every cluster are from its center. `positions` holds the points
    /// in leaf order, and `step` gets the length of one step of the spread of a cluster that formed
    /// on a zoom level.
    fn set_spreads(&mut self, positions: &[[f64; 2]], step: impl Fn(u8) -> f64) {
        let mut spreads: Vec<u8> = Vec::with_capacity(self.len());

        for cluster in 0..self.len() {
            let start = self.leaf_starts[cluster] as usize;
            let leaves = &positions[start..start + self.point_counts[cluster] as usize];
            let [x, y] = self.positions[cluster];

            // The farthest point, by the square of its distance, which saves a root per point
            let farthest = leaves.iter().fold(0.0_f64, |farthest, &[leaf_x, leaf_y]| {
                farthest.max((leaf_x - x).powi(2) + (leaf_y - y).powi(2))
            });

            // Loading keeps every spread below four radii, so only data that was read can have one
            // too wide to hold, and its clusters are then always checked
            let steps = (farthest.sqrt() / step(self.zooms[cluster])).ceil();

            spreads.push(if steps < f64::from(UNKNOWN_SPREAD) {
                steps as u8
            } else {
                UNKNOWN_SPREAD
            });
        }

        self.spreads = spreads;
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
/// The coordinate system is used to determine the range of the incoming data. More coordinate
/// systems may come, so a `match` on it needs a `_` arm.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize))]
#[non_exhaustive]
pub enum CoordinateSystem {
    /// Longitude and latitude coordinates, in that order, as in GeoJSON. Used for geospatial data.
    LngLat,

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
            CoordinateSystem::LngLat => [
                convert_longitude_to_spherical_mercator(x),
                convert_latitude_to_spherical_mercator(y),
            ],
        }
    }

    /// Convert a position from the [0..1] range the KD-trees use to coordinates.
    pub(crate) fn denormalize(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        match self {
            CoordinateSystem::Cartesian { range } => [range.denormalize(x), range.denormalize(y)],
            CoordinateSystem::LngLat => [
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
    id: ClusterId,

    /// The number of all the points of the cluster.
    total_count: u32,

    /// The center of the cluster, normalized to the [0..1] range the KD-trees use.
    position: [f64; 2],

    /// The coordinate system that converts the center into coordinates.
    coordinate_system: &'a CoordinateSystem,
}

impl Cluster<'_> {
    /// Get the unique identifier of the cluster, which `children`, `leaves` and
    /// `cluster_expansion_zoom` take.
    pub fn id(&self) -> ClusterId {
        self.id
    }

    /// Get the number of all the points of the cluster, also in a filtered tile, where
    /// `TileItem::count` counts the ones that pass.
    pub fn total_count(&self) -> u32 {
        self.total_count
    }

    /// Get the coordinates of the center of the cluster, in the coordinate system of the
    /// supercluster: the longitude and latitude for `LngLat`. They are computed on each call, so a
    /// query that does not need them does not pay for them.
    pub fn coordinates(&self) -> [f64; 2] {
        self.coordinate_system.denormalize(self.position)
    }
}

/// A cluster, or a point that is not part of a cluster at the queried zoom level.
#[derive(Debug, PartialEq)]
pub enum Item<'a, P> {
    /// A cluster of points.
    Cluster(Cluster<'a>),

    /// A point that is not clustered. More fields may come, so a pattern on it needs `..` unless
    /// it names every field.
    #[non_exhaustive]
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
/// With the `serde` feature, a supercluster can be written and read back. It is written with the
/// version of its format first, and reading rejects another version, as long as the fields keep the
/// order they were written in. Reading also checks that the data holds together, so queries cannot
/// panic or loop on data that a supercluster did not write, but it does not check that the clusters
/// are the ones `load` would form.
#[derive(Clone, Debug)]
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
    /// use, in the order of `points`. The points that were left out have none. The cached filters
    /// keep a weak reference, to sum the positions of the points that pass. They are written as a
    /// list.
    positions: Arc<Positions>,

    /// KD-trees of the clusters of each zoom level, from 0 to the maximum zoom level, whose IDs
    /// are the handles of the clusters. The ones below the minimum zoom level are empty.
    trees: Vec<KDBush>,

    /// The KD-tree of every point that was clustered, for all zoom levels, whose IDs are the
    /// handles of the points. A zoom level shows the points whose `shown_from` is not above it. The
    /// points of every leaf node come in the order of their `shown_from`, lowest first, so a search
    /// stops reading a leaf at its first point that its zoom level does not show.
    point_tree: KDBush,

    /// The zoom level from which every point of `point_tree`, in the order of the tree, shows on
    /// its own: the one after the zoom level of the cluster it is a child of, or 0 for a point that
    /// no cluster holds. Reading a supercluster finds them again from the clusters.
    shown_from: Vec<u8>,

    /// The lowest `shown_from` under every node of `point_tree`, so a search skips the nodes whose
    /// points are all in clusters on its zoom level.
    node_shown_from: Vec<u8>,

    /// The clusters of every zoom level.
    clusters: Clusters,

    /// How far past a tile a filtered tile searches the KD-tree of the clusters of each zoom level,
    /// to find every cluster that can move into it: the widest spread of a cluster in the tree,
    /// normalized.
    reaches: Vec<f64>,
}

/// Writes a supercluster with `serde` and reads it back. The data is written with the version of
/// its format first, so a reader that gets the fields in that order rejects another layout before
/// it reads the rest. Serde reads the fields in the order they come, so data whose fields were
/// reordered, for example by a tool that sorts keys, can fail on another field first. The data is
/// checked after it is read, so queries do not index past it.
#[cfg(feature = "serde")]
mod format {
    use std::sync::Arc;

    use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};

    use super::{Clusters, Handle, Supercluster};
    use crate::{kdbush::KDBush, sums::Positions, SuperclusterOptions};

    /// The version of the format. Raise it with every change to what a supercluster writes.
    const VERSION: u32 = 1;

    /// The version of the format, which writes `VERSION` and reads only `VERSION`.
    struct Version;

    impl Serialize for Version {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_u32(VERSION)
        }
    }

    impl<'de> Deserialize<'de> for Version {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let version = u32::deserialize(deserializer)?;

            if version == VERSION {
                Ok(Version)
            } else {
                Err(D::Error::custom(format_args!(
                    "The supercluster has format version {version}, and only version {VERSION} can be read."
                )))
            }
        }
    }

    /// What a supercluster writes: the version, then its fields, with the positions as a list.
    #[derive(Serialize)]
    struct Written<'a, P> {
        /// The version of the format.
        version: Version,

        /// The options of the supercluster.
        options: &'a SuperclusterOptions,

        /// The points, in leaf order.
        points: &'a [P],

        /// The index of every point in the list that was loaded.
        indexes: &'a [u32],

        /// The positions of the points that were clustered.
        positions: &'a [[f64; 2]],

        /// The KD-trees of the clusters of every zoom level.
        trees: &'a [KDBush],

        /// The KD-tree of the points.
        point_tree: &'a KDBush,

        /// The clusters of every zoom level.
        clusters: &'a Clusters,
    }

    /// What reading gets back, before it is checked.
    #[derive(Deserialize)]
    struct Read<P> {
        /// The version of the format, which reading checks when it gets to it, before the fields
        /// after it.
        #[serde(rename = "version")]
        _version: Version,

        /// The options of the supercluster, which reading checks like `build` does.
        options: SuperclusterOptions,

        /// The points, in leaf order.
        points: Vec<P>,

        /// The index of every point in the list that was loaded.
        indexes: Vec<u32>,

        /// The positions of the points that were clustered, which build the running sums again.
        positions: Vec<[f64; 2]>,

        /// The KD-trees of the clusters of every zoom level.
        trees: Vec<KDBush>,

        /// The KD-tree of the points.
        point_tree: KDBush,

        /// The clusters of every zoom level.
        clusters: Clusters,
    }

    impl<P: Serialize> Serialize for Supercluster<P> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            Written {
                version: Version,
                options: &self.options,
                points: &self.points,
                indexes: &self.indexes,
                positions: &self.positions,
                trees: &self.trees,
                point_tree: &self.point_tree,
                clusters: &self.clusters,
            }
            .serialize(serializer)
        }
    }

    impl<'de, P: Deserialize<'de>> Deserialize<'de> for Supercluster<P> {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let read = Read::deserialize(deserializer)?;
            let mut supercluster = Supercluster {
                options: read.options,
                points: read.points,
                indexes: read.indexes,
                positions: Arc::new(Positions::new(read.positions)),
                trees: read.trees,
                point_tree: read.point_tree,
                shown_from: vec![],
                node_shown_from: vec![],
                clusters: read.clusters,
                reaches: vec![],
            };

            match supercluster.inconsistency() {
                Some(reason) => Err(D::Error::custom(format_args!(
                    "The supercluster data does not hold together: {reason}."
                ))),
                None => {
                    let options = &supercluster.options;

                    supercluster
                        .clusters
                        .set_spreads(&supercluster.positions, |zoom| {
                            super::spread_step(options, zoom)
                        });

                    supercluster.set_shown_from();
                    supercluster.set_reaches();

                    Ok(supercluster)
                }
            }
        }
    }

    impl<P> Supercluster<P> {
        /// Find what keeps the data from holding together, the way `load` builds it, where a query
        /// would index past it or loop. It does not check where the items are or how they cluster,
        /// which a query does not depend on.
        fn inconsistency(&self) -> Option<&'static str> {
            let points = self.points.len();
            let clustered = self.positions.len();
            let clusters = &self.clusters;
            let count = clusters.len();

            // A handle is a point that was clustered, or a cluster of the table
            let is_handle = |handle: u32| match self.handle(handle) {
                Handle::Point(point) => point < clustered,
                Handle::Cluster(cluster) => cluster < count,
            };

            if points > super::MAX_POINTS || count > points {
                return Some("there are more points or clusters than a supercluster holds");
            }

            if self.indexes.len() != points || self.indexes.iter().any(|&i| i as usize >= points) {
                return Some("the indexes do not match the points");
            }

            if clustered > points {
                return Some("there are more positions than points");
            }

            if self.trees.len() != usize::from(self.options.max_zoom) + 1 {
                return Some("the number of KD-trees does not match the maximum zoom level");
            }

            // Every tree of clusters holds clusters, and the tree of the points holds every point
            // that was clustered, so finding where they show reads within the points
            let is_cluster = |handle| matches!(self.handle(handle), Handle::Cluster(_));
            let trees = self.trees.iter().map(|tree| (tree, true));

            for (tree, of_clusters) in trees.chain([(&self.point_tree, false)]) {
                if tree.ids.len() != tree.coords.len()
                    || !tree
                        .ids
                        .iter()
                        .all(|&handle| is_handle(handle) && is_cluster(handle) == of_clusters)
                {
                    return Some("a KD-tree is not valid or refers to an item that does not exist");
                }
            }

            if self.point_tree.ids.len() != clustered {
                return Some("the KD-tree of the points does not hold every point");
            }

            // The number of clusters is the length of their point counts, and every cluster has
            // one start of its children, followed by the end of the children of the last one
            if [
                clusters.positions.len(),
                clusters.zooms.len(),
                clusters.leaf_starts.len(),
            ]
            .iter()
            .any(|&length| length != count)
                || clusters.child_starts.len() != count + 1
                || clusters
                    .child_starts
                    .windows(2)
                    .any(|pair| pair[0] > pair[1])
                || clusters.child_starts.last().map(|&end| end as usize)
                    != Some(clusters.children.len())
            {
                return Some("the table of clusters is not valid");
            }

            for cluster in 0..count {
                let leaves = u64::from(clusters.leaf_starts[cluster])
                    + u64::from(clusters.point_counts[cluster]);

                if clusters.zooms[cluster] > self.options.max_zoom || leaves > clustered as u64 {
                    return Some("a cluster has a zoom level or points that do not exist");
                }

                // The children of a cluster formed before it, so finding where it expands ends, and
                // their points follow each other and make up its points, so a query of a child
                // reads within what it read for the cluster
                let mut next_leaf = clusters.leaf_starts[cluster] as usize;

                for &child in clusters.children_of(cluster) {
                    let is_earlier = match self.handle(child) {
                        Handle::Point(_) => true,
                        Handle::Cluster(child) => child < cluster,
                    };

                    if !is_handle(child) || !is_earlier {
                        return Some("a cluster has a child that does not exist");
                    }

                    let child_leaves = self.leaf_range(child);

                    if child_leaves.start != next_leaf {
                        return Some("the points of a cluster are not the points of its children");
                    }

                    next_leaf = child_leaves.end;
                }

                if next_leaf as u64 != leaves {
                    return Some("the points of a cluster are not the points of its children");
                }
            }

            None
        }
    }
}

impl<P> Supercluster<P> {
    /// Load the points into a new supercluster, which clusters them on every zoom level.
    /// The points are clustered on the maximum zoom level, and the results are clustered on previous
    /// zoom levels, which results in a cluster hierarchy across zoom levels.
    ///
    /// `get_coordinates` returns the coordinates of a point in the coordinate system of the options,
    /// the longitude and latitude for `LngLat`. It runs once for every point, in order. A point that
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
            positions: Arc::new(Positions::new(vec![])),
            trees: vec![],
            point_tree: KDBush::default(),
            shown_from: vec![],
            node_shown_from: vec![],
            clusters: Clusters::new(),
            reaches: vec![],
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
    ///
    /// The items come in no set order. The same query on the same supercluster returns them in the
    /// same order, but a minor release may change it, so sort them if you need an order.
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
    /// if let Item::Cluster(cluster) = supercluster.tile(0, 0, 0).items[0].item() {
    ///     // The second page of two points holds the last point
    ///     let page: Vec<_> = supercluster.leaves(cluster.id())?.iter().skip(2).take(2).collect();
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
    /// on its edge are not cut off. For `LngLat`, the first and last tile also hold the items of the
    /// opposite edge, across the antimeridian. `Cartesian` does not wrap, because the two ends of
    /// its `DataRange` are not next to each other. A tile with nothing in it has no items.
    ///
    /// The items come in no set order. The same query on the same supercluster returns them in the
    /// same order, but a minor release may change it, so sort them if you need an order.
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
    ///     match tile_item.item() {
    ///         Item::Cluster(cluster) => println!("{} points", cluster.total_count()),
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

    /// Get the number of a cluster in the table of clusters from its ID, or an error if no cluster
    /// has the ID.
    fn cluster_index(&self, cluster_id: ClusterId) -> Result<usize, SuperclusterError> {
        match self.handle(cluster_id.0) {
            Handle::Cluster(cluster) if cluster < self.clusters.len() => Ok(cluster),
            _ => Err(SuperclusterError::ClusterNotFound),
        }
    }

    /// Decode a handle of a KD-tree into the point or the cluster it refers to.
    fn handle(&self, handle: u32) -> Handle {
        Handle::decode(handle, self.points.len())
    }

    /// Get the positions in `points` of the points of an item of a KD-tree: the point itself, or
    /// the points of a cluster.
    pub(crate) fn leaf_range(&self, handle: u32) -> Range<usize> {
        match self.handle(handle) {
            Handle::Cluster(cluster) => {
                let start = self.clusters.leaf_starts[cluster] as usize;

                start..start + self.clusters.point_counts[cluster] as usize
            }
            Handle::Point(leaf) => leaf..leaf + 1,
        }
    }

    /// Check whether an item of a KD-tree at `position` can end up in `bounds` when a filter moves
    /// it to the center of some of its points. A point stays where it is, and a cluster stays
    /// within its spread.
    pub(crate) fn can_end_up_in(&self, handle: u32, position: [f64; 2], bounds: Bounds) -> bool {
        bounds.contains(position)
            || match self.handle(handle) {
                Handle::Cluster(cluster) => bounds.distance_to(position) <= self.spread_of(cluster),
                Handle::Point(_) => false,
            }
    }

    /// Get how far the center of some points of a cluster can be from its center, normalized, or
    /// infinity when its spread is unknown.
    fn spread_of(&self, cluster: usize) -> f64 {
        match self.clusters.spreads[cluster] {
            UNKNOWN_SPREAD => f64::INFINITY,
            spread => spread_length(
                spread,
                spread_step(&self.options, self.clusters.zooms[cluster]),
            ),
        }
    }

    /// Find how far past a tile a filtered tile searches the KD-tree of the clusters of each zoom
    /// level: the widest spread of a cluster in it. Points do not move, so the search of the points
    /// does not reach past the tile.
    fn set_reaches(&mut self) {
        self.reaches = self
            .trees
            .iter()
            .map(|tree| {
                tree.ids
                    .iter()
                    .filter_map(|&handle| match self.handle(handle) {
                        Handle::Cluster(cluster) => Some(self.spread_of(cluster)),
                        Handle::Point(_) => None,
                    })
                    .fold(0.0, f64::max)
            })
            .collect();
    }

    /// Get the normalized position of an item of a KD-tree, from the table of clusters or of the
    /// points.
    fn position(&self, handle: u32) -> [f64; 2] {
        match self.handle(handle) {
            Handle::Cluster(cluster) => self.clusters.positions[cluster],
            Handle::Point(leaf) => self.positions[leaf],
        }
    }

    /// Get the positions of the points that were clustered, with their running sums, which the
    /// filtered queries and the cached filters use.
    pub(crate) fn positions(&self) -> &Arc<Positions> {
        &self.positions
    }

    /// Get the number of points that were clustered, which come before the ones left out in
    /// `points`.
    pub(crate) fn clustered_points(&self) -> usize {
        self.positions.len()
    }

    /// Determine the zoom level at which the cluster at `cluster` in the table of clusters expands,
    /// up to `cluster_max_zoom` plus one.
    fn expansion_zoom_of(&self, cluster: usize, cluster_max_zoom: u8) -> u8 {
        // A cluster has at least two children, so it splits on the zoom level after it formed
        (self.clusters.zooms[cluster] + 1).min(cluster_max_zoom + 1)
    }

    /// Determine the zoom level at which an item of a KD-tree expands when all of its points
    /// count, up to `cluster_max_zoom` plus one, or `None` for a point.
    pub(crate) fn expansion_zoom_of_handle(&self, handle: u32, cluster_max_zoom: u8) -> Option<u8> {
        match self.handle(handle) {
            Handle::Cluster(cluster) => Some(self.expansion_zoom_of(cluster, cluster_max_zoom)),
            Handle::Point(_) => None,
        }
    }

    /// Convert an item of a KD-tree into a cluster or a point, at the position the table of
    /// clusters or of the points has for it.
    pub(crate) fn item_of(&self, handle: u32) -> Item<'_, P> {
        self.item(handle, self.position(handle))
    }

    /// Convert an item of a KD-tree, at its normalized `position`, into a cluster or a point.
    pub(crate) fn item(&self, handle: u32, position: [f64; 2]) -> Item<'_, P> {
        match self.handle(handle) {
            Handle::Cluster(cluster) => Item::Cluster(Cluster {
                id: ClusterId(handle),
                total_count: self.clusters.point_counts[cluster],
                position,
                coordinate_system: &self.options.coordinate_system,
            }),
            Handle::Point(leaf) => Item::Point {
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
                (Some(&child), None) => match self.handle(child) {
                    Handle::Cluster(child) => cluster = child,
                    Handle::Point(_) => return Ok(origin_zoom),
                },
                _ => return Ok(origin_zoom),
            }
        }
    }

    /// Call `visit` with the handle and the normalized position of every item of the zoom level at
    /// `index` in a box, as the left, top, right and bottom edge, edges included: first the
    /// clusters, within `reach` of the box, then the points. The index past the maximum zoom level
    /// has every point and no clusters.
    pub(crate) fn tree_range(
        &self,
        index: usize,
        [min_x, min_y, max_x, max_y]: [f64; 4],
        reach: f64,
        mut visit: impl FnMut(u32, [f64; 2]),
    ) {
        let node_size = self.options.node_size;

        if let Some(tree) = self.trees.get(index) {
            tree.range(
                node_size,
                min_x - reach,
                min_y - reach,
                max_x + reach,
                max_y + reach,
                |at| visit(tree.ids[at], tree.coords[at]),
            );
        }

        // The index is at most the maximum zoom level plus one, which a `u8` holds
        let zoom = index as u8;
        let tree = &self.point_tree;

        tree.range_where(
            node_size,
            [min_x, min_y, max_x, max_y],
            |node| {
                self.node_shown_from
                    .get(node)
                    .is_some_and(|&from| from <= zoom)
            },
            |at| self.shown_from[at] <= zoom,
            |at| visit(tree.ids[at], tree.coords[at]),
        );
    }

    /// Call `visit` with the handle and the normalized position of every item of the tile at zoom
    /// level `z`, `x` tiles from the left and `y` tiles from the top, with the X of the tile the
    /// position is relative to, and with the bounds of the tile and its buffer. That X is a world
    /// width away from `x` for the items that the first and last tile show from across the
    /// antimeridian.
    ///
    /// `items` tells whether the caller moves clusters. If it does, it also gets the clusters close
    /// enough to move into the bounds, and keeps the items that end up in them.
    pub(crate) fn for_each_in_tile(
        &self,
        cluster_max_zoom: u8,
        (z, x, y): (u8, u32, u32),
        items: TileItems,
        mut visit: impl FnMut(u32, [f64; 2], f64, Bounds),
    ) {
        let index = self.tree_index(z, cluster_max_zoom);
        let (x, y) = (f64::from(x), f64::from(y));
        let z2 = tiles_per_axis(z);
        let p = self.options.radius / f64::from(self.options.extent);
        let top = (y - p) / z2;
        let bottom = (y + 1.0 + p) / z2;

        // A moved cluster is at most its spread from where the KD-tree has it
        let reach = match items {
            TileItems::StayInPlace => 0.0,
            TileItems::MoveClusters => self.reaches.get(index).copied().unwrap_or(0.0),
        };

        let mut visit_range = |min_x: f64, max_x: f64, origin_x: f64| {
            let bounds = Bounds {
                min_x,
                min_y: top,
                max_x,
                max_y: bottom,
            };

            self.tree_range(
                index,
                [min_x, top, max_x, bottom],
                reach,
                |handle, position| visit(handle, position, origin_x, bounds),
            );
        };

        visit_range((x - p) / z2, (x + 1.0 + p) / z2, x);

        // For `LngLat`, the first and last tile also show the other edge.
        // The two ends of a `Cartesian` range are not next to each other, so they do not wrap.
        if self.options.coordinate_system == CoordinateSystem::LngLat {
            if x == 0.0 {
                visit_range(1.0 - p / z2, 1.0, z2);
            }

            if x == z2 - 1.0 {
                visit_range(0.0, p / z2, -1.0);
            }
        }
    }

    /// Get the position, on the grid of the tile extent, of a point that is normalized to the
    /// [0..1] range, in the tile at `tile_x`, `tile_y`, where `z2` is the number of tiles per axis.
    /// It rounds to the nearest whole number, and rounds up at a half, so -2.5 is -2. An item of a
    /// tile is at most the radius outside it, which `build` checks an `i32` holds.
    pub(crate) fn tile_position(
        &self,
        [x, y]: [f64; 2],
        z2: f64,
        tile_x: f64,
        tile_y: f64,
    ) -> [i32; 2] {
        let extent = f64::from(self.options.extent);

        [
            (extent * (x * z2 - tile_x) + 0.5).floor() as i32,
            (extent * (y * z2 - tile_y) + 0.5).floor() as i32,
        ]
    }

    /// Get the index of the zoom level whose items a query for `zoom` gets, for `tree_range`. Above
    /// the highest zoom level to cluster on, this is the one past the maximum zoom level, where
    /// every point shows on its own.
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

        // Cluster points on max zoom, then cluster the results on previous zoom, etc.;
        // Results in a cluster hierarchy across zoom levels. The tree of the points is kept for
        // all zoom levels, and the KD-trees of the clusters go from the highest zoom level down.
        let mut point_tree = Self::create_tree(&entries, node_size);
        let mut clusters = Clusters::new();
        let mut current = self.cluster(&point_tree, &entries, max_zoom, &mut clusters);
        let mut trees = Vec::with_capacity(max_zoom as usize + 1);

        set_handles(&mut point_tree, &entries);

        #[cfg(feature = "tracing")]
        tracing::trace!(
            zoom = max_zoom,
            items = current.len(),
            "Clustered zoom level"
        );

        for zoom in (min_zoom..max_zoom).rev() {
            // A tree of every item of the zoom level, to cluster them, which only lives as long
            // as that
            let tree = Self::create_tree(&current, node_size);
            let next = self.cluster(&tree, &current, zoom, &mut clusters);

            trees.push(Self::create_cluster_tree(
                &current,
                node_size,
                self.points.len(),
            ));

            #[cfg(feature = "tracing")]
            tracing::trace!(zoom, items = next.len(), "Clustered zoom level");

            current = next;
        }

        trees.push(Self::create_cluster_tree(
            &current,
            node_size,
            self.points.len(),
        ));

        // The zoom levels below the minimum one have no clusters
        trees.extend((0..min_zoom).map(|_| KDBush::default()));
        trees.reverse();

        self.move_into_leaf_order(&current, &mut trees, &mut point_tree, &mut clusters);

        clusters.set_spreads(&self.positions, |zoom| spread_step(&self.options, zoom));
        clusters.shrink_to_fit();

        self.trees = trees;
        self.point_tree = point_tree;
        self.clusters = clusters;
        self.set_shown_from();
        self.set_reaches();

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

    /// Create a KD-tree over the clusters of the items of a zoom level, whose IDs are their
    /// handles. A handle from `point_count` up is a cluster.
    fn create_cluster_tree(entries: &[Entry], node_size: usize, point_count: usize) -> KDBush {
        let clusters: Vec<Entry> = entries
            .iter()
            .filter(|entry| entry.handle as usize >= point_count)
            .copied()
            .collect();
        let mut tree = Self::create_tree(&clusters, node_size);

        set_handles(&mut tree, &clusters);
        tree
    }

    /// Find the zoom level from which every point of the KD-tree of the points shows on its own,
    /// from the clusters it is a child of, sort the points of every leaf node by it, and find the
    /// lowest one under every node of the tree.
    fn set_shown_from(&mut self) {
        let clusters = &self.clusters;
        let mut by_leaf = vec![0; self.positions.len()];

        for cluster in 0..clusters.len() {
            for &child in clusters.children_of(cluster) {
                if let Handle::Point(leaf) = self.handle(child) {
                    by_leaf[leaf] = clusters.zooms[cluster] + 1;
                }
            }
        }

        let node_size = self.options.node_size;
        let mut shown_from: Vec<u8> = self
            .point_tree
            .ids
            .iter()
            .map(|&leaf| by_leaf[leaf as usize])
            .collect();

        self.point_tree
            .sort_leaves_by_key(node_size, &mut shown_from);
        self.node_shown_from = self.point_tree.node_minimums(node_size, &shown_from);
        self.shown_from = shown_from;
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
        let r = cluster_radius(&self.options, zoom);
        let min_points = self.options.min_points;

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

            tree.within_into(self.options.node_size, x, y, r, &mut stack, &mut neighbors);

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
        point_tree: &mut KDBush,
        clusters: &mut Clusters,
    ) {
        let point_count = self.points.len();

        // The position of every point in leaf order, by its index in the list that was loaded
        let mut leaf_of = vec![u32::MAX; point_count];
        let mut next_leaf = 0;

        for entry in top {
            match Handle::decode(entry.handle, point_count) {
                Handle::Cluster(cluster) => {
                    clusters.collect_leaves(cluster, point_count, &mut leaf_of, &mut next_leaf)
                }
                Handle::Point(point) => {
                    leaf_of[point] = next_leaf;
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
            .chain([&mut *point_tree])
            .flat_map(|tree| tree.ids.iter_mut())
            .chain(clusters.children.iter_mut());

        for handle in handles {
            if let Some(&leaf) = leaf_of.get(*handle as usize) {
                *handle = leaf;
            }
        }

        // The tree of the points holds every point that was clustered
        let mut positions = vec![[0.0, 0.0]; clustered];

        for (&leaf, &position) in point_tree.ids.iter().zip(&point_tree.coords) {
            positions[leaf as usize] = position;
        }

        let mut indexes = vec![0; point_count];

        for (index, &leaf) in leaf_of.iter().enumerate() {
            indexes[leaf as usize] = index as u32;
        }

        permute(&mut self.points, &indexes);

        self.indexes = indexes;
        self.positions = Arc::new(Positions::new(positions));
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
        let index = supercluster.tree_index(zoom, self.cluster_max_zoom);
        let mut add =
            |handle: u32, position: [f64; 2]| items.push(supercluster.item(handle, position));

        match &supercluster.options.coordinate_system {
            CoordinateSystem::Cartesian { range } => supercluster.tree_range(
                index,
                [
                    range.normalize(bbox[0]),
                    range.normalize(bbox[1]),
                    range.normalize(bbox[2]),
                    range.normalize(bbox[3]),
                ],
                0.0,
                add,
            ),
            CoordinateSystem::LngLat => {
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

                supercluster.tree_range(
                    index,
                    [
                        convert_longitude_to_spherical_mercator(min_lng),
                        convert_latitude_to_spherical_mercator(max_lat),
                        convert_longitude_to_spherical_mercator(max_lng),
                        convert_latitude_to_spherical_mercator(min_lat),
                    ],
                    0.0,
                    &mut add,
                )
            }
        }
    }

    /// Retrieve the items of a tile, like `Supercluster::tile`.
    pub fn tile(&self, z: u8, x: u32, y: u32) -> Tile<'a, P> {
        every_point_tile(*self, (z, x, y))
    }

    /// Determine the zoom level at which a cluster expands, like
    /// `Supercluster::cluster_expansion_zoom`. Above the zoom level to cluster on, this is that
    /// zoom level plus one.
    ///
    /// # Errors
    ///
    /// Returns `SuperclusterError::ClusterNotFound` if there is no such cluster.
    pub fn cluster_expansion_zoom(&self, cluster_id: ClusterId) -> Result<u8, SuperclusterError> {
        let cluster = self.supercluster.cluster_index(cluster_id)?;

        Ok(self
            .supercluster
            .expansion_zoom_of(cluster, self.cluster_max_zoom))
    }
}

/// Get the number of tiles per axis at a zoom level.
pub(crate) fn tiles_per_axis(z: u8) -> f64 {
    2f64.powi(z as i32)
}

/// Whether the items of a tile stay where the KD-tree has them.
pub(crate) enum TileItems {
    /// Every item stays where it is, so the search of the tile and its buffer finds them all.
    StayInPlace,

    /// A cluster moves to the center of some of its points, so the search also finds the clusters
    /// that can move into the tile and its buffer.
    MoveClusters,
}

/// The normalized bounds of a tile and its buffer, edges included, as a KD-tree searches them.
#[derive(Clone, Copy)]
pub(crate) struct Bounds {
    /// The left edge.
    min_x: f64,

    /// The top edge.
    min_y: f64,

    /// The right edge.
    max_x: f64,

    /// The bottom edge.
    max_y: f64,
}

impl Bounds {
    /// Check whether a normalized position is in the bounds, the same way a KD-tree checks it.
    pub(crate) fn contains(&self, [x, y]: [f64; 2]) -> bool {
        x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y
    }

    /// Get the distance from a normalized position to the nearest point of the bounds, or 0 when
    /// it is in them.
    fn distance_to(&self, [x, y]: [f64; 2]) -> f64 {
        let dx = (self.min_x - x).max(x - self.max_x).max(0.0);
        let dy = (self.min_y - y).max(y - self.max_y).max(0.0);

        dx.hypot(dy)
    }
}

/// The steps of the spread of a cluster per radius it formed within. The children of a cluster are
/// at most the radius from the first of them, and so from its center, which lies between them, at
/// most twice the radius. Their points are as far from them again, at half the radius per zoom
/// level, so the points of a cluster are less than four radii from its center. Its spread takes at
/// most 252 steps, which fits a `u8` with room to round up.
const SPREAD_STEPS_PER_RADIUS: f64 = 63.0;

/// The spread of a cluster that is too wide to hold in steps, which only data that was read can
/// have. A cluster with it is always checked.
const UNKNOWN_SPREAD: u8 = u8::MAX;

/// How far past its spread the center of some points of a cluster can be, because that center is
/// found from integers that are up to the width of the area over 2^50 off.
const SPREAD_PADDING: f64 = 1.0 / (1u64 << 40) as f64;

/// Get the normalized radius that clusters form within on a zoom level.
fn cluster_radius(options: &SuperclusterOptions, zoom: u8) -> f64 {
    options.radius / (f64::from(options.extent) * tiles_per_axis(zoom))
}

/// Get the length of one step of the spread of a cluster that formed on a zoom level.
fn spread_step(options: &SuperclusterOptions, zoom: u8) -> f64 {
    cluster_radius(options, zoom) / SPREAD_STEPS_PER_RADIUS
}

/// Get the normalized length of a spread, with room for how far a center can be off.
fn spread_length(spread: u8, step: f64) -> f64 {
    f64::from(spread) * step + SPREAD_PADDING
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

    use crate::test_points::{create_spread_points, load_spread_points, TestPoint};

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
        match &supercluster.tile(0, 0, 0).items[0].item() {
            Item::Cluster(cluster) => cluster.id(),
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
        assert_eq!(clone_of(&tile.items[0].item()), tile.items[0].item());
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
                Item::Cluster(cluster) => Some(cluster.id()),
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
        let points = create_spread_points(500);
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

    /// Get the number of points an item stands for.
    fn get_point_count(item: &Item<'_, [f64; 2]>) -> usize {
        match item {
            Item::Cluster(cluster) => cluster.total_count() as usize,
            Item::Point { .. } => 1,
        }
    }

    #[test]
    fn test_clusters_counts_every_point_on_every_zoom_level() {
        let supercluster = load_spread_points(500);
        let options = SuperclusterOptions::builder().min_zoom(3).build().unwrap();
        let from_zoom_3 =
            Supercluster::load(options, create_spread_points(500), |point| Some(*point)).unwrap();

        // Every point shows once, on its own or in a cluster, also below the minimum zoom level,
        // above the zoom level a view clusters up to, and on the zoom level that holds the points
        // that are not clustered
        for view in [
            supercluster.view(),
            supercluster.with_cluster_max_zoom(5),
            from_zoom_3.view(),
        ] {
            for zoom in 0..=17 {
                let count: usize = view
                    .clusters([-180.0, -90.0, 180.0, 90.0], zoom)
                    .iter()
                    .map(get_point_count)
                    .sum();

                assert_eq!(count, 500, "zoom level {zoom}");
            }
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
                    supercluster.leaves(cluster.id()).unwrap().iter().collect();
                let mut leaves_of_children: Vec<&[f64; 2]> = vec![];

                for child in supercluster.children(cluster.id()).unwrap() {
                    match child {
                        Item::Cluster(child) => {
                            leaves_of_children.extend(supercluster.leaves(child.id()).unwrap())
                        }
                        Item::Point { point, .. } => leaves_of_children.push(point),
                    }
                }

                assert_eq!(leaves.len(), cluster.total_count() as usize);
                assert_eq!(leaves, leaves_of_children);

                clusters += 1;
            }
        }

        assert!(clusters > 0);
    }

    #[test]
    fn test_load_finds_how_far_the_points_of_every_cluster_spread() {
        let distance = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
        let supercluster = load_spread_points(2000);
        let clusters = &supercluster.clusters;

        assert!(clusters.len() > 0);
        assert_eq!(clusters.spreads.len(), clusters.len());

        for (cluster, &spread) in clusters.spreads.iter().enumerate() {
            let handle = (supercluster.points.len() + cluster) as u32;
            let step = spread_step(&supercluster.options, clusters.zooms[cluster]);

            // Below four radii, so it is never unknown
            assert!(spread <= 252);

            for &position in &supercluster.positions[supercluster.leaf_range(handle)] {
                assert!(
                    distance(clusters.positions[cluster], position) <= spread_length(spread, step)
                );
            }
        }
    }

    #[test]
    fn test_filtered_tile_finds_the_items_that_a_search_of_every_cluster_finds() {
        let supercluster = load_spread_points(3000);
        let mut everywhere = supercluster.clone();

        // A search of the whole of every tree
        for reach in &mut everywhere.reaches {
            *reach = f64::INFINITY;
        }

        let summarize = |supercluster: &Supercluster<[f64; 2]>, z, x, y| {
            let filter = supercluster.filter(|point: &[f64; 2]| point[0] > point[1]);
            let mut items: Vec<(i32, i32, u32)> = supercluster
                .with_filter(filter)
                .unwrap()
                .tile(z, x, y)
                .items
                .iter()
                .map(|tile_item| (tile_item.x(), tile_item.y(), tile_item.count()))
                .collect();

            // A wider search visits the items in another order
            items.sort_unstable();
            items
        };

        for z in 0..=5 {
            for x in 0..1 << z {
                for y in 0..1 << z {
                    assert_eq!(
                        summarize(&supercluster, z, x, y),
                        summarize(&everywhere, z, x, y),
                        "tile {z}/{x}/{y}"
                    );
                }
            }
        }
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_deserialize_finds_the_spreads_of_the_clusters_again() {
        let supercluster = load_spread_points(300);
        let json = serde_json::to_string(&supercluster).unwrap();
        let restored: Supercluster<[f64; 2]> = serde_json::from_str(&json).unwrap();

        assert!(!restored.clusters.spreads.is_empty());
        assert_eq!(restored.clusters.spreads, supercluster.clusters.spreads);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_deserialize_reads_back_json() {
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

        // The filtered clusters move to the same centers, from the sums that were read back
        let filter = |point: &[f64; 2]| point[0] > 0.0;
        let centers = |supercluster: &Supercluster<[f64; 2]>| -> Vec<[f64; 2]> {
            supercluster
                .with_filter(supercluster.filter(filter))
                .unwrap()
                .tile(0, 0, 0)
                .items
                .iter()
                .map(|item| item.coordinates())
                .collect()
        };

        assert_eq!(centers(&restored), centers(&supercluster));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_serialize_writes_only_the_loaded_data() {
        let json = serde_json::to_value(load_spread_points(10)).unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();

        // The sums are built again when it is read, so they always match the positions
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "clusters",
                "indexes",
                "options",
                "point_tree",
                "points",
                "positions",
                "trees",
                "version"
            ]
        );
    }

    /// Write a supercluster of 300 points as JSON, change the JSON with `change`, and read it back.
    #[cfg(feature = "serde")]
    fn read_changed(
        change: impl FnOnce(&mut serde_json::Value),
    ) -> Result<Supercluster<[f64; 2]>, serde_json::Error> {
        let mut json = serde_json::to_value(load_spread_points(300)).unwrap();

        change(&mut json);

        serde_json::from_value(json)
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_serialize_writes_the_version_first() {
        let json = serde_json::to_string(&load_spread_points(10)).unwrap();

        // A reader of another version finds it before the fields it cannot read
        assert!(json.starts_with(r#"{"version":1,"#));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_deserialize_rejects_another_version() {
        let error = read_changed(|json| json["version"] = 2.into()).unwrap_err();

        assert!(error
            .to_string()
            .starts_with("The supercluster has format version 2, and only version 1 can be read."));

        // A supercluster written before the format had a version has none
        let error = read_changed(|json| {
            json.as_object_mut().unwrap().remove("version");
        })
        .unwrap_err();

        assert!(error.to_string().contains("missing field `version`"));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_deserialize_rejects_data_that_does_not_hold_together() {
        type Change = fn(&mut serde_json::Value);

        // Tree 0 holds the clusters of zoom level 0, which hold all 300 points, so handle 300 is a
        // cluster
        let changes: [(Change, &str); 14] = [
            (
                |json| json["indexes"].as_array_mut().unwrap().truncate(10),
                "the indexes do not match the points",
            ),
            (
                |json| json["indexes"][0] = 300.into(),
                "the indexes do not match the points",
            ),
            (
                |json| json["positions"] = serde_json::json!(vec![[0.5, 0.5]; 301]),
                "there are more positions than points",
            ),
            (
                |json| {
                    json["trees"].as_array_mut().unwrap().pop();
                },
                "the number of KD-trees does not match the maximum zoom level",
            ),
            (
                |json| json["point_tree"]["ids"][0] = 1_000_000.into(),
                "a KD-tree is not valid or refers to an item that does not exist",
            ),
            (
                |json| {
                    json["point_tree"]["coords"].as_array_mut().unwrap().pop();
                },
                "a KD-tree is not valid or refers to an item that does not exist",
            ),
            (
                |json| json["point_tree"]["ids"][0] = 300.into(),
                "a KD-tree is not valid or refers to an item that does not exist",
            ),
            (
                |json| json["trees"][0]["ids"][0] = 0.into(),
                "a KD-tree is not valid or refers to an item that does not exist",
            ),
            (
                |json| {
                    json["point_tree"]["ids"].as_array_mut().unwrap().pop();
                    json["point_tree"]["coords"].as_array_mut().unwrap().pop();
                },
                "the KD-tree of the points does not hold every point",
            ),
            (
                |json| {
                    json["clusters"]["child_starts"]
                        .as_array_mut()
                        .unwrap()
                        .pop();
                },
                "the table of clusters is not valid",
            ),
            (
                |json| json["clusters"]["zooms"][0] = 200.into(),
                "a cluster has a zoom level or points that do not exist",
            ),
            (
                |json| json["clusters"]["leaf_starts"][0] = 299.into(),
                "a cluster has a zoom level or points that do not exist",
            ),
            // The first cluster formed of points, so its first child becomes its second point
            (
                |json| json["clusters"]["children"][0] = json["clusters"]["children"][1].clone(),
                "the points of a cluster are not the points of its children",
            ),
            // A cluster that is its own child would make finding where it expands loop
            (
                |json| json["clusters"]["children"][0] = 300.into(),
                "a cluster has a child that does not exist",
            ),
        ];

        for (change, reason) in changes {
            let error = read_changed(change).unwrap_err().to_string();

            assert!(
                error.starts_with(&format!(
                    "The supercluster data does not hold together: {reason}."
                )),
                "{error}"
            );
        }
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
        assert!(
            matches!(&tile.items[0].item(), Item::Cluster(cluster) if cluster.total_count() == 2)
        );
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
            .all(|tile_item| matches!(tile_item.item(), Item::Point { .. })));
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
        assert!(tile.items.iter().any(|tile_item| tile_item.x() < 0));
    }

    #[test]
    fn test_tile_position_of_items() {
        let supercluster = load(16, &[(0.0, 0.0, true)]);
        let tile = supercluster.tile(0, 0, 0);

        assert_eq!(tile.items.len(), 1);
        assert_eq!((tile.items[0].x(), tile.items[0].y()), (256, 256));
    }

    #[test]
    fn test_tile_rounds_positions_up() {
        // With a range from 0 to 1, a coordinate is its normalized position, so a position in the
        // tile of the whole world is the coordinate times the extent of 512, and these are exact
        let range = DataRange::new(0.0, 0.0, 1.0, 1.0);
        let options = SuperclusterOptions::builder()
            .coordinate_system(CoordinateSystem::Cartesian { range })
            .build()
            .unwrap();
        let points = vec![[2.5 / 512.0, 0.5], [-2.5 / 512.0, 0.25]];
        let supercluster = Supercluster::load(options, points, |point| Some(*point)).unwrap();
        let mut positions: Vec<i32> = supercluster
            .tile(0, 0, 0)
            .items
            .iter()
            .map(|tile_item| tile_item.x())
            .collect();

        // Rounding up gives -2 and 3, where rounding away from zero gives -3 and 3
        positions.sort_unstable();
        assert_eq!(positions, [-2, 3]);
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
                Item::Cluster(cluster) => cluster.total_count(),
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
                Item::Cluster(cluster) => Some(cluster.id()),
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
    fn test_filtered_cluster_expansion_zoom_accepting_all() {
        let supercluster = load_nested_cluster();
        let cluster_id = get_top_cluster_id(&supercluster);

        assert_eq!(
            supercluster
                .with_filter(supercluster.filter(|_| true))
                .unwrap()
                .cluster_expansion_zoom(cluster_id),
            Ok(2)
        );
    }

    #[test]
    fn test_filtered_cluster_expansion_zoom_rejecting_a_child() {
        let supercluster = load_nested_cluster();
        let cluster_id = get_top_cluster_id(&supercluster);

        // Without the farther point the cluster looks the same until the close points split on zoom level 17.
        assert_eq!(
            supercluster
                .with_filter(supercluster.filter(|point| point.keep))
                .unwrap()
                .cluster_expansion_zoom(cluster_id),
            Ok(17)
        );
    }

    #[test]
    fn test_filtered_cluster_expansion_zoom_rejecting_all() {
        let supercluster = load_nested_cluster();
        let cluster_id = get_top_cluster_id(&supercluster);

        assert_eq!(
            supercluster
                .with_filter(supercluster.filter(|_| false))
                .unwrap()
                .cluster_expansion_zoom(cluster_id),
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
                .with_filter(supercluster.filter(|_| true))
                .unwrap()
                .cluster_expansion_zoom(cluster_id),
            Ok(1)
        );
        assert_eq!(
            supercluster
                .with_cluster_max_zoom(5)
                .with_filter(supercluster.filter(|_| true))
                .unwrap()
                .cluster_expansion_zoom(cluster_id),
            Ok(2)
        );
        assert_eq!(
            supercluster
                .with_cluster_max_zoom(5)
                .with_filter(supercluster.filter(|point| point.keep))
                .unwrap()
                .cluster_expansion_zoom(cluster_id),
            Ok(6)
        );
        assert_eq!(
            supercluster
                .with_cluster_max_zoom(20)
                .with_filter(supercluster.filter(|point| point.keep))
                .unwrap()
                .cluster_expansion_zoom(cluster_id),
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
    fn test_load_with_min_points_above_255() {
        let options = SuperclusterOptions::builder()
            .min_points(300)
            .build()
            .unwrap();
        let load = |count| {
            Supercluster::load(options.clone(), vec![[10.0, 50.0]; count], |point| {
                Some(*point)
            })
            .unwrap()
        };

        // 300 points at one place form a cluster, and 299 stay points
        assert_eq!(load(300).tile(0, 0, 0).items.len(), 1);
        assert_eq!(load(299).tile(0, 0, 0).items.len(), 299);
    }

    #[test]
    fn test_tile_iterates_over_its_items() {
        let supercluster = load_spread_points(500);
        let tile = supercluster.tile(1, 0, 0);
        let borrowed: Vec<_> = (&tile).into_iter().cloned().collect();

        assert!(!tile.is_empty());
        assert_eq!(borrowed, tile.items);
        assert_eq!(tile.clone().into_iter().collect::<Vec<_>>(), tile.items);
    }

    #[test]
    fn test_count_and_debug_of_tile_items() {
        let points = vec![[10.0, 50.0], [10.001, 50.0], [-100.0, -30.0]];
        let supercluster =
            Supercluster::load(SuperclusterOptions::default(), points, |point| Some(*point))
                .unwrap();
        let mut items = supercluster.tile(0, 0, 0).items;

        items.sort_by_key(|item| item.count());

        // A point counts itself, and a cluster its points
        assert!(matches!(items[0].item(), Item::Point { .. }));
        assert_eq!(items[0].count(), 1);
        assert!(matches!(items[1].item(), Item::Cluster(_)));
        assert_eq!(items[1].count(), 2);

        // A cluster prints its count and where it expands
        let debug = format!("{:?}", items[1]);

        assert!(debug.starts_with("TileItem { x:"));
        assert!(debug.contains("count: 2, expansion_zoom: Some("));
        assert!(debug.ends_with(".. }"));
    }

    #[test]
    fn test_tile_items_of_views_that_expand_on_other_zoom_levels_differ() {
        // Two points that split long after zoom level 0
        let points = vec![[10.0, 50.0], [10.5, 50.0]];
        let supercluster =
            Supercluster::load(SuperclusterOptions::default(), points, |point| Some(*point))
                .unwrap();
        let full = supercluster.tile(0, 0, 0);
        let capped = supercluster.with_cluster_max_zoom(0).tile(0, 0, 0);
        let [full_item, capped_item] = [&full.items[0], &capped.items[0]];

        // The same cluster at the same place, which expands on another zoom level
        assert_eq!(
            (full_item.x(), full_item.y(), full_item.item()),
            (capped_item.x(), capped_item.y(), capped_item.item())
        );
        assert_eq!(capped_item.expansion_zoom(), Some(1));
        assert!(full_item.expansion_zoom() > Some(1));
        assert_ne!(full, capped);
    }

    #[test]
    fn test_tile_items_of_views_that_expand_on_the_same_zoom_levels_are_equal() {
        // Two points that split long before zoom level 15
        let points = vec![[10.0, 50.0], [10.5, 50.0]];
        let supercluster =
            Supercluster::load(SuperclusterOptions::default(), points, |point| Some(*point))
                .unwrap();
        let lower = supercluster.with_cluster_max_zoom(15).tile(0, 0, 0);
        let higher = supercluster.with_cluster_max_zoom(16).tile(0, 0, 0);

        assert!(matches!(lower.items[0].item(), Item::Cluster(_)));
        assert!(lower.items[0].expansion_zoom() < Some(15));
        assert_eq!(lower, higher);
    }

    #[test]
    fn test_tile_points_of_views_with_another_highest_zoom_level_are_equal() {
        let points = vec![[10.0, 50.0], [10.0001, 50.0]];
        let supercluster =
            Supercluster::load(SuperclusterOptions::default(), points, |point| Some(*point))
                .unwrap();
        let z2 = tiles_per_axis(17);
        let x = (convert_longitude_to_spherical_mercator(10.0) * z2).floor() as u32;
        let y = (convert_latitude_to_spherical_mercator(50.0) * z2).floor() as u32;

        // Above the highest zoom level of both views, the tile holds the same points, which have
        // no expansion zoom, so the highest zoom level makes no difference
        let full = supercluster.tile(17, x, y);
        let capped = supercluster.with_cluster_max_zoom(5).tile(17, x, y);

        assert!(full
            .items
            .iter()
            .all(|item| matches!(item.item(), Item::Point { .. })));
        assert!(!full.is_empty());
        assert_eq!(full, capped);
    }

    #[test]
    fn test_tile_item_expansion_zoom() {
        let supercluster = load_spread_points(500);

        for view in [supercluster.view(), supercluster.with_cluster_max_zoom(3)] {
            for tile_item in view.tile(0, 0, 0).items {
                let expected = match tile_item.item() {
                    Item::Cluster(cluster) => {
                        Some(view.cluster_expansion_zoom(cluster.id()).unwrap())
                    }
                    Item::Point { .. } => None,
                };

                assert_eq!(tile_item.expansion_zoom(), expected);
            }
        }
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
