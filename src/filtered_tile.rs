//! # Filtered tile module
//!
//! Contains the queries that build a tile from the points that pass a filter. A cluster stays in
//! the tile when at least one of its points passes. When some of its points fail, the cluster
//! moves to the exact center of the points that pass, and a cluster with a single point left
//! becomes that point. The coordinates come from the supercluster, which got them when the points
//! were loaded.
//!
//! The filter is a closure that runs on the points of the tile, or a `Selection` that ran it on
//! every point once, or a combination of selections that the query evaluates on the points of the
//! tile.

use std::{
    fmt::{self, Debug, Formatter},
    mem,
    ops::Range,
    slice, vec,
};

use crate::{
    selection::{filter_bits, Matches, Scratch},
    sums::Positions,
    supercluster::tiles_per_axis,
    ClusterId, Item, Select, Selection, Supercluster, SuperclusterError, SuperclusterView,
};

/// The items of a tile that have a point that passes a filter, which `tile_filtered` and
/// `tile_selected` return.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub struct FilteredTile<'a, P> {
    /// The clusters and points with a point that passes, which also holds the ones of the buffer
    /// around the tile.
    pub items: Vec<FilteredTileItem<'a, P>>,
}

impl<P> FilteredTile<'_, P> {
    /// Check whether the tile has no items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<'a, P> IntoIterator for FilteredTile<'a, P> {
    type Item = FilteredTileItem<'a, P>;
    type IntoIter = vec::IntoIter<FilteredTileItem<'a, P>>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<'t, 'a, P> IntoIterator for &'t FilteredTile<'a, P> {
    type Item = &'t FilteredTileItem<'a, P>;
    type IntoIter = slice::Iter<'t, FilteredTileItem<'a, P>>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

// Written by hand, because a derive would also require the point type to be `Clone`, and the items
// only hold references to points.
impl<P> Clone for FilteredTile<'_, P> {
    fn clone(&self) -> Self {
        FilteredTile {
            items: self.items.clone(),
        }
    }
}

/// An item of a filtered tile: a cluster or a point with at least one point that passes the filter.
#[non_exhaustive]
pub struct FilteredTileItem<'a, P> {
    /// The X position in the tile, in the units of the tile extent. A cluster with points that
    /// fail is at the center of the points left.
    pub x: f64,

    /// The Y position in the tile, in the units of the tile extent.
    pub y: f64,

    /// The cluster or the point, as the supercluster has it. The point count of a cluster counts
    /// all of its points, and `count` the ones that pass the filter.
    pub item: Item<'a, P>,

    /// The number of points that pass the filter.
    count: u32,

    /// The supercluster the item comes from.
    supercluster: &'a Supercluster<P>,

    /// The highest zoom level to cluster on, from the view that built the tile.
    cluster_max_zoom: u8,

    /// The positions of the points of the item in the points of the supercluster.
    leaves: Range<u32>,

    /// Which points of the item pass the filter.
    matches: Matches<'a>,

    /// Where the item is, normalized to the [0..1] range the KD-trees use, which only a call to
    /// `coordinates` converts.
    position: [f64; 2],
}

impl<'a, P> FilteredTileItem<'a, P> {
    /// Get the number of points that pass the filter: of the cluster, or 1 for a point.
    pub fn count(&self) -> u32 {
        self.count
    }

    /// Get the position in the coordinate system of the supercluster, the longitude and latitude
    /// for `LatLng`. It is the center of the points left for a cluster that moved, the center of
    /// the cluster for one that did not, and the position of a point. Unlike `x` and `y`, it is not
    /// rounded, and a copy of an item across the antimeridian keeps its own coordinates. It is
    /// computed on each call, so a query that does not need it does not pay for it.
    pub fn coordinates(&self) -> [f64; 2] {
        self.supercluster
            .options()
            .coordinate_system
            .denormalize(self.position)
    }

    /// Iterate over the points that pass the filter: the points of the cluster, in the order
    /// `Supercluster::leaves` returns them, or the point itself. The first point is
    /// `points().next()`.
    pub fn points(&self) -> impl Iterator<Item = &'a P> + '_ {
        let points = self.supercluster.points();

        self.matches
            .passing(self.leaves())
            .map(move |leaf| &points[leaf])
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
            .expansion_zoom_where(cluster.id, self.cluster_max_zoom, |leaves| {
                self.matches.any(leaves)
            })
            .ok()
    }

    /// Get the positions of the points of the item in the points of the supercluster.
    fn leaves(&self) -> Range<usize> {
        self.leaves.start as usize..self.leaves.end as usize
    }
}

// Written by hand, because a derive would also require the point type to be `Clone`, and an item
// only holds references to points.
impl<P> Clone for FilteredTileItem<'_, P> {
    fn clone(&self) -> Self {
        FilteredTileItem {
            x: self.x,
            y: self.y,
            item: self.item,
            count: self.count,
            supercluster: self.supercluster,
            cluster_max_zoom: self.cluster_max_zoom,
            leaves: self.leaves.clone(),
            matches: self.matches.clone(),
            position: self.position,
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
            .field("coordinates", &self.coordinates())
            .field("item", &self.item)
            .field("count", &self.count)
            .finish_non_exhaustive()
    }
}

// Written by hand, because a derive would compare the whole supercluster. Items are equal at the
// same position with the same cluster or point and the same points that pass, counted first, which
// is cheaper. Clusters also need views with the same highest zoom level, so they expand on the same
// zoom level.
impl<P: PartialEq> PartialEq for FilteredTileItem<'_, P> {
    fn eq(&self, other: &Self) -> bool {
        self.x == other.x
            && self.y == other.y
            && self.item == other.item
            && self.count == other.count
            && (matches!(self.item, Item::Point { .. })
                || self.cluster_max_zoom == other.cluster_max_zoom)
            && self.points().eq(other.points())
    }
}

impl<P> Supercluster<P> {
    /// Run a filter once on every point that was clustered, and keep which ones pass, for
    /// `tile_selected` and `cluster_expansion_zoom_selected` to reuse. The points do not change
    /// after `load`, so run it again only when the filter reads data that changes, outside the
    /// points or shared through them.
    ///
    /// It scans the points in the order they are stored, and does not branch on what the filter
    /// returns, so it runs the filter about as fast as the filter itself allows.
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
    /// // Run the filter once, then reuse it for every tile until the availability changes
    /// let available = supercluster.select(|point| point.2);
    /// let items = supercluster.tile_selected(0, 0, 0, &available).items;
    ///
    /// assert_eq!(items[0].count(), 1);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn select<F>(&self, filter: F) -> Selection
    where
        F: Fn(&P) -> bool,
    {
        let points = &self.points()[..self.clustered_points()];

        Selection::new(
            filter_bits(points, filter).collect(),
            Some(self.positions()),
        )
    }

    /// Retrieve the items of a tile, like `tile`, but keep only the points for which `filter`
    /// returns `true`. A cluster without any point left is left out, and a cluster with points that
    /// fail moves to the exact center of the points left. Map the items to the type of your
    /// response with an iterator.
    ///
    /// It calls `filter` once for every point of every item of the tile, and the items reuse the
    /// results, for example in `FilteredTileItem::expansion_zoom`. A cluster that the tile also
    /// shows as a copy across the antimeridian is filtered once for each copy. To filter many
    /// tiles the same way, `select` once and use `tile_selected`. With a selection, its cost does
    /// not grow with the points of the tile, and with a combination of selections, it runs no
    /// filter.
    ///
    /// # Examples
    ///
    /// ```
    /// use supercluster::{Item, Supercluster, SuperclusterOptions};
    ///
    /// // A point is a longitude, a latitude and whether it is available
    /// let points = vec![(10.0, 50.0, true), (10.001, 50.0, false)];
    /// let supercluster = Supercluster::load(SuperclusterOptions::default(), points, |point| {
    ///     Some([point.0, point.1])
    /// })?;
    ///
    /// let items = supercluster.tile_filtered(0, 0, 0, |point| point.2).items;
    ///
    /// // The cluster has one point left, so the tile holds that point
    /// assert_eq!(items.len(), 1);
    /// assert!(matches!(items[0].item, Item::Point { index: 0, .. }));
    ///
    /// let [lng, lat] = items[0].coordinates();
    ///
    /// assert!((lng - 10.0).abs() < 1e-9 && (lat - 50.0).abs() < 1e-9);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn tile_filtered<F>(&self, z: u8, x: u32, y: u32, filter: F) -> FilteredTile<'_, P>
    where
        F: Fn(&P) -> bool,
    {
        self.view().tile_filtered(z, x, y, filter)
    }

    /// Retrieve the items of a tile, like `tile_filtered`, but keep only the points of a
    /// `Selection` that `select` made, or of a `Combination` of selections. It takes a reference to
    /// a selection, also behind an `Arc` or a `Box`, or a combination, by value or by reference.
    /// The items borrow the selections, not the combination, so the combination can be a
    /// temporary, such as `a.and(&b)`.
    ///
    /// A selection counts the points of a cluster that pass in constant time, so the cost of the
    /// tile grows with its items, not with the points in them. The query evaluates a combination on
    /// the points of the tile only, and keeps nothing for the next query, so its cost grows with the
    /// points of the tile, not with all the points.
    ///
    /// The selections must come from this supercluster, or the items mean nothing.
    ///
    /// # Examples
    ///
    /// ```
    /// use supercluster::{Supercluster, SuperclusterOptions};
    ///
    /// // A point is a longitude, a latitude and whether it is available
    /// let points = vec![(10.0, 50.0, true), (10.001, 50.0, false), (10.002, 50.0, true)];
    /// let supercluster = Supercluster::load(SuperclusterOptions::default(), points, |point| {
    ///     Some([point.0, point.1])
    /// })?;
    /// let available = supercluster.select(|point| point.2);
    ///
    /// let items = supercluster.tile_selected(0, 0, 0, &available).items;
    ///
    /// // The cluster holds the two available points and moves to their center
    /// assert_eq!(items[0].count(), 2);
    ///
    /// let [lng, lat] = items[0].coordinates();
    ///
    /// assert!((lng - 10.001).abs() < 1e-9 && (lat - 50.0).abs() < 1e-9);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn tile_selected<'s, S: Select<'s>>(
        &'s self,
        z: u8,
        x: u32,
        y: u32,
        selection: S,
    ) -> FilteredTile<'s, P> {
        self.view().tile_selected(z, x, y, selection)
    }

    /// Determine the zoom level at which a cluster expands, like
    /// `cluster_expansion_zoom_filtered`, but count only the points of a `Selection` that `select`
    /// made, or of a `Combination` of selections, without running a filter. It takes them like
    /// `tile_selected`.
    ///
    /// # Errors
    ///
    /// Returns `SuperclusterError::ClusterNotFound` if there is no such cluster.
    pub fn cluster_expansion_zoom_selected<'s, S: Select<'s>>(
        &self,
        cluster_id: ClusterId,
        selection: S,
    ) -> Result<u8, SuperclusterError> {
        self.view()
            .cluster_expansion_zoom_selected(cluster_id, selection)
    }
}

impl<'a, P> SuperclusterView<'a, P> {
    /// Retrieve the items of a tile that have a point for which `filter` returns `true`, like
    /// `Supercluster::tile_filtered`.
    pub fn tile_filtered<F>(&self, z: u8, x: u32, y: u32, filter: F) -> FilteredTile<'a, P>
    where
        F: Fn(&P) -> bool,
    {
        let points = self.supercluster.points();

        // One buffer for the bits of every cluster, which an item takes only when some of its
        // points fail, so a cluster whose points all pass allocates nothing
        let mut bits = vec![];

        filter_tile(*self, (z, x, y), |leaves| {
            let first = leaves.start;

            match &points[leaves] {
                // A point needs no bits
                [point] => filter(point).then_some((1, Matches::All)),
                points => {
                    bits.clear();
                    bits.extend(filter_bits(points, &filter));

                    let count = bits.iter().map(|word| word.count_ones()).sum();

                    Matches::found(count, points.len(), || Matches::Local {
                        words: mem::take(&mut bits),
                        first,
                    })
                }
            }
        })
    }

    /// Retrieve the items of a tile that have a point of a selection, like
    /// `Supercluster::tile_selected`.
    pub fn tile_selected<'s, S: Select<'s>>(
        &self,
        z: u8,
        x: u32,
        y: u32,
        selection: S,
    ) -> FilteredTile<'s, P>
    where
        'a: 's,
    {
        let view: SuperclusterView<'s, P> = *self;
        let mut scratch = Scratch::default();

        filter_tile(view, (z, x, y), |leaves| {
            selection.matches(leaves, &mut scratch)
        })
    }

    /// Determine the zoom level at which a cluster expands, counting only the points of a
    /// selection, like `Supercluster::cluster_expansion_zoom_selected`.
    ///
    /// # Errors
    ///
    /// Returns `SuperclusterError::ClusterNotFound` if there is no such cluster.
    pub fn cluster_expansion_zoom_selected<'s, S: Select<'s>>(
        &self,
        cluster_id: ClusterId,
        selection: S,
    ) -> Result<u8, SuperclusterError> {
        let mut scratch = Scratch::default();

        self.supercluster
            .expansion_zoom_where(cluster_id, self.cluster_max_zoom, |leaves| {
                selection.any(leaves, &mut scratch)
            })
    }
}

/// Build the items of a tile that have a point that passes a filter, and place them. `tile` is the
/// zoom level, X and Y coordinates of the tile. `find_matches` gets the positions of the points of
/// an item in the points of the supercluster, and returns how many pass and which ones, or `None`
/// if none passes.
fn filter_tile<'a, P>(
    view: SuperclusterView<'a, P>,
    tile: (u8, u32, u32),
    mut find_matches: impl FnMut(Range<usize>) -> Option<(u32, Matches<'a>)>,
) -> FilteredTile<'a, P> {
    let supercluster = view.supercluster;
    let (z, _, tile_y) = tile;
    let z2 = tiles_per_axis(z);
    let tile_y = f64::from(tile_y);
    let mut items = vec![];

    supercluster.for_each_in_tile(view.cluster_max_zoom, tile, |handle, position, origin_x| {
        let leaves = supercluster.leaf_range(handle);
        let Some((count, matches)) = find_matches(leaves.clone()) else {
            return;
        };

        let positions = supercluster.positions();
        let only = || {
            (count == 1)
                .then(|| matches.first(leaves.clone()))
                .flatten()
        };

        let (item, position, leaves, matches) = match supercluster.item(handle, position) {
            // A cluster with one point left is that point, exactly where it is
            Item::Cluster(_) if let Some(leaf) = only() => (
                supercluster.item(leaf as u32, positions[leaf]),
                positions[leaf],
                leaf..leaf + 1,
                Matches::All,
            ),
            Item::Cluster(cluster) if count < cluster.point_count => {
                let center = center_of_left(positions, &matches, leaves.clone(), count);

                (Item::Cluster(cluster), center, leaves, matches)
            }
            item => (item, position, leaves, matches),
        };

        // A copy of the item across the antimeridian is placed by the origin of its copy of the
        // world, so it keeps its own position
        let [x, y] = supercluster.tile_position(position, z2, origin_x, tile_y);

        items.push(FilteredTileItem {
            x,
            y,
            item,
            count,
            supercluster,
            cluster_max_zoom: view.cluster_max_zoom,
            leaves: leaves.start as u32..leaves.end as u32,
            matches,
            position,
        });
    });

    FilteredTile { items }
}

/// Find the center of the `count` points at the positions `leaves` that pass, from the sum of their
/// integer positions, the same way for every kind of filter. When the mean of the integers is the
/// integers of the first of them, as for points at one place, the cluster goes onto that point
/// exactly instead.
fn center_of_left(
    positions: &Positions,
    matches: &Matches<'_>,
    leaves: Range<usize>,
    count: u32,
) -> [f64; 2] {
    let sum = matches.sum(leaves.clone(), count, positions);
    let wide = u128::from(count);

    // Points at one place sum to a multiple of their count, which few other points do, so the
    // first point is only looked up then
    if sum.iter().all(|value| value % wide == 0) {
        if let Some(leaf) = matches.first(leaves) {
            if positions
                .quantized(leaf)
                .map(|value| u128::from(value) * wide)
                == sum
            {
                return positions[leaf];
            }
        }
    }

    positions.center(sum, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::Cell;

    use crate::{test_points, CoordinateSystem, DataRange, SuperclusterOptions};

    /// A point that tells whether a filter should keep it.
    #[derive(Debug, PartialEq)]
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
    type Summary = (usize, u32, [f64; 2]);

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
                    Item::Cluster(cluster) => cluster.id.get() as usize,
                    Item::Point { index, .. } => index,
                };

                (id, tile_item.count(), [tile_item.x, tile_item.y])
            })
            .collect()
    }

    /// Load the points and get the filtered tile, which keeps the points the filter keeps.
    fn tile_filtered(points: &[(f64, f64, bool)], (z, x, y): (u8, u32, u32)) -> Vec<Summary> {
        let supercluster = load(points);

        summarize(
            &supercluster
                .tile_filtered(z, x, y, |point| point.keep)
                .items,
        )
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

        // The cluster becomes the point that is left
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

        // The cluster becomes the point, which stays left of the tile
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

        // The cluster becomes the point, which stays right of the tile
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
        let items = supercluster
            .tile_filtered(0, 0, 0, |point| point.keep)
            .items;
        let copy = items[0].clone();

        assert_eq!(
            summarize(std::slice::from_ref(&copy)),
            summarize(&items[..1])
        );
    }

    /// Create the points spread over the world that the tests share, for the filter to keep every
    /// third point.
    fn create_spread_points(count: usize) -> Vec<(f64, f64, bool)> {
        test_points::create_spread_points(count)
            .into_iter()
            .enumerate()
            .map(|(i, [lng, lat])| (lng, lat, i % 3 == 0))
            .collect()
    }

    #[test]
    fn test_filtered_tile_iterates_over_its_items() {
        let supercluster = load(&create_spread_points(500));
        let tile = supercluster.tile_filtered(0, 0, 0, |point| point.keep);
        let borrowed: Vec<_> = (&tile).into_iter().cloned().collect();
        let owned: Vec<_> = tile.clone().into_iter().collect();

        assert!(!tile.is_empty());
        assert_eq!(borrowed, tile.items);
        assert_eq!(owned, tile.items);
    }

    #[test]
    fn test_filtered_tiles_are_equal_with_the_same_points_left() {
        let supercluster = load(&create_spread_points(500));
        let tile = supercluster.tile_filtered(0, 0, 0, |point| point.keep);
        let selection = supercluster.select(|point| point.keep);

        // The same points left, from a closure and from a selection, give equal tiles, and other
        // points left do not
        assert_eq!(tile, supercluster.tile_selected(0, 0, 0, &selection));
        assert_ne!(
            tile,
            supercluster.tile_filtered(0, 0, 0, |point| !point.keep)
        );
    }

    #[test]
    fn test_filtered_tile_items_differ_with_other_points_left() {
        // Four points at the corners of a square, which form one cluster
        let corners = [(10.0, 50.0), (10.01, 50.0), (10.0, 50.01), (10.01, 50.01)];
        let points: Vec<_> = corners.iter().map(|&(lng, lat)| (lng, lat, true)).collect();
        let supercluster = load(&points);
        let kept = |kept: [usize; 2]| {
            move |point: &TestPoint| {
                kept.iter().any(|&corner| {
                    point.coordinates == Some([corners[corner].0, corners[corner].1])
                })
            }
        };

        // Opposite corners leave the same cluster with the same count at the same center, but
        // other points
        let one = supercluster.tile_filtered(0, 0, 0, kept([0, 3])).items;
        let other = supercluster.tile_filtered(0, 0, 0, kept([1, 2])).items;

        assert_eq!(
            (one[0].x, one[0].y, one[0].count()),
            (other[0].x, other[0].y, 2)
        );
        assert_eq!(one[0].coordinates(), other[0].coordinates());
        assert_ne!(one[0], other[0]);
        assert_eq!(
            one[0],
            supercluster.tile_filtered(0, 0, 0, kept([0, 3])).items[0]
        );
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
        let items = supercluster
            .tile_filtered(0, 0, 0, |point| {
                calls.set(calls.get() + 1);
                point.keep
            })
            .items;

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
                            for item in view.tile_filtered(z, x, y, filter).items {
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
        let leaves: Vec<&TestPoint> = supercluster.leaves(cluster.id).unwrap().iter().collect();
        let coordinates_of = |points: Vec<&TestPoint>| -> Vec<Option<[f64; 2]>> {
            points.iter().map(|point| point.coordinates).collect()
        };

        // The points that pass, in the order of the leaves
        let filtered = supercluster
            .tile_filtered(0, 0, 0, |point| point.keep)
            .items;
        let kept = leaves.iter().copied().filter(|point| point.keep).collect();

        assert_eq!(filtered[0].count(), 2);
        assert_eq!(
            coordinates_of(filtered[0].points().collect()),
            coordinates_of(kept)
        );

        // When every point passes, the item holds all of them
        let unfiltered = supercluster.tile_filtered(0, 0, 0, |_| true).items;

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
        let items = supercluster
            .tile_filtered(0, 0, 0, |point| point.keep)
            .items;
        let close =
            |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;

        for item in &items {
            match item.item {
                // The cluster moved to the center of the two points left, without rounding
                Item::Cluster(_) => assert!(close(item.coordinates(), [11.0, 50.0])),
                Item::Point { .. } => assert!(close(item.coordinates(), [-100.0, -30.0])),
            }
        }

        // A cluster that keeps all of its points keeps its own center
        let unfiltered = supercluster.tile_filtered(0, 0, 0, |_| true).items;
        let Item::Cluster(cluster) = unfiltered[0].item else {
            panic!("expected a cluster");
        };

        assert_eq!(unfiltered[0].coordinates(), cluster.coordinates());
    }

    #[test]
    fn test_tile_filtered_centers_many_points_left_exactly() {
        // 200 points close together, which form one cluster on zoom level 0
        let grid: Vec<[f64; 2]> = (0..200)
            .map(|i| {
                [
                    10.0 + f64::from(i % 20) * 0.01,
                    50.0 + f64::from(i / 20) * 0.01,
                ]
            })
            .collect();
        let lat_lng = CoordinateSystem::LatLng;

        // Half of them pass, then most of them, which sums the points that fail instead
        for west in [10.095, 10.015] {
            let points: Vec<_> = grid
                .iter()
                .map(|&[lng, lat]| (lng, lat, lng > west))
                .collect();
            let supercluster = load(&points);
            let kept: Vec<[f64; 2]> = grid
                .iter()
                .filter(|&&[lng, _]| lng > west)
                .map(|&point| lat_lng.normalize(point))
                .collect();
            let count = kept.len() as f64;
            let sum = kept
                .iter()
                .fold([0.0, 0.0], |sum, [x, y]| [sum[0] + x, sum[1] + y]);
            let center = lat_lng.denormalize([sum[0] / count, sum[1] / count]);

            let items = supercluster
                .tile_filtered(0, 0, 0, |point| point.keep)
                .items;
            let [lng, lat] = items[0].coordinates();

            assert_eq!(items.len(), 1);
            assert_eq!(items[0].count() as usize, kept.len());
            assert!((lng - center[0]).abs() < 1e-9 && (lat - center[1]).abs() < 1e-9);

            // Every way of filtering finds the same center, to the bit
            let selection = supercluster.select(|point| point.keep);
            let all = supercluster.select(|_| true);
            let computed = selection.and(&all).compute();
            let ways = [
                supercluster.tile_selected(0, 0, 0, &selection).items,
                supercluster
                    .tile_selected(0, 0, 0, selection.and(&all))
                    .items,
                supercluster.tile_selected(0, 0, 0, &computed).items,
            ];

            for way in ways {
                assert_eq!(way[0].coordinates(), items[0].coordinates());
                assert_eq!((way[0].x, way[0].y), (items[0].x, items[0].y));
            }
        }
    }

    #[test]
    fn test_tile_filtered_turns_a_cluster_with_one_point_left_into_it() {
        // Points at the corners of the map make the integers as coarse as in a real dataset
        let supercluster = load(&[
            (10.0137, 50.0071, true),
            (10.0237, 50.0171, false),
            (-179.0, -80.0, true),
            (179.0, 80.0, true),
        ]);
        let lat_lng = CoordinateSystem::LatLng;
        let point = lat_lng.denormalize(lat_lng.normalize([10.0137, 50.0071]));
        let selection = supercluster.select(|point| point.keep);
        let all = supercluster.select(|_| true);
        let computed = selection.and(&all).compute();

        // Every way of filtering turns the cluster into the point, exactly where it is
        let ways = [
            supercluster
                .tile_filtered(0, 0, 0, |point| point.keep)
                .items,
            supercluster.tile_selected(0, 0, 0, &selection).items,
            supercluster
                .tile_selected(0, 0, 0, selection.and(&all))
                .items,
            supercluster.tile_selected(0, 0, 0, &computed).items,
        ];

        for items in ways {
            let item = items
                .iter()
                .find(|item| matches!(item.item, Item::Point { index: 0, .. }))
                .unwrap();

            assert!(items
                .iter()
                .all(|item| matches!(item.item, Item::Point { .. })));
            assert_eq!(item.count(), 1);
            assert_eq!(item.coordinates(), point);
            assert_eq!(item.points().count(), 1);
            assert_eq!(item.expansion_zoom(), None);
        }
    }

    #[test]
    fn test_tile_filtered_puts_a_cluster_of_points_at_one_place_on_them() {
        // Three points at one place, two of which pass, and points at the corners of the map
        let supercluster = load(&[
            (10.0137, 50.0071, true),
            (10.0137, 50.0071, true),
            (10.0137, 50.0071, false),
            (-179.0, -80.0, true),
            (179.0, 80.0, true),
        ]);
        let lat_lng = CoordinateSystem::LatLng;
        let place = lat_lng.denormalize(lat_lng.normalize([10.0137, 50.0071]));
        let selection = supercluster.select(|point| point.keep);
        let all = supercluster.select(|_| true);
        let computed = selection.and(&all).compute();

        // Every way of filtering puts it exactly on the place, not on the mean of the integers
        let ways = [
            supercluster
                .tile_filtered(0, 0, 0, |point| point.keep)
                .items,
            supercluster.tile_selected(0, 0, 0, &selection).items,
            supercluster
                .tile_selected(0, 0, 0, selection.and(&all))
                .items,
            supercluster.tile_selected(0, 0, 0, &computed).items,
        ];

        for items in ways {
            let item = items
                .iter()
                .find(|item| matches!(item.item, Item::Cluster(_)))
                .unwrap();

            assert_eq!(item.count(), 2);
            assert_eq!(item.coordinates(), place);
        }
    }

    #[test]
    fn test_tile_filtered_centers_exactly_with_points_far_away() {
        // A range of 1000 by 1000, with points far outside it, and one at each end of what a
        // float holds, which no span between them could hold
        let range = DataRange::new(0.0, 0.0, 1000.0, 1000.0);
        let options = SuperclusterOptions::builder()
            .coordinate_system(CoordinateSystem::Cartesian { range })
            .build()
            .unwrap();
        let points = vec![
            (100.3, 200.7, true),
            (100.31, 200.71, true),
            (100.4, 200.8, false),
            (1e15, 1e15, true),
            (-1e300, -1e300, true),
            (1e300, 1e300, true),
        ];
        let supercluster =
            Supercluster::load(options, points, |point| Some([point.0, point.1])).unwrap();
        let items = supercluster.tile_filtered(0, 0, 0, |point| point.2).items;
        let [x, y] = items[0].coordinates();

        assert_eq!(items[0].count(), 2);
        assert!(
            (x - 100.305).abs() < 1e-9 && (y - 200.705).abs() < 1e-9,
            "{x}, {y}"
        );
    }

    #[test]
    fn test_tile_filtered_centers_the_points_left_like_loading() {
        let supercluster = load(&[(10.0, 50.0, true), (10.0, 55.0, false), (10.0, 60.0, true)]);
        let items = supercluster
            .tile_filtered(0, 0, 0, |point| point.keep)
            .items;

        // Loading averages the normalized positions, where latitudes are not evenly spread, so the
        // center of the points at 50 and 60 is north of 55
        let lat_lng = CoordinateSystem::LatLng;
        let [south, north] = [[10.0, 50.0], [10.0, 60.0]].map(|point| lat_lng.normalize(point));
        let center =
            lat_lng.denormalize([(south[0] + north[0]) / 2.0, (south[1] + north[1]) / 2.0]);
        let [lng, lat] = items[0].coordinates();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].count(), 2);
        assert!((lng - center[0]).abs() < 1e-9 && (lat - center[1]).abs() < 1e-9);
        assert!(lat > 55.1);
    }

    #[test]
    fn test_expansion_zoom_of_a_point() {
        let supercluster = load(&[(10.0, 50.0, true), (-100.0, -30.0, true)]);
        let items = supercluster
            .tile_filtered(0, 0, 0, |point| point.keep)
            .items;

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
        let items = supercluster
            .tile_filtered(0, 0, 0, |point| point[0] > 0.0)
            .items;

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
        let failing = supercluster.leaves(cluster.id).unwrap()[100];
        let filter = |point: &[f64; 2]| *point != failing;

        let items = supercluster.tile_filtered(0, 0, 0, filter).items;

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
    fn test_tile_filtered_without_points_left() {
        let points = [(10.0, 50.0, false), (12.0, 50.0, false)];

        assert!(tile_filtered(&points, WORLD_TILE).is_empty());
    }

    #[test]
    fn test_tile_filtered_with_cluster_max_zoom() {
        let supercluster = load(&[(10.0, 50.0, true), (12.0, 50.0, true)]);

        let clustered = supercluster
            .tile_filtered(1, 1, 0, |point| point.keep)
            .items;

        assert_eq!(clustered.len(), 1);
        assert_eq!(clustered[0].count(), 2);

        let unclustered = supercluster
            .with_cluster_max_zoom(0)
            .tile_filtered(1, 1, 0, |point| point.keep)
            .items;

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
    fn test_tile_filtered_with_a_point_left_out() {
        let supercluster = Supercluster::load(
            SuperclusterOptions::default(),
            vec![
                TestPoint::new(10.0, 50.0, true),
                TestPoint::without_coordinates(),
                TestPoint::new(10.001, 50.0, false),
            ],
            |point| point.coordinates,
        )
        .unwrap();

        // The point without coordinates is in no cluster, so the cluster becomes the point that
        // passes
        let items = supercluster
            .tile_filtered(0, 0, 0, |point| point.keep)
            .items;
        let [lng, lat] = items[0].coordinates();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].count(), 1);
        assert!((lng - 10.0).abs() < 1e-9 && (lat - 50.0).abs() < 1e-9);

        let selection = supercluster.select(|point| point.keep);

        assert_eq!(
            summarize(&supercluster.tile_selected(0, 0, 0, &selection).items),
            summarize(&items)
        );
    }

    #[test]
    fn test_select_calls_the_filter_once_per_clustered_point() {
        let supercluster = Supercluster::load(
            SuperclusterOptions::default(),
            vec![
                TestPoint::new(10.0, 50.0, true),
                TestPoint::without_coordinates(),
                TestPoint::new(-100.0, -30.0, false),
            ],
            |point| point.coordinates,
        )
        .unwrap();
        let calls = Cell::new(0);
        let selection = supercluster.select(|point| {
            calls.set(calls.get() + 1);
            point.keep
        });

        // The point without coordinates is left out
        assert_eq!(calls.get(), 2);
        assert_eq!(
            summarize(&supercluster.tile_selected(0, 0, 0, &selection).items),
            vec![(0, 1, [270.0, 174.0])]
        );
    }

    /// Check that two points come from the same place in the supercluster.
    fn same_points<'a>(
        a: impl Iterator<Item = &'a TestPoint>,
        b: impl Iterator<Item = &'a TestPoint>,
    ) -> bool {
        let a: Vec<*const TestPoint> = a.map(|point| point as *const _).collect();
        let b: Vec<*const TestPoint> = b.map(|point| point as *const _).collect();

        a == b
    }

    #[test]
    fn test_tile_selected_matches_tile_filtered() {
        let supercluster = load(&create_spread_points(500));
        let filters: [fn(&TestPoint) -> bool; 3] = [|point| point.keep, |_| true, |_| false];
        let mut items = 0;

        for view in [supercluster.view(), supercluster.with_cluster_max_zoom(3)] {
            for filter in filters {
                let selection = supercluster.select(filter);

                for z in 0..=5 {
                    for x in 0..1 << z {
                        for y in 0..1 << z {
                            let filtered = view.tile_filtered(z, x, y, filter).items;
                            let selected = view.tile_selected(z, x, y, &selection).items;

                            assert_eq!(summarize(&selected), summarize(&filtered));

                            for (selected, filtered) in selected.iter().zip(&filtered) {
                                assert_eq!(selected.coordinates(), filtered.coordinates());
                                assert_eq!(selected.expansion_zoom(), filtered.expansion_zoom());
                                assert!(same_points(selected.points(), filtered.points()));

                                if let Item::Cluster(cluster) = selected.item {
                                    // The points that pass, in the order of the leaves
                                    let leaves = supercluster.leaves(cluster.id).unwrap();

                                    assert!(same_points(
                                        selected.points(),
                                        leaves.iter().filter(|point| filter(point))
                                    ));

                                    assert_eq!(
                                        view.cluster_expansion_zoom_selected(
                                            cluster.id, &selection
                                        ),
                                        view.cluster_expansion_zoom_filtered(cluster.id, filter)
                                    );
                                }
                            }

                            items += selected.len();
                        }
                    }
                }
            }
        }

        assert!(items > 0);
    }

    #[test]
    fn test_cluster_expansion_zoom_selected_of_unknown_cluster() {
        let supercluster = load(&[(10.0, 50.0, true), (10.001, 50.0, true)]);
        let selection = supercluster.select(|point| point.keep);

        assert_eq!(
            supercluster.cluster_expansion_zoom_selected(ClusterId::new(100000), &selection),
            Err(SuperclusterError::ClusterNotFound)
        );
    }

    #[test]
    fn test_tile_selected_with_a_selection_of_another_supercluster() {
        let small = load(&[(10.0, 50.0, true)]);
        let large = load(&create_spread_points(500));
        let selection = small.select(|point| point.keep);

        // The items mean nothing, but the queries do not panic
        for item in large.tile_selected(0, 0, 0, &selection).items {
            assert!(item.points().count() <= item.count() as usize);
            item.expansion_zoom();
        }
    }

    #[test]
    fn test_tile_selected_with_a_selection_of_a_supercluster_elsewhere() {
        // One supercluster near a corner of the map, with a point at the other corner, and one far
        // from it, so the integer positions of the two differ the most
        let spread = |i: u32, size: f64| {
            let i = f64::from(i);

            ((i * 0.618).fract() * size, (i * 0.755).fract() * size)
        };
        let near: Vec<_> = (0..500)
            .map(|i| (spread(i, 0.5), i % 2 == 0))
            .map(|((x, y), keep)| (-179.0 + x, -80.0 + y, keep))
            .chain([(179.0, 80.0, true)])
            .collect();
        let far: Vec<_> = (0..2000)
            .map(|i| spread(i, 20.0))
            .map(|(x, y)| (100.0 + x, 50.0 + y, true))
            .collect();
        let near = load(&near);
        let far = load(&far);
        let selection = near.select(|point| point.keep);
        let all = far.select(|_| true);
        let items = |items: Vec<FilteredTileItem<'_, TestPoint>>| -> Vec<_> {
            items
                .iter()
                .map(|item| (item.count(), item.x, item.y, item.coordinates()))
                .collect()
        };

        // The running sums of the selection are of other positions, so the query sums its bits
        // with its own positions, as it does for the same bits in a combination
        for z in 0..=5 {
            for x in 0..1 << z {
                for y in 0..1 << z {
                    assert_eq!(
                        items(far.tile_selected(z, x, y, &selection).items),
                        items(far.tile_selected(z, x, y, selection.and(&all)).items),
                        "tile {z}/{x}/{y}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_tile_selected_without_loaded_points() {
        let supercluster = load(&[]);
        let selection = supercluster.select(|point| point.keep);

        assert!(supercluster
            .tile_selected(0, 0, 0, &selection)
            .items
            .is_empty());
    }

    #[test]
    fn test_tile_selected_with_one_point() {
        let supercluster = load(&[(10.0, 50.0, true)]);
        let kept = supercluster.select(|point| point.keep);
        let rejected = supercluster.select(|point| !point.keep);

        assert_eq!(
            summarize(&supercluster.tile_selected(0, 0, 0, &kept).items),
            vec![(0, 1, [270.0, 174.0])]
        );
        assert!(supercluster
            .tile_selected(0, 0, 0, &rejected)
            .items
            .is_empty());
    }

    #[test]
    fn test_tile_selected_with_cartesian_coordinates() {
        let range = DataRange::new(0.0, 0.0, 100.0, 100.0);
        let options = SuperclusterOptions::builder()
            .coordinate_system(CoordinateSystem::Cartesian { range })
            .build()
            .unwrap();
        let supercluster = Supercluster::load(
            options,
            vec![
                TestPoint::new(10.0, 10.0, true),
                TestPoint::new(10.5, 10.0, false),
                TestPoint::new(11.0, 10.0, true),
                TestPoint::new(90.0, 90.0, false),
            ],
            |point| point.coordinates,
        )
        .unwrap();
        let selection = supercluster.select(|point| point.keep);
        let items = supercluster.tile_selected(0, 0, 0, &selection).items;

        assert_eq!(
            summarize(&items),
            summarize(
                &supercluster
                    .tile_filtered(0, 0, 0, |point| point.keep)
                    .items
            )
        );

        // The cluster moves to the center of the two points that pass, and the far point fails
        let [x, y] = items[0].coordinates();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].count(), 2);
        assert!((x - 10.5).abs() < 1e-9 && (y - 10.0).abs() < 1e-9);
    }
}
