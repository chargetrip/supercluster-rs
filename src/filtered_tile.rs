//! # Filtered tile module
//!
//! Contains the query that builds a tile from the points that pass a filter. A cluster stays in
//! the tile when at least one of its points passes. When a few points pass, the cluster moves to
//! their center, and when a single point passes, the cluster moves onto that point. The
//! coordinates come from the supercluster, which got them when the points were loaded.

use std::{
    fmt::{self, Debug, Formatter},
    ops::Range,
};

use crate::{
    supercluster::tiles_per_axis, Cluster, Item, Supercluster, SuperclusterView, TileItem,
};

/// The number of points below which a filtered cluster is moved to the center of the points that
/// are left.
const RECENTER_LIMIT: usize = 20;

/// An item of a filtered tile: a cluster or a point with at least one point that passes the filter.
#[non_exhaustive]
pub struct FilteredTileItem<'a, P> {
    /// The X position in the tile, in the units of the tile extent. A cluster with a few points
    /// left is at their center, and a cluster with one point left is on that point.
    pub x: f64,

    /// The Y position in the tile, in the units of the tile extent.
    pub y: f64,

    /// The position in the coordinate system of the supercluster, the longitude and latitude for
    /// `LatLng`. It is the center of the points left for a cluster that moved, the center of the
    /// cluster for one that did not, and the position of a point. Unlike `x` and `y`, it is not
    /// rounded, and a copy of an item across the antimeridian keeps its own coordinates.
    pub coordinates: [f64; 2],

    /// The cluster or the point, as the supercluster has it. The point count of a cluster counts
    /// all of its points, and `count` the ones that pass the filter.
    pub item: Item<'a, P>,

    /// The supercluster the item comes from.
    supercluster: &'a Supercluster<P>,

    /// The highest zoom level to cluster on, from the view that built the tile.
    cluster_max_zoom: u8,

    /// One bit per point of the cluster, in the order of its leaves, set when the point passes the
    /// filter. It is empty when every point passes, which takes no memory, and for a point.
    matches: Vec<u64>,
}

impl<'a, P> FilteredTileItem<'a, P> {
    /// Get the number of points that pass the filter: of the cluster, or 1 for a point.
    pub fn count(&self) -> usize {
        match self.item {
            Item::Cluster(cluster) if self.matches.is_empty() => cluster.point_count,
            Item::Cluster(_) => self
                .matches
                .iter()
                .map(|word| word.count_ones() as usize)
                .sum(),
            Item::Point { .. } => 1,
        }
    }

    /// Iterate over the points that pass the filter: the points of the cluster, in the order
    /// `Supercluster::leaves` returns them, or the point itself. The first point is
    /// `points().next()`.
    pub fn points(&self) -> impl Iterator<Item = &'a P> + '_ {
        let (point, leaves): (Option<&'a P>, &'a [u32]) = match self.item {
            Item::Cluster(cluster) => (None, self.supercluster.cluster_leaves(cluster.id)),
            Item::Point { point, .. } => (Some(point), &[]),
        };
        let points = self.supercluster.points();

        point
            .into_iter()
            .chain(passing(leaves, &self.matches).map(move |index| &points[index]))
    }

    /// Determine the zoom level at which the cluster expands, counting only the points that pass
    /// the filter, like `cluster_expansion_zoom_filtered` with the same filter on the same view.
    /// It reuses what the filter returned while the tile was built, so it does not call the filter
    /// again.
    ///
    /// Returns `None` for a point.
    pub fn expansion_zoom(&self) -> Option<u8> {
        let Item::Cluster(cluster) = self.item else {
            return None;
        };

        // The cluster comes from the supercluster, so finding its children does not fail
        self.supercluster
            .expansion_zoom_where(cluster.id, self.cluster_max_zoom, |_, _, positions| {
                any_set(&self.matches, positions)
            })
            .ok()
    }
}

// Written by hand, because a derive would also require the point type to be `Clone`, and an item
// only holds references to points.
impl<P> Clone for FilteredTileItem<'_, P> {
    fn clone(&self) -> Self {
        FilteredTileItem {
            x: self.x,
            y: self.y,
            coordinates: self.coordinates,
            item: self.item,
            supercluster: self.supercluster,
            cluster_max_zoom: self.cluster_max_zoom,
            matches: self.matches.clone(),
        }
    }
}

// Written by hand, because a derive would print the whole supercluster.
impl<P: Debug> Debug for FilteredTileItem<'_, P> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FilteredTileItem")
            .field("x", &self.x)
            .field("y", &self.y)
            .field("coordinates", &self.coordinates)
            .field("item", &self.item)
            .field("count", &self.count())
            .finish_non_exhaustive()
    }
}

impl<P> Supercluster<P> {
    /// Retrieve the items of a tile, like `tile`, but keep only the points for which `filter`
    /// returns `true`. A cluster without any point left is left out, and a cluster with a few points
    /// left moves to their center. Map the items to the type of your response with an iterator.
    ///
    /// It calls `filter` once for every point of every item of the tile, and the items reuse the
    /// results, for example in `FilteredTileItem::expansion_zoom`. A cluster that the tile also
    /// shows as a copy across the antimeridian is filtered once for each copy.
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
    /// // The cluster holds the available point only, and moves onto it
    /// assert_eq!(items.len(), 1);
    /// assert_eq!(items[0].count(), 1);
    ///
    /// let [lng, lat] = items[0].coordinates;
    ///
    /// assert!((lng - 10.0).abs() < 1e-9 && (lat - 50.0).abs() < 1e-9);
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
        self.tile(z, x, y)
            .items
            .iter()
            .filter_map(|tile_item| filter_item(self, (z, x, y), tile_item, &filter))
            .collect()
    }
}

/// Filter the points of a tile item and place what is left, or return `None` if no point is left.
/// `tile` is the zoom level, X and Y coordinates of the tile.
fn filter_item<'a, P, F>(
    view: &SuperclusterView<'a, P>,
    tile: (u8, u32, u32),
    tile_item: &TileItem<'a, P>,
    filter: &F,
) -> Option<FilteredTileItem<'a, P>>
where
    F: Fn(&P) -> bool,
{
    let supercluster = view.supercluster;

    let (matches, [x, y], coordinates) = match tile_item.item {
        Item::Point { index, point } => {
            if !filter(point) {
                return None;
            }

            let coordinates = supercluster.get_point_coordinates(index)?;

            (vec![], [tile_item.x, tile_item.y], coordinates)
        }
        Item::Cluster(cluster) => {
            let leaves = supercluster.cluster_leaves(cluster.id);
            let points = supercluster.points();
            let mut matches = vec![];
            let mut count = 0;

            for (position, &index) in leaves.iter().enumerate() {
                if filter(&points[index as usize]) {
                    count += 1;

                    if !matches.is_empty() {
                        matches[position / 64] |= 1 << (position % 64);
                    }
                } else if matches.is_empty() {
                    // The first point that fails: every point before it passed
                    matches = vec![0; leaves.len().div_ceil(64)];
                    matches[..position / 64].fill(u64::MAX);
                    matches[position / 64] = (1 << (position % 64)) - 1;
                }
            }

            if count == 0 {
                return None;
            }

            let center = if count < cluster.point_count && count < RECENTER_LIMIT {
                find_center(supercluster, passing(leaves, &matches))
            } else {
                None
            };

            match center {
                Some(center) => {
                    let position =
                        get_center_position(supercluster, tile, tile_item, &cluster, center);

                    (matches, position, center)
                }
                None => (matches, [tile_item.x, tile_item.y], [cluster.x, cluster.y]),
            }
        }
    };

    Some(FilteredTileItem {
        x,
        y,
        coordinates,
        item: tile_item.item,
        supercluster,
        cluster_max_zoom: view.cluster_max_zoom,
        matches,
    })
}

/// Iterate over the indexes of the points of a cluster whose bit is set in `bits`, or of every
/// point if `bits` is empty, in the order of its leaves.
fn passing<'b>(leaves: &'b [u32], bits: &'b [u64]) -> impl Iterator<Item = usize> + 'b {
    leaves
        .iter()
        .enumerate()
        .filter(|&(position, _)| {
            bits.is_empty() || (bits[position / 64] >> (position % 64)) & 1 == 1
        })
        .map(|(_, &index)| index as usize)
}

/// Check whether a bit at one of `positions` is set, which is always the case for a non-empty range
/// if `bits` is empty. It checks a word at a time, because a selective filter leaves long runs of
/// points that fail in the large clusters of low zoom levels.
fn any_set(bits: &[u64], Range { mut start, end }: Range<usize>) -> bool {
    if bits.is_empty() {
        return start < end;
    }

    while start < end {
        // The bits from `start` to the end of its word, or to `end`
        let width = (64 - start % 64).min(end - start);

        if (bits[start / 64] >> (start % 64)) & (u64::MAX >> (64 - width)) != 0 {
            return true;
        }

        start += width;
    }

    false
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
fn find_center<P>(
    supercluster: &Supercluster<P>,
    indexes: impl Iterator<Item = usize>,
) -> Option<[f64; 2]> {
    let mut count = 0;
    let mut sum = [0.0, 0.0];

    for [x, y] in indexes.filter_map(|index| supercluster.get_point_coordinates(index)) {
        count += 1;
        sum[0] += x;
        sum[1] += y;
    }

    (count > 0).then(|| [sum[0] / count as f64, sum[1] / count as f64])
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::Cell;

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

                (id, tile_item.count(), [tile_item.x, tile_item.y])
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

    /// Create points spread over the world with a fixed formula, so every run uses the same points.
    /// The filter keeps every third point.
    fn create_spread_points(count: usize) -> Vec<(f64, f64, bool)> {
        (0..count)
            .map(|i| {
                let lng = (i as f64 * 0.618_033_988_749_895) % 1.0;
                let lat = (i as f64 * 0.754_877_666_246_693) % 1.0;

                (-180.0 + 360.0 * lng, -80.0 + 160.0 * lat, i % 3 == 0)
            })
            .collect()
    }

    #[test]
    fn test_tile_filtered_calls_the_filter_once_per_point() {
        // Far from the antimeridian, so the tile shows every item once
        let supercluster = load(&[
            (10.0, 50.0, true),
            (10.001, 50.0, false),
            (10.002, 50.0, true),
            (-100.0, -30.0, true),
        ]);
        let calls = Cell::new(0);
        let items = supercluster.tile_filtered(0, 0, 0, |point| {
            calls.set(calls.get() + 1);
            point.keep
        });

        assert_eq!(items.len(), 2);
        assert_eq!(calls.get(), 4);

        // The expansion zoom reuses what the filter returned
        assert!(items.iter().any(|item| item.expansion_zoom().is_some()));
        assert_eq!(calls.get(), 4);
    }

    #[test]
    fn test_expansion_zoom_matches_cluster_expansion_zoom_filtered() {
        let supercluster = load(&create_spread_points(500));
        let filters: [fn(&TestPoint) -> bool; 2] = [|point| point.keep, |_| true];
        let mut clusters = 0;

        for view in [supercluster.view(), supercluster.with_cluster_max_zoom(3)] {
            for filter in filters {
                for z in 0..=5 {
                    for x in 0..1 << z {
                        for y in 0..1 << z {
                            for item in view.tile_filtered(z, x, y, filter) {
                                let expected = match item.item {
                                    Item::Cluster(cluster) => Some(
                                        view.cluster_expansion_zoom_filtered(cluster.id, filter)
                                            .unwrap(),
                                    ),
                                    Item::Point { .. } => None,
                                };

                                clusters += usize::from(expected.is_some());

                                assert_eq!(item.expansion_zoom(), expected);
                            }
                        }
                    }
                }
            }
        }

        assert!(clusters > 0);
    }

    #[test]
    fn test_points_of_a_filtered_cluster() {
        let supercluster = load(&[
            (10.0, 50.0, true),
            (10.001, 50.0, false),
            (10.002, 50.0, true),
        ]);
        let Item::Cluster(cluster) = supercluster.tile(0, 0, 0).items[0].item else {
            panic!("expected a cluster");
        };
        let leaves: Vec<&TestPoint> = supercluster.leaves(cluster.id).unwrap().collect();
        let coordinates_of = |points: Vec<&TestPoint>| -> Vec<Option<[f64; 2]>> {
            points.iter().map(|point| point.coordinates).collect()
        };

        // The points that pass, in the order of the leaves
        let filtered = supercluster.tile_filtered(0, 0, 0, |point| point.keep);
        let kept = leaves.iter().copied().filter(|point| point.keep).collect();

        assert_eq!(filtered[0].count(), 2);
        assert_eq!(
            coordinates_of(filtered[0].points().collect()),
            coordinates_of(kept)
        );

        // When every point passes, the item holds all of them
        let unfiltered = supercluster.tile_filtered(0, 0, 0, |_| true);

        assert_eq!(unfiltered[0].count(), 3);
        assert_eq!(
            coordinates_of(unfiltered[0].points().collect()),
            coordinates_of(leaves)
        );
    }

    #[test]
    fn test_coordinates_of_filtered_items() {
        let supercluster = load(&[
            (10.0, 50.0, true),
            (12.0, 50.0, true),
            (14.0, 50.0, false),
            (-100.0, -30.0, true),
        ]);
        let items = supercluster.tile_filtered(0, 0, 0, |point| point.keep);
        let close =
            |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;

        for item in &items {
            match item.item {
                // The cluster moved to the center of the two points left, without rounding
                Item::Cluster(_) => assert!(close(item.coordinates, [11.0, 50.0])),
                Item::Point { .. } => assert!(close(item.coordinates, [-100.0, -30.0])),
            }
        }

        // A cluster that keeps all of its points keeps its own center
        let unfiltered = supercluster.tile_filtered(0, 0, 0, |_| true);
        let Item::Cluster(cluster) = unfiltered[0].item else {
            panic!("expected a cluster");
        };

        assert_eq!(unfiltered[0].coordinates, [cluster.x, cluster.y]);
    }

    #[test]
    fn test_expansion_zoom_of_a_point() {
        let supercluster = load(&[(10.0, 50.0, true), (-100.0, -30.0, true)]);
        let items = supercluster.tile_filtered(0, 0, 0, |point| point.keep);

        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item.expansion_zoom().is_none()));
        assert!(items.iter().all(|item| item.points().count() == 1));
    }

    #[test]
    fn test_debug_of_a_filtered_tile_item() {
        let supercluster = Supercluster::load(
            SuperclusterOptions::default(),
            vec![[10.0, 50.0], [10.001, 50.0], [-100.0, -30.0]],
            |point| Some(*point),
        )
        .unwrap();
        let items = supercluster.tile_filtered(0, 0, 0, |point| point[0] > 0.0);

        // The output shows the item, not the whole supercluster it comes from
        let debug = format!("{:?}", items[0]);

        assert!(debug.starts_with("FilteredTileItem { x:"));
        assert!(debug.contains("count: 2"));
        assert!(!debug.contains("trees"));
    }

    #[test]
    fn test_tile_filtered_with_a_large_cluster() {
        // 130 close points form one cluster, and only its 101st leaf fails the filter
        let points: Vec<[f64; 2]> = (0..130).map(|i| [10.0 + i as f64 * 1e-6, 50.0]).collect();
        let supercluster =
            Supercluster::load(SuperclusterOptions::default(), points, |point| Some(*point))
                .unwrap();
        let Item::Cluster(cluster) = supercluster.tile(0, 0, 0).items[0].item else {
            panic!("expected a cluster");
        };
        let failing = *supercluster.leaves(cluster.id).unwrap().nth(100).unwrap();
        let filter = |point: &[f64; 2]| *point != failing;

        let items = supercluster.tile_filtered(0, 0, 0, filter);

        assert_eq!(items[0].count(), 129);
        assert!(items[0].points().all(|point| *point != failing));
        assert_eq!(
            items[0].expansion_zoom(),
            supercluster
                .cluster_expansion_zoom_filtered(cluster.id, filter)
                .ok()
        );
    }

    #[test]
    fn test_any_set_across_words() {
        // 130 bits, which spread over three words, with only bit 64 set
        let bits = [0, 1, 0];

        assert!(any_set(&bits, 0..65));
        assert!(any_set(&bits, 64..65));
        assert!(any_set(&bits, 60..130));
        assert!(!any_set(&bits, 0..64));
        assert!(!any_set(&bits, 65..130));
        assert!(!any_set(&bits, 64..64));
        assert!(any_set(&[u64::MAX], 0..64));

        // No bits means every point passes
        assert!(any_set(&[], 3..5));
        assert!(!any_set(&[], 5..5));
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
        assert_eq!(clustered[0].count(), 2);

        let unclustered = supercluster
            .with_cluster_max_zoom(0)
            .tile_filtered(1, 1, 0, |point| point.keep);

        assert_eq!(unclustered.len(), 2);
        assert!(unclustered.iter().all(|tile_item| tile_item.count() == 1));
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

        assert_eq!(find_center(&supercluster, [1].into_iter()), None);
        assert_eq!(find_center(&supercluster, std::iter::empty()), None);

        // A point without coordinates is not part of the center
        let [lng, lat] = find_center(&supercluster, [0, 1].into_iter()).unwrap();

        assert!((lng - 10.0).abs() < 1e-9);
        assert!((lat - 50.0).abs() < 1e-9);
    }
}
