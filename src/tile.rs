//! # Tile module
//!
//! Contains the items of a tile, the queries that build a tile, and the view whose queries keep
//! only the points that pass a filter. A tile without a filter is a tile where every point passes.
//!
//! With a filter, a cluster stays in the tile when at least one of its points passes. When some of its points fail, the cluster
//! moves to the exact center of the points that pass, and a cluster with a single point left
//! becomes that point. The coordinates come from the supercluster, which got them when the points
//! were loaded.
//!
//! The filter is a `Filter`, whose closure runs on the points of the tile, a `CachedFilter`, which
//! ran a closure on every point once, or a combined filter of cached filters that the query
//! evaluates on the points of the tile.

use std::{
    fmt::{self, Debug, Formatter},
    ops::Range,
    ptr, slice, vec,
};

use crate::{
    filter::{filter_bits, Matches, Scratch},
    sums::Positions,
    supercluster::{tiles_per_axis, Bounds, TileItems},
    AsFilter, CachedFilter, ClusterId, Filter, Item, Supercluster, SuperclusterError,
    SuperclusterView,
};

/// The items of a tile, which `tile` returns, on a supercluster or one of its views.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub struct Tile<'a, P> {
    /// The clusters and points of the tile, which also holds the ones of the buffer around it. In
    /// a filtered tile, only the ones with a point that passes. They come in no set order, as
    /// `Supercluster::tile` describes.
    pub items: Vec<TileItem<'a, P>>,
}

impl<P> Tile<'_, P> {
    /// Check whether the tile has no items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<'a, P> IntoIterator for Tile<'a, P> {
    type Item = TileItem<'a, P>;
    type IntoIter = vec::IntoIter<TileItem<'a, P>>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<'t, 'a, P> IntoIterator for &'t Tile<'a, P> {
    type Item = &'t TileItem<'a, P>;
    type IntoIter = slice::Iter<'t, TileItem<'a, P>>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

// Written by hand, because a derive would also require the point type to be `Clone`, and the items
// only hold references to points.
impl<P> Clone for Tile<'_, P> {
    fn clone(&self) -> Self {
        Tile {
            items: self.items.clone(),
        }
    }
}

/// An item of a tile: a cluster or a point, with its position in the tile. In a filtered tile, it
/// has at least one point that passes the filter.
#[non_exhaustive]
pub struct TileItem<'a, P> {
    /// Horizontal position in the tile, in extent units from the left edge.
    x: i32,

    /// Vertical position in the tile, in extent units from the top edge.
    y: i32,

    /// The number of points of the item, or in a filtered tile, the ones that pass the filter.
    count: u32,

    /// The supercluster of the tile.
    supercluster: &'a Supercluster<P>,

    /// The highest zoom level that the view that built the tile clusters on, which the expansion
    /// zoom is capped by.
    cluster_max_zoom: u8,

    /// The handle of the item in the KD-trees, which tells which cluster or point it is and where
    /// its points are.
    handle: u32,

    /// Which points of the item pass the filter, which is all of them without a filter.
    matches: Matches<'a>,

    /// Where the item is, normalized to the [0..1] range the KD-trees use, which only a call to
    /// `coordinates` converts. The tile found it to place the item, so `coordinates` does not find
    /// the center of the points that pass of a cluster again.
    position: [f64; 2],
}

impl<'a, P> TileItem<'a, P> {
    /// Get the horizontal position in the tile, in extent units from the left edge. An item in the
    /// buffer around the tile can be negative or above the extent. In a filtered tile, a cluster
    /// with points that fail the filter is placed at the center of the points that pass.
    pub fn x(&self) -> i32 {
        self.x
    }

    /// Get the vertical position in the tile, in extent units from the top edge. An item in the
    /// buffer around the tile can be negative or above the extent. In a filtered tile, a cluster
    /// with points that fail the filter is placed at the center of the points that pass.
    pub fn y(&self) -> i32 {
        self.y
    }

    /// Get the cluster, as the supercluster has it, or the point. In a filtered tile, a cluster
    /// with one point left is that point, and the `total_count` of a cluster counts all of
    /// its points, where `count` counts the ones that pass the filter.
    ///
    /// It is looked up from the supercluster on each call, so an item holds only a handle to it.
    pub fn item(&self) -> Item<'a, P> {
        self.supercluster.item_of(self.handle)
    }

    /// Get the number of points of the item: of the cluster, or 1 for a point. In a filtered tile,
    /// it counts only the points that pass the filter.
    pub fn count(&self) -> u32 {
        self.count
    }

    /// Get the position in the coordinate system of the supercluster, the longitude and latitude
    /// for `LngLat`. It is the center of a cluster, the position of a point, or in a filtered tile,
    /// the center of the points that pass for a cluster with points that fail. Unlike `x` and `y`,
    /// it is not on the grid of the tile, and a copy of an item across the antimeridian keeps its
    /// own coordinates. It is converted on each call, so a query that does not need it does not
    /// pay for it.
    pub fn coordinates(&self) -> [f64; 2] {
        self.supercluster
            .options()
            .coordinate_system
            .denormalize(self.position)
    }

    /// Iterate over the points of the item: the points of the cluster, in the order
    /// `Supercluster::leaves` returns them, or the point itself. In a filtered tile, only the
    /// points that pass the filter. The first point is `leaves().next()`.
    pub fn leaves(&self) -> impl Iterator<Item = &'a P> + '_ {
        let points = self.supercluster.points();

        self.matches
            .passing(self.leaf_range())
            .map(move |leaf| &points[leaf])
    }

    /// Determine the zoom level at which the cluster expands, up to the highest zoom level of the
    /// view that built the tile, like `cluster_expansion_zoom`. In a filtered tile, it counts only
    /// the points that pass the filter, like `FilteredView::cluster_expansion_zoom` with the same
    /// filter, and it reuses what the query found while the tile was built, so it does not run a
    /// closure or combine cached filters again.
    ///
    /// Returns `None` for a point.
    ///
    /// # Examples
    ///
    /// ```
    /// use supercluster::{Supercluster, SuperclusterOptions};
    ///
    /// let points = vec![[10.0, 50.0], [10.001, 50.0]];
    /// let supercluster = Supercluster::load(SuperclusterOptions::default(), points, |point| {
    ///     Some(*point)
    /// })?;
    /// let tile = supercluster.tile(0, 0, 0);
    ///
    /// // The two points form a cluster, which splits on a zoom level a map can zoom in to
    /// assert!(tile.items[0].expansion_zoom().is_some());
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn expansion_zoom(&self) -> Option<u8> {
        match self.matches {
            // Every child counts, so the cluster splits on the zoom level after it formed
            Matches::All => self
                .supercluster
                .expansion_zoom_of_handle(self.handle, self.cluster_max_zoom),
            // The handle of a point is no cluster, so the search finds none and gives `None`
            _ => self
                .supercluster
                .expansion_zoom_where(
                    ClusterId::new(self.handle),
                    self.cluster_max_zoom,
                    |leaves| self.matches.any(leaves),
                )
                .ok(),
        }
    }

    /// Get the positions of the points of the item in the points of the supercluster.
    fn leaf_range(&self) -> Range<usize> {
        self.supercluster.leaf_range(self.handle)
    }

    /// Check whether two items keep the same points of the same supercluster without reading
    /// them, which holds when every point passes or they share a cached filter.
    fn keeps_the_same_points(&self, other: &Self) -> bool {
        ptr::eq(self.supercluster, other.supercluster)
            && self.handle == other.handle
            && match (&self.matches, &other.matches) {
                (Matches::All, Matches::All) => true,
                (Matches::Shared(cached), Matches::Shared(other)) => ptr::eq(*cached, *other),
                _ => false,
            }
    }
}

// Written by hand, because a derive would also require the point type to be `Clone`, and an item
// only holds references to points.
impl<P> Clone for TileItem<'_, P> {
    fn clone(&self) -> Self {
        TileItem {
            x: self.x,
            y: self.y,
            count: self.count,
            supercluster: self.supercluster,
            cluster_max_zoom: self.cluster_max_zoom,
            handle: self.handle,
            matches: self.matches.clone(),
            position: self.position,
        }
    }
}

// Written by hand, because a derive would print the whole supercluster.
impl<P: Debug> Debug for TileItem<'_, P> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TileItem")
            .field("x", &self.x)
            .field("y", &self.y)
            .field("coordinates", &self.coordinates())
            .field("item", &self.item())
            .field("count", &self.count)
            .field("expansion_zoom", &self.expansion_zoom())
            .finish_non_exhaustive()
    }
}

// Written by hand, because a derive would compare the whole supercluster. Items are equal at the
// same position with the same cluster or point and the same points that pass, counted first, which
// is cheaper, and read one by one only when the items do not keep the same points of the same
// supercluster. Clusters also need to expand on the same zoom level, which only needs finding when
// the views that built them cluster up to other zoom levels.
impl<P: PartialEq> PartialEq for TileItem<'_, P> {
    fn eq(&self, other: &Self) -> bool {
        self.x == other.x
            && self.y == other.y
            && self.item() == other.item()
            && self.count == other.count
            && (self.cluster_max_zoom == other.cluster_max_zoom
                || self.expansion_zoom() == other.expansion_zoom())
            && (self.keeps_the_same_points(other) || self.leaves().eq(other.leaves()))
    }
}

impl<P> Supercluster<P> {
    /// Make a filter that runs `filter` on the points of each query of the view that gets it, for
    /// `with_filter`. It runs nothing now and caches
    /// nothing, so every query reads the current data. It suits a condition that changes with every
    /// request, such as a text search.
    ///
    /// A query runs `filter` once for every point of every item of its tile, so a zoomed-out tile,
    /// which covers the most points, costs the most. To run a filter once for many queries, use
    /// `cache_filter`.
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
    /// let items = supercluster
    ///     .with_filter(supercluster.filter(|point| point.2))?
    ///     .tile(0, 0, 0)
    ///     .items;
    ///
    /// // The cluster has one point left, so the tile holds that point
    /// assert_eq!(items.len(), 1);
    /// assert!(matches!(items[0].item(), Item::Point { index: 0, .. }));
    ///
    /// let [lng, lat] = items[0].coordinates();
    ///
    /// assert!((lng - 10.0).abs() < 1e-9 && (lat - 50.0).abs() < 1e-9);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn filter<F>(&self, filter: F) -> Filter<P, F>
    where
        F: Fn(&P) -> bool,
    {
        Filter::new(filter)
    }

    /// Run `filter` once on every point that was clustered, and cache which ones pass, for the
    /// queries of `with_filter` to reuse. The points do not change after `load`, so run it again
    /// only when the filter reads data that changes, outside the points or shared through them.
    ///
    /// It runs `filter` on every clustered point, one at a time, so `filter` can update state, such
    /// as statistics. The points come in the supercluster's own order, not the order they were
    /// loaded in, so decide on each point by itself.
    ///
    /// The scan does not branch on what `filter` returns, so it runs as fast as `filter` allows.
    /// It then sums the positions of the points that pass, so a tile finds the center of the
    /// points left in a cluster without reading them.
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
    ///
    /// // Run the filter once, then reuse it for every tile until the availability changes
    /// let available = supercluster.cache_filter(|point| point.2);
    /// let items = supercluster.with_filter(&available)?.tile(0, 0, 0).items;
    ///
    /// // The cluster holds the two available points and moves to their center
    /// assert_eq!(items[0].count(), 2);
    ///
    /// let [lng, lat] = items[0].coordinates();
    ///
    /// assert!((lng - 10.001).abs() < 1e-9 && (lat - 50.0).abs() < 1e-9);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn cache_filter<F>(&self, filter: F) -> CachedFilter
    where
        F: FnMut(&P) -> bool,
    {
        let points = &self.points()[..self.clustered_points()];

        CachedFilter::new(
            filter_bits(points, filter).collect(),
            Some(self.positions()),
        )
    }

    /// Get a view whose queries keep only the points that pass `filter`. A cluster without any
    /// point left is left out, and a cluster with points that fail moves to the exact center of
    /// the points left. The view checks `filter` once, here, so its queries cannot fail on it.
    ///
    /// `filter` is one of these, and the queries give the same results for the same condition:
    ///
    /// - A `Filter` that `filter` made, by value or by reference. A query runs its closure once
    ///   for every point of every item it reads, so its cost grows with the points of the tile.
    /// - A `CachedFilter` that `cache_filter` made, behind a reference, an `Arc` or a `Box`. It
    ///   counts the points of a cluster that pass in constant time, so the cost grows with the
    ///   items of the tile, not with the points in them.
    /// - A `CombinedFilter` of cached filters, by value or by reference, which a query evaluates
    ///   on the points it reads only.
    ///
    /// The view holds `filter`. The items of its tiles borrow the cached filters, not the view, a
    /// `Filter` or a combined filter, so those can be temporaries, such as
    /// `supercluster.filter(...)` or `a.and(&b)`.
    ///
    /// # Errors
    ///
    /// Returns `SuperclusterError::ForeignCachedFilter` if a cached filter, on its own or in a
    /// combined filter, comes from another supercluster, even one that holds the same points. A
    /// `Filter` runs on the points of the query, so it works with every supercluster.
    ///
    /// # Examples
    ///
    /// ```
    /// use supercluster::{Supercluster, SuperclusterOptions};
    ///
    /// // A station is a longitude, a latitude, its power in kW and whether it is available
    /// let stations = vec![(10.0, 50.0, 150, true), (10.001, 50.0, 150, false), (10.002, 50.0, 11, true)];
    /// let supercluster = Supercluster::load(SuperclusterOptions::default(), stations, |station| {
    ///     Some([station.0, station.1])
    /// })?;
    /// let available = supercluster.cache_filter(|station| station.3);
    ///
    /// // A condition per request, a cached one, or cached ones combined, give the same tile
    /// let fast = supercluster.filter(|station| station.2 >= 50 && station.3);
    /// let cached_fast = supercluster.cache_filter(|station| station.2 >= 50);
    ///
    /// let filtered = supercluster.with_filter(&fast)?.tile(0, 0, 0);
    ///
    /// assert_eq!(filtered.items[0].count(), 1);
    /// assert_eq!(supercluster.with_filter(cached_fast.and(&available))?.tile(0, 0, 0), filtered);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn with_filter<'s, S: AsFilter<'s, P>>(
        &'s self,
        filter: S,
    ) -> Result<FilteredView<'s, P, S>, SuperclusterError> {
        self.view().with_filter(filter)
    }
}

impl<'a, P> SuperclusterView<'a, P> {
    /// Get a view that keeps only the points that pass `filter` and clusters up to the zoom level
    /// of this view, like `Supercluster::with_filter`.
    ///
    /// # Errors
    ///
    /// Returns `SuperclusterError::ForeignCachedFilter` if a cached filter comes from another
    /// supercluster.
    pub fn with_filter<'s, S: AsFilter<'s, P>>(
        &self,
        filter: S,
    ) -> Result<FilteredView<'s, P, S>, SuperclusterError>
    where
        'a: 's,
    {
        if !filter.belongs_to(self.supercluster.positions()) {
            return Err(SuperclusterError::ForeignCachedFilter);
        }

        Ok(FilteredView {
            view: *self,
            filter,
        })
    }
}

/// A view of a supercluster whose queries keep only the points that pass a filter. Create one
/// with `Supercluster::with_filter` or `SuperclusterView::with_filter`.
pub struct FilteredView<'s, P, S> {
    /// The view to query, which sets the zoom level to cluster on.
    view: SuperclusterView<'s, P>,

    /// The filter the points of every query pass, which belongs to the supercluster of the view.
    filter: S,
}

impl<'s, P, S: AsFilter<'s, P>> FilteredView<'s, P, S> {
    /// Retrieve the items of a tile, like `Supercluster::tile`, that have a point that passes the
    /// filter. Map the items to the type of your response with an iterator.
    ///
    /// The items reuse what the query found, for example in `TileItem::expansion_zoom`. A cluster
    /// that the tile also shows as a copy across the antimeridian is filtered once for each copy.
    pub fn tile(&self, z: u8, x: u32, y: u32) -> Tile<'s, P> {
        let points = self.view.supercluster.points();

        filter_tile(self.view, (z, x, y), |leaves, scratch| {
            self.filter.matches(points, leaves, scratch)
        })
    }

    /// Determine the zoom level at which a cluster expands, like
    /// `Supercluster::cluster_expansion_zoom`, but count only the points that pass the filter. A
    /// cluster counts when one of its points passes. When a single cluster or point is left after
    /// the filter, the cluster does not look different on the next zoom level, so the search goes
    /// on with what is left. Above the zoom level to cluster on, this is that zoom level plus one.
    ///
    /// # Errors
    ///
    /// Returns `SuperclusterError::ClusterNotFound` if there is no such cluster.
    pub fn cluster_expansion_zoom(&self, cluster_id: ClusterId) -> Result<u8, SuperclusterError> {
        let supercluster = self.view.supercluster;
        let points = supercluster.points();
        let mut scratch = Scratch::default();

        supercluster.expansion_zoom_where(cluster_id, self.view.cluster_max_zoom, |leaves| {
            self.filter.any(points, leaves, &mut scratch)
        })
    }
}

// Written by hand, because a derive would also require the point type to be `Clone`, and a view
// only holds a reference to the supercluster.
impl<P, S: Clone> Clone for FilteredView<'_, P, S> {
    fn clone(&self) -> Self {
        FilteredView {
            view: self.view,
            filter: self.filter.clone(),
        }
    }
}

impl<P, S: Copy> Copy for FilteredView<'_, P, S> {}

// Written by hand, because a derive would print the whole supercluster, and a closure has nothing
// to print.
impl<P, S> Debug for FilteredView<'_, P, S> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FilteredView")
            .field("cluster_max_zoom", &self.view.cluster_max_zoom)
            .finish_non_exhaustive()
    }
}

/// What an item of a tile keeps, which a tile builder decides for every item of the KD-tree.
struct Placed<'a> {
    /// The number of points of the item, or of the ones that pass the filter.
    count: u32,

    /// The handle of the cluster or point the item is.
    handle: u32,

    /// Which points of the item pass the filter.
    matches: Matches<'a>,

    /// Where the item is, normalized to the [0..1] range the KD-trees use.
    position: [f64; 2],
}

/// Build the items of a tile and place them. `tile` is the zoom level, X and Y coordinates of the
/// tile. `place` gets the handle and the normalized position of every item of the KD-tree in the
/// tile, the bounds of the tile and its buffer, and buffers to reuse, and returns what the item
/// keeps, or `None` to leave it out. `kind` tells whether `place` moves clusters, and if so, it also
/// gets the clusters that can move into the bounds. Every tile is built through it, so a tile with and without a
/// filter place their items the same way, and keep the ones that end up in the bounds.
fn build_tile<'a, P>(
    view: SuperclusterView<'a, P>,
    tile: (u8, u32, u32),
    kind: TileItems,
    mut place: impl FnMut(u32, [f64; 2], Bounds, &mut Scratch) -> Option<Placed<'a>>,
) -> Tile<'a, P> {
    let supercluster = view.supercluster;
    let (z, _, tile_y) = tile;
    let z2 = tiles_per_axis(z);
    let tile_y = f64::from(tile_y);
    let mut items = vec![];
    let mut scratch = Scratch::default();

    supercluster.for_each_in_tile(
        view.cluster_max_zoom,
        tile,
        kind,
        |handle, position, origin_x, bounds| {
            let Some(placed) = place(handle, position, bounds, &mut scratch) else {
                return;
            };

            if !bounds.contains(placed.position) {
                scratch.give_back(placed.matches);

                return;
            }

            // A copy of the item across the antimeridian is placed by the origin of its copy of the
            // world, so it keeps its own position
            let [x, y] = supercluster.tile_position(placed.position, z2, origin_x, tile_y);

            items.push(TileItem {
                x,
                y,
                count: placed.count,
                supercluster,
                cluster_max_zoom: view.cluster_max_zoom,
                handle: placed.handle,
                matches: placed.matches,
                position: placed.position,
            });
        },
    );

    Tile { items }
}

/// Build the items of a tile that have a point that passes a filter. `tile` is the zoom level, X
/// and Y coordinates of the tile. `find_matches` gets the positions of the points of an item in the
/// points of the supercluster and buffers to reuse, and returns how many pass and which ones, or
/// `None` if none passes.
fn filter_tile<'a, P>(
    view: SuperclusterView<'a, P>,
    tile: (u8, u32, u32),
    mut find_matches: impl FnMut(Range<usize>, &mut Scratch) -> Option<(u32, Matches<'a>)>,
) -> Tile<'a, P> {
    let supercluster = view.supercluster;

    build_tile(
        view,
        tile,
        TileItems::MoveClusters,
        |handle, position, bounds, scratch| {
            let leaves = supercluster.leaf_range(handle);

            // An item outside the bounds is only worth filtering if it can move into them
            if !supercluster.can_end_up_in(handle, position, bounds) {
                return None;
            }

            let (count, matches) = find_matches(leaves.clone(), scratch)?;
            let positions = supercluster.positions();
            let only = || {
                (count == 1)
                    .then(|| matches.first(leaves.clone()))
                    .flatten()
            };

            let placed = if count as usize == leaves.len() {
                Placed {
                    count,
                    handle,
                    matches,
                    position,
                }
            } else if let Some(leaf) = only() {
                // A cluster with one point left is that point, exactly where it is, and the next item
                // reuses its bits
                scratch.give_back(matches);

                Placed {
                    count,
                    handle: leaf as u32,
                    matches: Matches::All,
                    position: positions[leaf],
                }
            } else {
                let center = center_of_left(positions, &matches, leaves.clone(), count);

                Placed {
                    count,
                    handle,
                    matches,
                    position: center,
                }
            };

            Some(placed)
        },
    )
}

/// Build the items of a tile where every point passes. `tile` is the zoom level, X and Y
/// coordinates of the tile. No item moves or loses points, so it skips the work of `filter_tile`
/// for each item, which `tile` measurably pays for on tiles with many items.
pub(crate) fn every_point_tile<P>(
    view: SuperclusterView<'_, P>,
    tile: (u8, u32, u32),
) -> Tile<'_, P> {
    let supercluster = view.supercluster;

    build_tile(
        view,
        tile,
        TileItems::StayInPlace,
        |handle, position, _, _| {
            Some(Placed {
                count: supercluster.leaf_range(handle).len() as u32,
                handle,
                matches: Matches::All,
                position,
            })
        },
    )
}

/// Find the center of the `count` points at the positions `leaves` that pass, from the sum of their
/// integer positions, the same way for every kind of filter. When the mean of the integers is the
/// integers of the first of them, as for points at one place, the cluster goes onto that point
/// exactly instead. When one of them is far away, the center comes from their floats.
fn center_of_left(
    positions: &Positions,
    matches: &Matches<'_>,
    leaves: Range<usize>,
    count: u32,
) -> [f64; 2] {
    // The integers of a point far away are not where it is, so the center of points with one of
    // them is found from their floats, in the same order on every path
    let far = positions.far_in(leaves.clone());

    if far.iter().any(|&leaf| matches.any(leaf..leaf + 1)) {
        return positions.mean(matches.passing(leaves), count);
    }

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

    use crate::{
        test_points::{self, TestPoint},
        CoordinateSystem, DataRange, SuperclusterOptions,
    };

    /// The ID or index, the number of points left and the position in the tile of an item.
    type Summary = (usize, u32, [i32; 2]);

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
    fn summarize(items: &[TileItem<'_, TestPoint>]) -> Vec<Summary> {
        items
            .iter()
            .map(|tile_item| {
                let id = match tile_item.item() {
                    Item::Cluster(cluster) => cluster.id().get() as usize,
                    Item::Point { index, .. } => index,
                };

                (id, tile_item.count(), [tile_item.x(), tile_item.y()])
            })
            .collect()
    }

    /// Load the points and get the filtered tile, which keeps the points the filter keeps.
    fn filtered_tile(points: &[(f64, f64, bool)], (z, x, y): (u8, u32, u32)) -> Vec<Summary> {
        let supercluster = load(points);

        summarize(
            &supercluster
                .with_filter(supercluster.filter(|point| point.keep))
                .unwrap()
                .tile(z, x, y)
                .items,
        )
    }

    #[test]
    fn test_filtered_tile() {
        let points = [
            (10.0, 50.0, true),
            (12.0, 50.0, true),
            (-100.0, -30.0, true),
        ];
        let mut tile = filtered_tile(&points, WORLD_TILE);

        tile.sort_by_key(|(_, count, _)| *count);

        assert_eq!(tile.len(), 2);

        // The point that is not clustered has its index.
        assert_eq!((tile[0].0, tile[0].1), (2, 1));

        // A cluster without filtered points stays where the supercluster put it.
        assert_eq!(tile[1].1, 2);
        assert_eq!(tile[1].2[0], 272);
    }

    #[test]
    fn test_filtered_tile_with_one_point_left() {
        let points = [(10.0, 50.0, true), (12.0, 50.0, false)];
        let tile = filtered_tile(&points, WORLD_TILE);

        assert_eq!(tile.len(), 1);

        // The cluster becomes the point that is left
        assert_eq!(tile[0].1, 1);
        assert_eq!(tile[0].2[0], 270);
    }

    #[test]
    fn test_filtered_tile_with_few_points_left() {
        let points = [(10.0, 50.0, true), (12.0, 50.0, true), (14.0, 50.0, false)];
        let tile = filtered_tile(&points, WORLD_TILE);

        assert_eq!(tile.len(), 1);

        // The cluster moves to the center of the points that are left, not of all three.
        assert_eq!(tile[0].1, 2);
        assert_eq!(tile[0].2[0], 272);
    }

    #[test]
    fn test_filtered_tile_with_one_point_left_at_the_west_edge() {
        // Tile 0 only holds these points as a copy that sits left of the tile
        let tile = (1, 0, 0);
        let filtered = filtered_tile(&[(179.0, 10.0, true), (179.5, 10.0, false)], tile);
        let unfiltered = filtered_tile(&[(179.0, 10.0, true), (179.5, 10.0, true)], tile);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].1, 1);

        // The cluster becomes the point, which stays left of the tile
        assert_eq!(filtered[0].2[0], -3);
        assert_eq!(filtered[0].2[1], unfiltered[0].2[1]);
    }

    #[test]
    fn test_filtered_tile_with_one_point_left_at_the_east_edge() {
        // The last tile only holds these points as a copy that sits right of the tile
        let tile = (1, 1, 0);
        let filtered = filtered_tile(&[(-179.0, 10.0, true), (-179.5, 10.0, false)], tile);
        let unfiltered = filtered_tile(&[(-179.0, 10.0, true), (-179.5, 10.0, true)], tile);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].1, 1);

        // The cluster becomes the point, which stays right of the tile
        assert_eq!(filtered[0].2[0], 515);
        assert_eq!(filtered[0].2[1], unfiltered[0].2[1]);
    }

    #[test]
    fn test_filtered_tile_with_a_cluster_that_moves_into_the_next_tile() {
        let points = create_points_of_a_cluster_that_moves();

        // The point is in the buffer of the left tile, and in the right tile itself
        assert_eq!(filtered_tile(&points, (1, 0, 0)), [(101, 1, [515, 483])]);
        assert_eq!(filtered_tile(&points, (1, 1, 0)), [(101, 1, [3, 483])]);
    }

    /// Create points whose cluster at zoom 1 forms around the failing ones, more than the radius
    /// left of the edge between the two tiles, while the only point that passes is just right of
    /// that edge.
    fn create_points_of_a_cluster_that_moves() -> Vec<(f64, f64, bool)> {
        let mut points = vec![(-15.0, 10.0, false); 100];

        points.extend([(-6.0, 10.0, false), (0.9, 10.0, true)]);

        points
    }

    #[test]
    fn test_filtered_tile_shows_every_point_that_passes_once_in_the_tiles() {
        let extent = SuperclusterOptions::default().extent as i32;

        for points in [
            create_spread_points(2000),
            create_points_of_a_cluster_that_moves(),
        ] {
            let supercluster = load(&points);
            let filter = supercluster.filter(|point| point.keep);
            let passing = points.iter().filter(|(_, _, keep)| *keep).count();

            // Without the buffers, the tiles of a zoom level split the world, so they hold every
            // point that passes once, wherever the filter moves a cluster to
            for z in 0..=4 {
                let mut shown = 0;

                for x in 0..1 << z {
                    for y in 0..1 << z {
                        shown += supercluster
                            .with_filter(&filter)
                            .unwrap()
                            .tile(z, x, y)
                            .items
                            .iter()
                            .filter(|tile_item| {
                                (0..extent).contains(&tile_item.x())
                                    && (0..extent).contains(&tile_item.y())
                            })
                            .map(|tile_item| tile_item.count() as usize)
                            .sum::<usize>();
                    }
                }

                assert_eq!(shown, passing, "zoom {z}");
            }
        }
    }

    #[test]
    fn test_filtered_tile_with_a_point_that_is_not_clustered() {
        // The two points are far apart, so the filter rejects one of them on its own
        let points = [(10.0, 50.0, false), (-100.0, -30.0, true)];
        let tile = filtered_tile(&points, WORLD_TILE);

        assert_eq!(tile.len(), 1);

        // The point that is left has its index
        assert_eq!((tile[0].0, tile[0].1), (1, 1));
    }

    #[test]
    fn test_clone_of_a_filtered_tile_without_a_clone_point() {
        let supercluster = load(&[(10.0, 50.0, true), (12.0, 50.0, true)]);
        let items = supercluster
            .with_filter(supercluster.filter(|point| point.keep))
            .unwrap()
            .tile(0, 0, 0)
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
        let tile = supercluster
            .with_filter(supercluster.filter(|point| point.keep))
            .unwrap()
            .tile(0, 0, 0);
        let borrowed: Vec<_> = (&tile).into_iter().cloned().collect();
        let owned: Vec<_> = tile.clone().into_iter().collect();

        assert!(!tile.is_empty());
        assert_eq!(borrowed, tile.items);
        assert_eq!(owned, tile.items);
    }

    #[test]
    fn test_filtered_tiles_are_equal_with_the_same_points_left() {
        let supercluster = load(&create_spread_points(500));
        let tile = supercluster
            .with_filter(supercluster.filter(|point| point.keep))
            .unwrap()
            .tile(0, 0, 0);
        let cached_filter = supercluster.cache_filter(|point| point.keep);

        // The same points left, from a closure and from a cached filter, give equal tiles, and
        // other points left do not
        assert_eq!(
            tile,
            supercluster
                .with_filter(&cached_filter)
                .unwrap()
                .tile(0, 0, 0)
        );
        assert_ne!(
            tile,
            supercluster
                .with_filter(supercluster.filter(|point| !point.keep))
                .unwrap()
                .tile(0, 0, 0)
        );
    }

    #[test]
    fn test_filtered_tile_items_of_views_are_equal_when_they_expand_on_the_same_zoom_level() {
        // Two points that pass and split long before zoom level 15, and one that fails
        let supercluster = load(&[(10.0, 50.0, true), (10.5, 50.0, true), (10.2, 50.0, false)]);
        let tile = |cluster_max_zoom| {
            supercluster
                .with_cluster_max_zoom(cluster_max_zoom)
                .with_filter(supercluster.filter(|point| point.keep))
                .unwrap()
                .tile(0, 0, 0)
        };
        let [lower, higher, lowest] = [tile(15), tile(16), tile(0)];

        assert!(matches!(lower.items[0].item(), Item::Cluster(_)));
        assert!(lower.items[0].expansion_zoom() < Some(15));
        assert_eq!(lower, higher);

        // A view that clusters up to zoom level 0 expands the cluster on zoom level 1
        assert_eq!(lowest.items[0].expansion_zoom(), Some(1));
        assert_ne!(lowest, lower);
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
        let one = supercluster
            .with_filter(supercluster.filter(kept([0, 3])))
            .unwrap()
            .tile(0, 0, 0)
            .items;
        let other = supercluster
            .with_filter(supercluster.filter(kept([1, 2])))
            .unwrap()
            .tile(0, 0, 0)
            .items;

        assert_eq!(
            (one[0].x(), one[0].y(), one[0].count()),
            (other[0].x(), other[0].y(), 2)
        );
        assert_eq!(one[0].coordinates(), other[0].coordinates());
        assert_ne!(one[0], other[0]);
        assert_eq!(
            one[0],
            supercluster
                .with_filter(supercluster.filter(kept([0, 3])))
                .unwrap()
                .tile(0, 0, 0)
                .items[0]
        );
    }

    #[test]
    fn test_filtered_tile_calls_the_filter_once_per_point() {
        // Far from the antimeridian, so the tile shows every item once
        let supercluster = load(&[
            (10.0, 50.0, true),
            (10.001, 50.0, false),
            (10.002, 50.0, true),
            (-100.0, -30.0, true),
        ]);
        let calls = Cell::new(0);
        let items = supercluster
            .with_filter(supercluster.filter(|point| {
                calls.set(calls.get() + 1);
                point.keep
            }))
            .unwrap()
            .tile(0, 0, 0)
            .items;

        assert_eq!(items.len(), 2);
        assert_eq!(calls.get(), 4);

        // The expansion zoom reuses what the filter returned
        assert!(items.iter().any(|item| item.expansion_zoom().is_some()));
        assert_eq!(calls.get(), 4);
    }

    #[test]
    fn test_expansion_zoom_matches_filtered_cluster_expansion_zoom() {
        let supercluster = load(&create_spread_points(500));
        let filters: [fn(&TestPoint) -> bool; 2] = [|point| point.keep, |_| true];
        let mut clusters = 0;

        for view in [supercluster.view(), supercluster.with_cluster_max_zoom(3)] {
            for filter in filters {
                for z in 0..=5 {
                    for x in 0..1 << z {
                        for y in 0..1 << z {
                            for item in view
                                .with_filter(supercluster.filter(filter))
                                .unwrap()
                                .tile(z, x, y)
                                .items
                            {
                                let expected = match item.item() {
                                    Item::Cluster(cluster) => Some(
                                        view.with_filter(supercluster.filter(filter))
                                            .unwrap()
                                            .cluster_expansion_zoom(cluster.id())
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
        let Item::Cluster(cluster) = supercluster.tile(0, 0, 0).items[0].item() else {
            panic!("expected a cluster");
        };
        let leaves: Vec<&TestPoint> = supercluster.leaves(cluster.id()).unwrap().iter().collect();
        let coordinates_of = |points: Vec<&TestPoint>| -> Vec<Option<[f64; 2]>> {
            points.iter().map(|point| point.coordinates).collect()
        };

        // The points that pass, in the order of the leaves
        let filtered = supercluster
            .with_filter(supercluster.filter(|point| point.keep))
            .unwrap()
            .tile(0, 0, 0)
            .items;
        let kept = leaves.iter().copied().filter(|point| point.keep).collect();

        assert_eq!(filtered[0].count(), 2);
        assert_eq!(
            coordinates_of(filtered[0].leaves().collect()),
            coordinates_of(kept)
        );

        // When every point passes, the item holds all of them
        let unfiltered = supercluster
            .with_filter(supercluster.filter(|_| true))
            .unwrap()
            .tile(0, 0, 0)
            .items;

        assert_eq!(unfiltered[0].count(), 3);
        assert_eq!(
            coordinates_of(unfiltered[0].leaves().collect()),
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
            .with_filter(supercluster.filter(|point| point.keep))
            .unwrap()
            .tile(0, 0, 0)
            .items;
        let close =
            |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;

        for item in &items {
            match item.item() {
                // The cluster moved to the exact center of the two points left
                Item::Cluster(_) => assert!(close(item.coordinates(), [11.0, 50.0])),
                Item::Point { .. } => assert!(close(item.coordinates(), [-100.0, -30.0])),
            }
        }

        // A cluster that keeps all of its points keeps its own center
        let unfiltered = supercluster
            .with_filter(supercluster.filter(|_| true))
            .unwrap()
            .tile(0, 0, 0)
            .items;
        let Item::Cluster(cluster) = unfiltered[0].item() else {
            panic!("expected a cluster");
        };

        assert_eq!(unfiltered[0].coordinates(), cluster.coordinates());
    }

    /// Call `check` with the items of the tile at zoom level 0 for every kind of filter that keeps
    /// the points `keep` keeps: a `Filter`, a `CachedFilter`, cached filters combined per query and
    /// a cached combined filter, which must all give the same items. The items borrow the cached
    /// filters, so they cannot outlive this function.
    fn check_world_tile_every_way<P>(
        supercluster: &Supercluster<P>,
        keep: fn(&P) -> bool,
        check: impl FnOnce(&[Vec<TileItem<'_, P>>]),
    ) {
        let cached_filter = supercluster.cache_filter(keep);
        let all = supercluster.cache_filter(|_| true);
        let cached_combined = cached_filter.and(&all).cache();

        check(&[
            supercluster
                .with_filter(supercluster.filter(keep))
                .unwrap()
                .tile(0, 0, 0)
                .items,
            supercluster
                .with_filter(&cached_filter)
                .unwrap()
                .tile(0, 0, 0)
                .items,
            supercluster
                .with_filter(cached_filter.and(&all))
                .unwrap()
                .tile(0, 0, 0)
                .items,
            supercluster
                .with_filter(&cached_combined)
                .unwrap()
                .tile(0, 0, 0)
                .items,
        ]);
    }

    #[test]
    fn test_filtered_tile_centers_many_points_left_exactly() {
        // 200 points close together, which form one cluster on zoom level 0
        let grid: Vec<[f64; 2]> = (0..200)
            .map(|i| {
                [
                    10.0 + f64::from(i % 20) * 0.01,
                    50.0 + f64::from(i / 20) * 0.01,
                ]
            })
            .collect();
        let lng_lat = CoordinateSystem::LngLat;

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
                .map(|&point| lng_lat.normalize(point))
                .collect();
            let count = kept.len() as f64;
            let sum = kept
                .iter()
                .fold([0.0, 0.0], |sum, [x, y]| [sum[0] + x, sum[1] + y]);
            let center = lng_lat.denormalize([sum[0] / count, sum[1] / count]);

            let items = supercluster
                .with_filter(supercluster.filter(|point| point.keep))
                .unwrap()
                .tile(0, 0, 0)
                .items;
            let [lng, lat] = items[0].coordinates();

            assert_eq!(items.len(), 1);
            assert_eq!(items[0].count() as usize, kept.len());
            assert!((lng - center[0]).abs() < 1e-9 && (lat - center[1]).abs() < 1e-9);

            // Every way of filtering finds the same center, to the bit
            check_world_tile_every_way(
                &supercluster,
                |point| point.keep,
                |ways| {
                    for way in ways {
                        assert_eq!(way[0].coordinates(), items[0].coordinates());
                        assert_eq!((way[0].x(), way[0].y()), (items[0].x(), items[0].y()));
                    }
                },
            );
        }
    }

    #[test]
    fn test_filtered_tile_turns_a_cluster_with_one_point_left_into_it() {
        // Points at the corners of the map make the integers as coarse as in a real dataset
        let supercluster = load(&[
            (10.0137, 50.0071, true),
            (10.0237, 50.0171, false),
            (-179.0, -80.0, true),
            (179.0, 80.0, true),
        ]);
        let lng_lat = CoordinateSystem::LngLat;
        let point = lng_lat.denormalize(lng_lat.normalize([10.0137, 50.0071]));

        // Every way of filtering turns the cluster into the point, exactly where it is
        check_world_tile_every_way(
            &supercluster,
            |point| point.keep,
            |ways| {
                for items in ways {
                    let item = items
                        .iter()
                        .find(|item| matches!(item.item(), Item::Point { index: 0, .. }))
                        .unwrap();

                    assert!(items
                        .iter()
                        .all(|item| matches!(item.item(), Item::Point { .. })));
                    assert_eq!(item.count(), 1);
                    assert_eq!(item.coordinates(), point);
                    assert_eq!(item.leaves().count(), 1);
                    assert_eq!(item.expansion_zoom(), None);
                }
            },
        );
    }

    #[test]
    fn test_filtered_tile_puts_a_cluster_of_points_at_one_place_on_them() {
        // Three points at one place, two of which pass, and points at the corners of the map
        let supercluster = load(&[
            (10.0137, 50.0071, true),
            (10.0137, 50.0071, true),
            (10.0137, 50.0071, false),
            (-179.0, -80.0, true),
            (179.0, 80.0, true),
        ]);
        let lng_lat = CoordinateSystem::LngLat;
        let place = lng_lat.denormalize(lng_lat.normalize([10.0137, 50.0071]));

        // Every way of filtering puts it exactly on the place, not on the mean of the integers
        check_world_tile_every_way(
            &supercluster,
            |point| point.keep,
            |ways| {
                for items in ways {
                    let item = items
                        .iter()
                        .find(|item| matches!(item.item(), Item::Cluster(_)))
                        .unwrap();

                    assert_eq!(item.count(), 2);
                    assert_eq!(item.coordinates(), place);
                }
            },
        );
    }

    #[test]
    fn test_filtered_tile_centers_exactly_with_points_far_away() {
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
        let items = supercluster
            .with_filter(supercluster.filter(|point| point.2))
            .unwrap()
            .tile(0, 0, 0)
            .items;
        let [x, y] = items[0].coordinates();

        assert_eq!(items[0].count(), 2);
        assert!(
            (x - 100.305).abs() < 1e-9 && (y - 200.705).abs() < 1e-9,
            "{x}, {y}"
        );
    }

    #[test]
    fn test_filtered_tile_centers_a_cluster_with_points_past_the_range() {
        // A radius wider than the range makes clusters with points far past it
        let range = DataRange::new(0.0, 0.0, 100.0, 100.0);
        let options = SuperclusterOptions::builder()
            .coordinate_system(CoordinateSystem::Cartesian { range })
            .radius(3000.0)
            .build()
            .unwrap();
        let cases = [
            // One point that passes is past the range
            (
                vec![
                    (50.0, 50.0, true),
                    (52.0, 50.0, true),
                    (500.0, 50.0, true),
                    (60.0, 50.0, false),
                ],
                [602.0 / 3.0, 50.0],
            ),
            // Every point is past the range, on both axes
            (
                vec![
                    (500.0, 500.0, true),
                    (502.0, 500.0, true),
                    (504.0, 510.0, false),
                ],
                [501.0, 500.0],
            ),
            // The points that pass are at one place past the range
            (
                vec![
                    (500.3, 500.7, true),
                    (500.3, 500.7, true),
                    (504.0, 510.0, false),
                ],
                [500.3, 500.7],
            ),
        ];

        for (points, center) in cases {
            let supercluster =
                Supercluster::load(options.clone(), points, |point| Some([point.0, point.1]))
                    .unwrap();
            check_world_tile_every_way(
                &supercluster,
                |point| point.2,
                |ways| {
                    for items in ways {
                        let [x, y] = items[0].coordinates();

                        assert_eq!(items.len(), 1);
                        assert!(matches!(items[0].item(), Item::Cluster(_)));
                        assert!(
                            (x - center[0]).abs() < 1e-9 && (y - center[1]).abs() < 1e-9,
                            "{x}, {y}"
                        );

                        // Every way of filtering finds the same center, to the bit
                        assert_eq!(items[0].coordinates(), ways[0][0].coordinates());
                    }
                },
            );
        }
    }

    #[test]
    fn test_filtered_tile_centers_the_points_left_like_loading() {
        let supercluster = load(&[(10.0, 50.0, true), (10.0, 55.0, false), (10.0, 60.0, true)]);
        let items = supercluster
            .with_filter(supercluster.filter(|point| point.keep))
            .unwrap()
            .tile(0, 0, 0)
            .items;

        // Loading averages the normalized positions, where latitudes are not evenly spread, so the
        // center of the points at 50 and 60 is north of 55
        let lng_lat = CoordinateSystem::LngLat;
        let [south, north] = [[10.0, 50.0], [10.0, 60.0]].map(|point| lng_lat.normalize(point));
        let center =
            lng_lat.denormalize([(south[0] + north[0]) / 2.0, (south[1] + north[1]) / 2.0]);
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
            .with_filter(supercluster.filter(|point| point.keep))
            .unwrap()
            .tile(0, 0, 0)
            .items;

        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item.expansion_zoom().is_none()));
        assert!(items.iter().all(|item| item.leaves().count() == 1));
    }

    #[test]
    fn test_debug_of_a_filtered_tile_item_with_its_coordinates() {
        let supercluster = Supercluster::load(
            SuperclusterOptions::default(),
            vec![[10.0, 50.0], [10.001, 50.0], [-100.0, -30.0]],
            |point| Some(*point),
        )
        .unwrap();
        let items = supercluster
            .with_filter(supercluster.filter(|point| point[0] > 0.0))
            .unwrap()
            .tile(0, 0, 0)
            .items;

        // The output shows the item, not the whole supercluster it comes from
        let debug = format!("{:?}", items[0]);

        assert!(debug.starts_with("TileItem { x:"));
        assert!(debug.contains("count: 2, expansion_zoom: Some("));
        assert!(!debug.contains("trees"));
    }

    #[test]
    fn test_filtered_tile_with_a_large_cluster() {
        // 130 close points form one cluster, and only its 101st leaf fails the filter
        let points: Vec<[f64; 2]> = (0..130).map(|i| [10.0 + i as f64 * 1e-6, 50.0]).collect();
        let supercluster =
            Supercluster::load(SuperclusterOptions::default(), points, |point| Some(*point))
                .unwrap();
        let Item::Cluster(cluster) = supercluster.tile(0, 0, 0).items[0].item() else {
            panic!("expected a cluster");
        };
        let failing = supercluster.leaves(cluster.id()).unwrap()[100];
        let filter = |point: &[f64; 2]| *point != failing;

        let items = supercluster
            .with_filter(supercluster.filter(filter))
            .unwrap()
            .tile(0, 0, 0)
            .items;

        assert_eq!(items[0].count(), 129);
        assert!(items[0].leaves().all(|point| *point != failing));
        assert_eq!(
            items[0].expansion_zoom(),
            supercluster
                .with_filter(supercluster.filter(filter))
                .unwrap()
                .cluster_expansion_zoom(cluster.id())
                .ok()
        );
    }

    #[test]
    fn test_filtered_tile_without_points_left() {
        let points = [(10.0, 50.0, false), (12.0, 50.0, false)];

        assert!(filtered_tile(&points, WORLD_TILE).is_empty());
    }

    #[test]
    fn test_filtered_tile_with_cluster_max_zoom() {
        let supercluster = load(&[(10.0, 50.0, true), (12.0, 50.0, true)]);

        let clustered = supercluster
            .with_filter(supercluster.filter(|point| point.keep))
            .unwrap()
            .tile(1, 1, 0)
            .items;

        assert_eq!(clustered.len(), 1);
        assert_eq!(clustered[0].count(), 2);

        let unclustered = supercluster
            .with_cluster_max_zoom(0)
            .with_filter(supercluster.filter(|point| point.keep))
            .unwrap()
            .tile(1, 1, 0)
            .items;

        assert_eq!(unclustered.len(), 2);
        assert!(unclustered.iter().all(|tile_item| tile_item.count() == 1));
    }

    #[test]
    fn test_filtered_tile_for_empty_tile() {
        let points = [(10.0, 50.0, true), (12.0, 50.0, true)];

        assert!(filtered_tile(&points, (1, 0, 1)).is_empty());
    }

    #[test]
    fn test_filtered_tile_without_loaded_points() {
        assert!(filtered_tile(&[], WORLD_TILE).is_empty());
    }

    #[test]
    fn test_filtered_tile_with_a_point_left_out() {
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
            .with_filter(supercluster.filter(|point| point.keep))
            .unwrap()
            .tile(0, 0, 0)
            .items;
        let [lng, lat] = items[0].coordinates();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].count(), 1);
        assert!((lng - 10.0).abs() < 1e-9 && (lat - 50.0).abs() < 1e-9);

        let cached_filter = supercluster.cache_filter(|point| point.keep);

        assert_eq!(
            summarize(
                &supercluster
                    .with_filter(&cached_filter)
                    .unwrap()
                    .tile(0, 0, 0)
                    .items
            ),
            summarize(&items)
        );
    }

    #[test]
    fn test_cache_filter_calls_the_filter_once_per_clustered_point() {
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
        let mut calls = 0;

        // The filter can keep state, because it runs once per point, one after the other
        let cached_filter = supercluster.cache_filter(|point| {
            calls += 1;
            point.keep
        });

        // The point without coordinates is left out
        assert_eq!(calls, 2);
        assert_eq!(
            summarize(
                &supercluster
                    .with_filter(&cached_filter)
                    .unwrap()
                    .tile(0, 0, 0)
                    .items
            ),
            vec![(0, 1, [270, 174])]
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
    fn test_filtered_tile_with_a_cached_filter_matches_a_filter() {
        let supercluster = load(&create_spread_points(500));
        let filters: [fn(&TestPoint) -> bool; 3] = [|point| point.keep, |_| true, |_| false];
        let mut items = 0;

        for view in [supercluster.view(), supercluster.with_cluster_max_zoom(3)] {
            for filter in filters {
                let cached_filter = supercluster.cache_filter(filter);

                for z in 0..=5 {
                    for x in 0..1 << z {
                        for y in 0..1 << z {
                            let filtered = view
                                .with_filter(supercluster.filter(filter))
                                .unwrap()
                                .tile(z, x, y)
                                .items;
                            let selected = view
                                .with_filter(&cached_filter)
                                .unwrap()
                                .tile(z, x, y)
                                .items;

                            assert_eq!(summarize(&selected), summarize(&filtered));

                            for (selected, filtered) in selected.iter().zip(&filtered) {
                                assert_eq!(selected.coordinates(), filtered.coordinates());
                                assert_eq!(selected.expansion_zoom(), filtered.expansion_zoom());
                                assert!(same_points(selected.leaves(), filtered.leaves()));

                                if let Item::Cluster(cluster) = selected.item() {
                                    // The points that pass, in the order of the leaves
                                    let leaves = supercluster.leaves(cluster.id()).unwrap();

                                    assert!(same_points(
                                        selected.leaves(),
                                        leaves.iter().filter(|point| filter(point))
                                    ));

                                    assert_eq!(
                                        view.with_filter(&cached_filter).and_then(
                                            |view| view.cluster_expansion_zoom(cluster.id())
                                        ),
                                        view.with_filter(supercluster.filter(filter)).and_then(
                                            |view| view.cluster_expansion_zoom(cluster.id())
                                        )
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
    fn test_filtered_cluster_expansion_zoom_with_a_cached_filter_of_an_unknown_cluster() {
        let supercluster = load(&[(10.0, 50.0, true), (10.001, 50.0, true)]);
        let cached_filter = supercluster.cache_filter(|point| point.keep);

        assert_eq!(
            supercluster
                .with_filter(&cached_filter)
                .unwrap()
                .cluster_expansion_zoom(ClusterId::new(100000)),
            Err(SuperclusterError::ClusterNotFound)
        );
    }

    #[test]
    fn test_tile_matches_filtered_tile_accepting_every_point() {
        let supercluster = load(&create_spread_points(500));
        let every_point = supercluster.filter(|_| true);

        // A tile without a filter is a tile where every point passes, on every view
        for view in [supercluster.view(), supercluster.with_cluster_max_zoom(3)] {
            for z in 0..=5 {
                for x in 0..1 << z {
                    for y in 0..1 << z {
                        assert_eq!(
                            view.tile(z, x, y),
                            view.with_filter(&every_point).unwrap().tile(z, x, y),
                            "tile {z}/{x}/{y}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_tile_items_have_their_points_and_coordinates() {
        let supercluster = load(&create_spread_points(500));

        for tile_item in supercluster.tile(0, 0, 0) {
            match tile_item.item() {
                Item::Cluster(cluster) => {
                    assert_eq!(tile_item.count(), cluster.total_count());
                    assert_eq!(tile_item.coordinates(), cluster.coordinates());
                    assert!(tile_item
                        .leaves()
                        .eq(supercluster.leaves(cluster.id()).unwrap()));
                }
                Item::Point { point, .. } => {
                    assert_eq!(tile_item.count(), 1);
                    // The position goes through the projection and back, so it is off by a tiny bit
                    let [lng, lat] = tile_item.coordinates();
                    let [point_lng, point_lat] = point.coordinates.unwrap();

                    assert!((lng - point_lng).abs() < 1e-9 && (lat - point_lat).abs() < 1e-9);
                    assert!(tile_item.leaves().eq([point]));
                }
            }
        }
    }

    #[test]
    fn test_filtered_tile_with_a_filter_by_value_or_by_reference() {
        let supercluster = load(&create_spread_points(500));
        let keep = supercluster.filter(|point| point.keep);
        let by_reference = supercluster.with_filter(&keep).unwrap().tile(2, 1, 1);

        assert!(!by_reference.is_empty());
        assert_eq!(
            supercluster
                .with_filter(supercluster.filter(|point| point.keep))
                .unwrap()
                .tile(2, 1, 1),
            by_reference
        );
    }

    #[test]
    fn test_filtered_tile_outlives_its_filter() {
        let supercluster = load(&create_spread_points(500));
        // The items borrow the supercluster, not the filter, so the tile outlives the filter
        let tile = {
            let filter = supercluster.filter(|point| point.keep);

            supercluster.with_filter(&filter).unwrap().tile(0, 0, 0)
        };

        let cached = supercluster.cache_filter(|point| point.keep);

        assert_eq!(
            tile,
            supercluster.with_filter(&cached).unwrap().tile(0, 0, 0)
        );
    }

    #[test]
    fn test_with_filter_made_by_another_supercluster() {
        let small = load(&[(10.0, 50.0, true)]);
        let large = load(&create_spread_points(500));
        let by_small = large.with_filter(small.filter(|point| point.keep)).unwrap();
        let by_large = large.with_filter(large.filter(|point| point.keep)).unwrap();
        let cluster_id = large
            .tile(0, 0, 0)
            .items
            .iter()
            .find_map(|tile_item| match tile_item.item() {
                Item::Cluster(cluster) => Some(cluster.id()),
                Item::Point { .. } => None,
            })
            .unwrap();

        // A filter holds only its closure, which runs on the points of the query
        assert_eq!(by_small.tile(0, 0, 0), by_large.tile(0, 0, 0));
        assert_eq!(
            by_small.cluster_expansion_zoom(cluster_id),
            by_large.cluster_expansion_zoom(cluster_id)
        );
    }

    #[test]
    fn test_debug_of_a_filter() {
        let supercluster = load(&[(10.0, 50.0, true)]);

        // A closure has nothing to print, and the supercluster would print all its points
        assert_eq!(
            format!("{:?}", supercluster.filter(|point| point.keep)),
            "Filter { .. }"
        );
    }

    #[test]
    fn test_with_filter_with_a_cached_filter_of_another_supercluster() {
        let small = load(&[(10.0, 50.0, true)]);
        let large = load(&create_spread_points(500));
        let foreign = small.cache_filter(|point| point.keep);
        let own = large.cache_filter(|point| point.keep);
        let rejected = Some(SuperclusterError::ForeignCachedFilter);

        // On its own, or in a combined filter on either side, on every view
        assert_eq!(large.with_filter(&foreign).err(), rejected);
        assert_eq!(large.with_filter(own.and(&foreign)).err(), rejected);
        assert_eq!(large.with_filter(foreign.or(&own)).err(), rejected);
        assert_eq!(
            large.with_cluster_max_zoom(3).with_filter(&foreign).err(),
            rejected
        );
        assert!(large.with_filter(&own).is_ok());
    }

    #[test]
    fn test_clone_and_debug_of_a_filtered_view() {
        let supercluster = load(&create_spread_points(500));
        let cached_filter = supercluster.cache_filter(|point| point.keep);
        let filtered_view = supercluster
            .with_cluster_max_zoom(3)
            .with_filter(&cached_filter)
            .unwrap();
        let copy = filtered_view;

        // A view with a cached filter by reference is `Copy`, and a combined filter is `Clone`
        assert_eq!(filtered_view.tile(0, 0, 0), copy.tile(0, 0, 0));

        let combined = supercluster
            .with_filter(cached_filter.and(&cached_filter))
            .unwrap();

        assert_eq!(combined.clone().tile(0, 0, 0), combined.tile(0, 0, 0));

        // A closure has nothing to print, and the supercluster would print all its points
        assert_eq!(
            format!("{filtered_view:?}"),
            "FilteredView { cluster_max_zoom: 3, .. }"
        );
    }

    #[test]
    fn test_filtered_tile_with_a_cached_filter_without_loaded_points() {
        let supercluster = load(&[]);
        let cached_filter = supercluster.cache_filter(|point| point.keep);

        assert!(supercluster
            .with_filter(&cached_filter)
            .unwrap()
            .tile(0, 0, 0)
            .items
            .is_empty());
    }

    #[test]
    fn test_filtered_tile_with_a_cached_filter_of_one_point() {
        let supercluster = load(&[(10.0, 50.0, true)]);
        let kept = supercluster.cache_filter(|point| point.keep);
        let rejected = supercluster.cache_filter(|point| !point.keep);

        assert_eq!(
            summarize(&supercluster.with_filter(&kept).unwrap().tile(0, 0, 0).items),
            vec![(0, 1, [270, 174])]
        );
        assert!(supercluster
            .with_filter(&rejected)
            .unwrap()
            .tile(0, 0, 0)
            .items
            .is_empty());
    }

    #[test]
    fn test_filtered_tile_with_a_cached_filter_and_cartesian_coordinates() {
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
        let cached_filter = supercluster.cache_filter(|point| point.keep);
        let items = supercluster
            .with_filter(&cached_filter)
            .unwrap()
            .tile(0, 0, 0)
            .items;

        assert_eq!(
            summarize(&items),
            summarize(
                &supercluster
                    .with_filter(supercluster.filter(|point| point.keep))
                    .unwrap()
                    .tile(0, 0, 0)
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
