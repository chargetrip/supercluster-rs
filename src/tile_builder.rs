//! # Tile builder module
//!
//! Contains the `TileBuilder` trait, which builds the items of a tile while filtering the points
//! inside clusters and mapping every cluster or point to a type of the caller's choice.

use crate::{
    supercluster::tiles_per_axis, Cluster, Item, Supercluster, SuperclusterError, TileItem,
};

/// The number of points below which a filtered cluster is moved to the center of the points that
/// are left.
const RECENTER_LIMIT: usize = 20;

/// Coordinates of a tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileCoordinates {
    /// The X coordinate of the tile, counted from the left.
    pub x: u32,

    /// The Y coordinate of the tile, counted from the top.
    pub y: u32,

    /// The zoom level of the tile.
    pub z: u8,
}

/// A trait for building the items of a tile, with the points inside clusters filtered and every
/// cluster or point mapped to a type `U`.
///
/// A cluster stays in the tile when at least one of its points passes the filter. When filtering
/// leaves a few points, the cluster moves to their center, and when it leaves a single point, the
/// cluster moves onto that point. The coordinates come from the supercluster, which got them when
/// the points were loaded.
pub trait TileBuilder<P, U> {
    /// Get the coordinates of the tile to build.
    ///
    /// # Returns
    ///
    /// The coordinates of the tile.
    fn get_tile_coordinates(&self) -> TileCoordinates;

    /// Get the maximum zoom level to cluster on for this builder.
    /// It can lower the `max_zoom` of the supercluster but not raise it.
    ///
    /// # Returns
    ///
    /// The zoom level, or `None` to keep the `max_zoom` of the supercluster.
    fn get_cluster_max_zoom_override(&self) -> Option<u8> {
        None
    }

    /// Decide whether a point is part of the tile.
    ///
    /// # Arguments
    ///
    /// - `point`: The point to check.
    ///
    /// # Returns
    ///
    /// `true` if the point should be included, otherwise `false`.
    fn filter(&self, point: &P) -> bool;

    /// Map a cluster or a point into the type of the tile.
    ///
    /// # Arguments
    ///
    /// - `item`: The cluster or the point, as the supercluster has it.
    /// - `points`: The points that passed the filter. These are the points of a cluster, or the
    ///   point itself.
    /// - `coordinates`: The X and Y position in the tile, in the units of the tile extent.
    ///
    /// # Returns
    ///
    /// The mapped cluster or point.
    fn map(&self, item: &Item<'_, P>, points: &[&P], coordinates: [f64; 2]) -> U;

    /// Build the items of the tile. Clusters and points without any point left after the filter
    /// are left out.
    ///
    /// # Arguments
    ///
    /// - `supercluster`: The supercluster to build the tile from.
    ///
    /// # Returns
    ///
    /// The mapped items of the tile, which are none when the tile is empty.
    fn build(&self, supercluster: &Supercluster<P>) -> Result<Vec<U>, SuperclusterError> {
        let tile_coordinates = self.get_tile_coordinates();
        let TileCoordinates { x, y, z } = tile_coordinates;
        let tile = supercluster.get_tile_with_cluster_max_zoom(
            z,
            x,
            y,
            self.get_cluster_max_zoom_override(),
        );

        tile.items
            .iter()
            .filter_map(|tile_item| {
                reduce_item(self, supercluster, tile_coordinates, tile_item).transpose()
            })
            .collect()
    }
}

/// Filter the points of a tile item and map what is left.
///
/// # Arguments
///
/// - `builder`: The builder with the filter and the map.
/// - `supercluster`: The supercluster the tile is built from.
/// - `tile_coordinates`: The coordinates of the tile, as `build` already read them from `builder`.
/// - `tile_item`: The item of the tile, a cluster or a point.
///
/// # Returns
///
/// The mapped item, or `None` if no point is left after the filter.
fn reduce_item<P, U, B>(
    builder: &B,
    supercluster: &Supercluster<P>,
    tile_coordinates: TileCoordinates,
    tile_item: &TileItem<'_, P>,
) -> Result<Option<U>, SuperclusterError>
where
    B: TileBuilder<P, U> + ?Sized,
{
    let (indexes, points): (Vec<usize>, Vec<&P>) = match &tile_item.item {
        Item::Cluster(cluster) => supercluster
            .iter_leaf_indices(cluster.id)?
            .filter_map(|index| {
                let point = &supercluster.points()[index];

                builder.filter(point).then_some((index, point))
            })
            .unzip(),
        Item::Point { index, point } if builder.filter(point) => (vec![*index], vec![*point]),
        Item::Point { .. } => (vec![], vec![]),
    };

    if indexes.is_empty() {
        return Ok(None);
    }

    let mut coordinates = [tile_item.x, tile_item.y];

    if let Item::Cluster(cluster) = &tile_item.item {
        let center = if indexes.len() == 1 {
            supercluster.get_point_coordinates(indexes[0])
        } else if indexes.len() < cluster.point_count && indexes.len() < RECENTER_LIMIT {
            find_center(supercluster, &indexes)
        } else {
            None
        };

        if let Some(center) = center {
            coordinates =
                get_center_position(supercluster, tile_coordinates, tile_item, cluster, center);
        }
    }

    Ok(Some(builder.map(&tile_item.item, &points, coordinates)))
}

/// Get the position in a tile of the center of the points that are left in a cluster.
/// A tile on the edge of the world also holds the clusters of the opposite edge, which sit a world
/// width away, so the center goes in the same copy of the world as the cluster.
///
/// # Arguments
///
/// - `supercluster`: The supercluster the tile is built from.
/// - `tile`: The coordinates of the tile.
/// - `tile_item`: The item of the tile that holds the cluster.
/// - `cluster`: The cluster of the item.
/// - `center`: The center of the points that are left, in the coordinate system of the
///   supercluster.
///
/// # Returns
///
/// The X and Y position in the tile, in the units of the tile extent.
fn get_center_position<P>(
    supercluster: &Supercluster<P>,
    tile: TileCoordinates,
    tile_item: &TileItem<'_, P>,
    cluster: &Cluster,
    center: [f64; 2],
) -> [f64; 2] {
    let z2 = tiles_per_axis(tile.z);
    let (x, y) = supercluster.normalize(center);
    let (cluster_x, _) = supercluster.normalize([cluster.x, cluster.y]);
    let shown_x = (tile_item.x / supercluster.options().extent + f64::from(tile.x)) / z2;

    supercluster.get_tile_position(
        x + (shown_x - cluster_x).round(),
        y,
        z2,
        f64::from(tile.x),
        f64::from(tile.y),
    )
}

/// Find the center of a list of points.
///
/// # Arguments
///
/// - `supercluster`: The supercluster the points were loaded into.
/// - `indexes`: The indexes of the points to find the center of.
///
/// # Returns
///
/// The average of the coordinates of the points, or `None` if none of them has coordinates.
fn find_center<P>(supercluster: &Supercluster<P>, indexes: &[usize]) -> Option<[f64; 2]> {
    let coordinates: Vec<[f64; 2]> = indexes
        .iter()
        .filter_map(|&index| supercluster.get_point_coordinates(index))
        .collect();

    if coordinates.is_empty() {
        return None;
    }

    let count = coordinates.len() as f64;

    Some([
        coordinates
            .iter()
            .map(|coordinates| coordinates[0])
            .sum::<f64>()
            / count,
        coordinates
            .iter()
            .map(|coordinates| coordinates[1])
            .sum::<f64>()
            / count,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::SuperclusterBuilder;

    /// A point that tells whether a filter should keep it.
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

    /// The ID or index, the number of points and the coordinates a `TestBuilder` maps an item to.
    type Mapped = (usize, usize, [f64; 2]);

    /// A builder that keeps the points a filter keeps.
    struct TestBuilder {
        /// The tile to build.
        tile: TileCoordinates,

        /// The maximum zoom level to cluster on.
        cluster_max_zoom_override: Option<u8>,
    }

    impl TileBuilder<TestPoint, Mapped> for TestBuilder {
        fn get_tile_coordinates(&self) -> TileCoordinates {
            self.tile
        }

        fn get_cluster_max_zoom_override(&self) -> Option<u8> {
            self.cluster_max_zoom_override
        }

        fn filter(&self, point: &TestPoint) -> bool {
            point.keep
        }

        fn map(
            &self,
            item: &Item<'_, TestPoint>,
            points: &[&TestPoint],
            coordinates: [f64; 2],
        ) -> Mapped {
            let id = match item {
                Item::Cluster(cluster) => cluster.id,
                Item::Point { index, .. } => *index,
            };

            (id, points.len(), coordinates)
        }
    }

    /// A builder that leaves the maximum zoom level to cluster on to the default.
    struct DefaultBuilder;

    impl TileBuilder<TestPoint, ()> for DefaultBuilder {
        fn get_tile_coordinates(&self) -> TileCoordinates {
            WORLD_TILE
        }

        fn filter(&self, _: &TestPoint) -> bool {
            true
        }

        fn map(&self, _: &Item<'_, TestPoint>, _: &[&TestPoint], _: [f64; 2]) {}
    }

    /// The only tile at zoom level 0.
    const WORLD_TILE: TileCoordinates = TileCoordinates { x: 0, y: 0, z: 0 };

    /// Build a tile of a supercluster with the points loaded.
    /// A point is a longitude, a latitude and whether the filter keeps it.
    fn build(
        points: &[(f64, f64, bool)],
        tile: TileCoordinates,
        cluster_max_zoom_override: Option<u8>,
    ) -> Vec<Mapped> {
        let mut supercluster =
            Supercluster::<TestPoint>::new(SuperclusterBuilder::new().build().unwrap());

        supercluster
            .load_with(
                points
                    .iter()
                    .map(|&(lng, lat, keep)| TestPoint::new(lng, lat, keep))
                    .collect(),
                |point| point.coordinates,
            )
            .unwrap();

        let builder = TestBuilder {
            tile,
            cluster_max_zoom_override,
        };

        builder.build(&supercluster).unwrap()
    }

    #[test]
    fn test_build() {
        let points = [
            (10.0, 50.0, true),
            (12.0, 50.0, true),
            (-100.0, -30.0, true),
        ];
        let mut tile = build(&points, WORLD_TILE, None);

        tile.sort_by_key(|(_, count, _)| *count);

        assert_eq!(tile.len(), 2);

        // The point that is not clustered is mapped with its index.
        assert_eq!((tile[0].0, tile[0].1), (2, 1));

        // A cluster without filtered points stays where the supercluster put it.
        assert_eq!(tile[1].1, 2);
        assert_eq!(tile[1].2[0], 272.0);
    }

    #[test]
    fn test_build_with_one_point_left() {
        let points = [(10.0, 50.0, true), (12.0, 50.0, false)];
        let tile = build(&points, WORLD_TILE, None);

        assert_eq!(tile.len(), 1);

        // The cluster moves onto the point that is left.
        assert_eq!(tile[0].1, 1);
        assert_eq!(tile[0].2[0], 270.0);
    }

    #[test]
    fn test_build_with_few_points_left() {
        let points = [(10.0, 50.0, true), (12.0, 50.0, true), (14.0, 50.0, false)];
        let tile = build(&points, WORLD_TILE, None);

        assert_eq!(tile.len(), 1);

        // The cluster moves to the center of the points that are left, not of all three.
        assert_eq!(tile[0].1, 2);
        assert_eq!(tile[0].2[0], 272.0);
    }

    #[test]
    fn test_build_with_one_point_left_at_the_west_edge() {
        // Tile 0 only holds these points as a copy that sits left of the tile
        let points = [(179.0, 10.0, true), (179.5, 10.0, false)];
        let tile = TileCoordinates { x: 0, y: 0, z: 1 };
        let filtered = build(&points, tile, None);
        let unfiltered = build(&[(179.0, 10.0, true), (179.5, 10.0, true)], tile, None);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].1, 1);

        // The cluster moves onto the point and stays left of the tile
        assert_eq!(filtered[0].2[0], -3.0);
        assert_eq!(filtered[0].2[1], unfiltered[0].2[1]);
    }

    #[test]
    fn test_build_with_one_point_left_at_the_east_edge() {
        // The last tile only holds these points as a copy that sits right of the tile
        let points = [(-179.0, 10.0, true), (-179.5, 10.0, false)];
        let tile = TileCoordinates { x: 1, y: 0, z: 1 };
        let filtered = build(&points, tile, None);
        let unfiltered = build(&[(-179.0, 10.0, true), (-179.5, 10.0, true)], tile, None);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].1, 1);

        // The cluster moves onto the point and stays right of the tile
        assert_eq!(filtered[0].2[0], 515.0);
        assert_eq!(filtered[0].2[1], unfiltered[0].2[1]);
    }

    #[test]
    fn test_build_without_points_left() {
        let points = [(10.0, 50.0, false), (12.0, 50.0, false)];

        assert!(build(&points, WORLD_TILE, None).is_empty());
    }

    #[test]
    fn test_build_with_cluster_max_zoom_override() {
        let points = [(10.0, 50.0, true), (12.0, 50.0, true)];
        let tile = TileCoordinates { x: 1, y: 0, z: 1 };

        let clustered = build(&points, tile, None);

        assert_eq!(clustered.len(), 1);
        assert_eq!(clustered[0].1, 2);

        let unclustered = build(&points, tile, Some(0));

        assert_eq!(unclustered.len(), 2);
        assert!(unclustered.iter().all(|(_, count, _)| *count == 1));
    }

    #[test]
    fn test_build_for_empty_tile() {
        let points = [(10.0, 50.0, true), (12.0, 50.0, true)];
        let tile = TileCoordinates { x: 0, y: 1, z: 1 };

        assert!(build(&points, tile, None).is_empty());
    }

    #[test]
    fn test_build_without_loaded_points() {
        let supercluster = Supercluster::new(SuperclusterBuilder::new().build().unwrap());
        let builder = TestBuilder {
            tile: WORLD_TILE,
            cluster_max_zoom_override: None,
        };

        assert_eq!(builder.build(&supercluster), Ok(vec![]));
    }

    #[test]
    fn test_get_cluster_max_zoom_override_default() {
        assert_eq!(DefaultBuilder.get_cluster_max_zoom_override(), None);
    }

    #[test]
    fn test_find_center_without_coordinates() {
        let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build().unwrap());

        supercluster
            .load_with(
                vec![
                    TestPoint::new(10.0, 50.0, true),
                    TestPoint::without_coordinates(),
                ],
                |point| point.coordinates,
            )
            .unwrap();

        assert_eq!(find_center(&supercluster, &[1]), None);
        assert_eq!(find_center(&supercluster, &[]), None);

        // A point without coordinates is not part of the center
        let [lng, lat] = find_center(&supercluster, &[0, 1]).unwrap();

        assert!((lng - 10.0).abs() < 1e-9);
        assert!((lat - 50.0).abs() < 1e-9);
    }
}
