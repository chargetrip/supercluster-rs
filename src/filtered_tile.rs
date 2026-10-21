//! # Filtered tile module
//!
//! Contains the query that builds a tile from the points that pass a filter. A cluster stays in
//! the tile when at least one of its points passes. When a few points pass, the cluster moves to
//! their center, and when a single point passes, the cluster moves onto that point. The
//! coordinates come from the supercluster, which got them when the points were loaded.

use crate::{
    supercluster::tiles_per_axis, Cluster, Item, Supercluster, SuperclusterView, TileItem,
};

/// The number of points below which a filtered cluster is moved to the center of the points that
/// are left.
const RECENTER_LIMIT: usize = 20;

/// An item of a filtered tile: a cluster or a point with at least one point that passes the filter.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub struct FilteredTileItem<'a, P> {
    /// The X position in the tile, in the units of the tile extent. A cluster with a few points
    /// left is at their center, and a cluster with one point left is on that point.
    pub x: f64,

    /// The Y position in the tile, in the units of the tile extent.
    pub y: f64,

    /// The cluster or the point, as the supercluster has it. The point count of a cluster counts
    /// all of its points, and `points` holds the ones that pass the filter.
    pub item: Item<'a, P>,

    /// The points that pass the filter. These are points of the cluster, or the point itself.
    pub points: Vec<&'a P>,
}

// Written by hand, because a derive would also require the point type to be `Clone`, and an item
// only holds references to points.
impl<P> Clone for FilteredTileItem<'_, P> {
    fn clone(&self) -> Self {
        FilteredTileItem {
            x: self.x,
            y: self.y,
            item: self.item,
            points: self.points.clone(),
        }
    }
}

impl<P> Supercluster<P> {
    /// Retrieve the items of a tile, like `tile`, but keep only the points for which `filter`
    /// returns `true`. A cluster without any point left is left out, and a cluster with a few points
    /// left moves to their center. Map the items to the type of your response with an iterator.
    ///
    /// # Examples
    ///
    /// ```
    /// use supercluster::{Supercluster, SuperclusterOptions};
    ///
    /// // A point is a longitude, a latitude and whether it is available
    /// let points = vec![(10.0, 50.0, true), (10.001, 50.0, false)];
    /// let supercluster = Supercluster::load(SuperclusterOptions::default(), points, |point| {
    ///     Some([point.0, point.1])
    /// })?;
    ///
    /// let items = supercluster.tile_filtered(0, 0, 0, |point| point.2);
    ///
    /// // The cluster holds the available point only
    /// assert_eq!(items.len(), 1);
    /// assert_eq!(items[0].points.len(), 1);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn tile_filtered<F>(&self, z: u8, x: u32, y: u32, filter: F) -> Vec<FilteredTileItem<'_, P>>
    where
        F: Fn(&P) -> bool,
    {
        self.view().tile_filtered(z, x, y, filter)
    }
}

impl<'a, P> SuperclusterView<'a, P> {
    /// Retrieve the items of a tile that have a point for which `filter` returns `true`, like
    /// `Supercluster::tile_filtered`.
    pub fn tile_filtered<F>(&self, z: u8, x: u32, y: u32, filter: F) -> Vec<FilteredTileItem<'a, P>>
    where
        F: Fn(&P) -> bool,
    {
        let supercluster = self.supercluster;

        self.tile(z, x, y)
            .items
            .iter()
            .filter_map(|tile_item| filter_item(supercluster, (z, x, y), tile_item, &filter))
            .collect()
    }
}

/// Filter the points of a tile item and place what is left, or return `None` if no point is left.
/// `tile` is the zoom level, X and Y coordinates of the tile.
fn filter_item<'a, P, F>(
    supercluster: &'a Supercluster<P>,
    tile: (u8, u32, u32),
    tile_item: &TileItem<'a, P>,
    filter: &F,
) -> Option<FilteredTileItem<'a, P>>
where
    F: Fn(&P) -> bool,
{
    let (indexes, points): (Vec<usize>, Vec<&'a P>) = match &tile_item.item {
        Item::Cluster(cluster) => supercluster
            .cluster_leaf_indices(cluster.id)
            .filter_map(|index| {
                let point = &supercluster.points()[index];

                filter(point).then_some((index, point))
            })
            .unzip(),
        Item::Point { index, point } if filter(point) => (vec![*index], vec![*point]),
        Item::Point { .. } => return None,
    };

    if indexes.is_empty() {
        return None;
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
            coordinates = get_center_position(supercluster, tile, tile_item, cluster, center);
        }
    }

    Some(FilteredTileItem {
        x: coordinates[0],
        y: coordinates[1],
        item: tile_item.item,
        points,
    })
}

/// Get the position in a tile of the center of the points that are left in a cluster.
/// A tile on the edge of the world also holds the clusters of the opposite edge, which sit a world
/// width away, so the center goes in the same copy of the world as the cluster. `center` is in the
/// coordinate system of the supercluster, and the position is in the units of the tile extent.
fn get_center_position<P>(
    supercluster: &Supercluster<P>,
    (z, tile_x, tile_y): (u8, u32, u32),
    tile_item: &TileItem<'_, P>,
    cluster: &Cluster,
    center: [f64; 2],
) -> [f64; 2] {
    let z2 = tiles_per_axis(z);
    let (x, y) = supercluster.normalize(center);
    let (cluster_x, _) = supercluster.normalize([cluster.x, cluster.y]);
    let shown_x = (tile_item.x / supercluster.options().extent + f64::from(tile_x)) / z2;

    supercluster.get_tile_position(
        x + (shown_x - cluster_x).round(),
        y,
        z2,
        f64::from(tile_x),
        f64::from(tile_y),
    )
}

/// Find the average of the coordinates of the points at `indexes`, or `None` if none of them has
/// coordinates.
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

    use crate::SuperclusterOptions;

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

    /// The ID or index, the number of points left and the position in the tile of an item.
    type Summary = (usize, usize, [f64; 2]);

    /// The only tile at zoom level 0.
    const WORLD_TILE: (u8, u32, u32) = (0, 0, 0);

    /// Load a supercluster with the default options.
    /// A point is a longitude, a latitude and whether the filter keeps it.
    fn load(points: &[(f64, f64, bool)]) -> Supercluster<TestPoint> {
        Supercluster::load(
            SuperclusterOptions::default(),
            points
                .iter()
                .map(|&(lng, lat, keep)| TestPoint::new(lng, lat, keep))
                .collect(),
            |point| point.coordinates,
        )
        .unwrap()
    }

    /// Summarize the items of a filtered tile.
    fn summarize(items: &[FilteredTileItem<'_, TestPoint>]) -> Vec<Summary> {
        items
            .iter()
            .map(|tile_item| {
                let id = match tile_item.item {
                    Item::Cluster(cluster) => cluster.id.get(),
                    Item::Point { index, .. } => index,
                };

                (id, tile_item.points.len(), [tile_item.x, tile_item.y])
            })
            .collect()
    }

    /// Load the points and get the filtered tile, which keeps the points the filter keeps.
    fn tile_filtered(points: &[(f64, f64, bool)], (z, x, y): (u8, u32, u32)) -> Vec<Summary> {
        let supercluster = load(points);

        summarize(&supercluster.tile_filtered(z, x, y, |point| point.keep))
    }

    #[test]
    fn test_tile_filtered() {
        let points = [
            (10.0, 50.0, true),
            (12.0, 50.0, true),
            (-100.0, -30.0, true),
        ];
        let mut tile = tile_filtered(&points, WORLD_TILE);

        tile.sort_by_key(|(_, count, _)| *count);

        assert_eq!(tile.len(), 2);

        // The point that is not clustered has its index.
        assert_eq!((tile[0].0, tile[0].1), (2, 1));

        // A cluster without filtered points stays where the supercluster put it.
        assert_eq!(tile[1].1, 2);
        assert_eq!(tile[1].2[0], 272.0);
    }

    #[test]
    fn test_tile_filtered_with_one_point_left() {
        let points = [(10.0, 50.0, true), (12.0, 50.0, false)];
        let tile = tile_filtered(&points, WORLD_TILE);

        assert_eq!(tile.len(), 1);

        // The cluster moves onto the point that is left.
        assert_eq!(tile[0].1, 1);
        assert_eq!(tile[0].2[0], 270.0);
    }

    #[test]
    fn test_tile_filtered_with_few_points_left() {
        let points = [(10.0, 50.0, true), (12.0, 50.0, true), (14.0, 50.0, false)];
        let tile = tile_filtered(&points, WORLD_TILE);

        assert_eq!(tile.len(), 1);

        // The cluster moves to the center of the points that are left, not of all three.
        assert_eq!(tile[0].1, 2);
        assert_eq!(tile[0].2[0], 272.0);
    }

    #[test]
    fn test_tile_filtered_with_one_point_left_at_the_west_edge() {
        // Tile 0 only holds these points as a copy that sits left of the tile
        let tile = (1, 0, 0);
        let filtered = tile_filtered(&[(179.0, 10.0, true), (179.5, 10.0, false)], tile);
        let unfiltered = tile_filtered(&[(179.0, 10.0, true), (179.5, 10.0, true)], tile);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].1, 1);

        // The cluster moves onto the point and stays left of the tile
        assert_eq!(filtered[0].2[0], -3.0);
        assert_eq!(filtered[0].2[1], unfiltered[0].2[1]);
    }

    #[test]
    fn test_tile_filtered_with_one_point_left_at_the_east_edge() {
        // The last tile only holds these points as a copy that sits right of the tile
        let tile = (1, 1, 0);
        let filtered = tile_filtered(&[(-179.0, 10.0, true), (-179.5, 10.0, false)], tile);
        let unfiltered = tile_filtered(&[(-179.0, 10.0, true), (-179.5, 10.0, true)], tile);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].1, 1);

        // The cluster moves onto the point and stays right of the tile
        assert_eq!(filtered[0].2[0], 515.0);
        assert_eq!(filtered[0].2[1], unfiltered[0].2[1]);
    }

    #[test]
    fn test_tile_filtered_with_a_point_that_is_not_clustered() {
        // The two points are far apart, so the filter rejects one of them on its own
        let points = [(10.0, 50.0, false), (-100.0, -30.0, true)];
        let tile = tile_filtered(&points, WORLD_TILE);

        assert_eq!(tile.len(), 1);

        // The point that is left has its index
        assert_eq!((tile[0].0, tile[0].1), (1, 1));
    }

    #[test]
    fn test_clone_of_a_filtered_tile_item_without_a_clone_point() {
        let supercluster = load(&[(10.0, 50.0, true), (12.0, 50.0, true)]);
        let items = supercluster.tile_filtered(0, 0, 0, |point| point.keep);
        let copy = items[0].clone();

        assert_eq!(
            summarize(std::slice::from_ref(&copy)),
            summarize(&items[..1])
        );
    }

    #[test]
    fn test_tile_filtered_without_points_left() {
        let points = [(10.0, 50.0, false), (12.0, 50.0, false)];

        assert!(tile_filtered(&points, WORLD_TILE).is_empty());
    }

    #[test]
    fn test_tile_filtered_with_cluster_max_zoom() {
        let supercluster = load(&[(10.0, 50.0, true), (12.0, 50.0, true)]);

        let clustered = supercluster.tile_filtered(1, 1, 0, |point| point.keep);

        assert_eq!(clustered.len(), 1);
        assert_eq!(clustered[0].points.len(), 2);

        let unclustered = supercluster
            .with_cluster_max_zoom(0)
            .tile_filtered(1, 1, 0, |point| point.keep);

        assert_eq!(unclustered.len(), 2);
        assert!(unclustered
            .iter()
            .all(|tile_item| tile_item.points.len() == 1));
    }

    #[test]
    fn test_tile_filtered_for_empty_tile() {
        let points = [(10.0, 50.0, true), (12.0, 50.0, true)];

        assert!(tile_filtered(&points, (1, 0, 1)).is_empty());
    }

    #[test]
    fn test_tile_filtered_without_loaded_points() {
        assert!(tile_filtered(&[], WORLD_TILE).is_empty());
    }

    #[test]
    fn test_find_center_without_coordinates() {
        let supercluster = Supercluster::load(
            SuperclusterOptions::default(),
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
