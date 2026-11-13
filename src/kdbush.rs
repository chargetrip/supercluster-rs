//! # KDBush module
//!
//! Contains the static spatial index for 2D points based on a flat KD-tree.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Static spatial index for 2D points based on a flat KD-tree.
/// The index is built once from a list of points, and finds the points in a bounding box or within
/// a radius. Every point has an ID, which is its index in the list the tree is built from, until
/// the owner of the tree replaces the IDs. It holds up to `u32::MAX` points. Its leaf nodes hold up
/// to a node size of points, which the owner keeps and passes to every search, the same one the
/// tree was built with.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize))]
pub struct KDBush {
    /// The ID of every point, in the order of the tree.
    pub ids: Vec<u32>,

    /// The X and Y coordinates of every point, in the order of the tree.
    pub coords: Vec<[f64; 2]>,
}

impl KDBush {
    /// Build the index over the points of `coords`, whose leaf nodes hold up to `node_size`
    /// points. The ID of a point is its index in `coords`.
    pub fn new(coords: Vec<[f64; 2]>, node_size: usize) -> Self {
        let mut tree = KDBush {
            ids: (0..coords.len() as u32).collect(),
            coords,
        };

        if !tree.ids.is_empty() {
            tree.sort(node_size, 0, tree.ids.len() - 1, 0);
        }

        #[cfg(feature = "tracing")]
        tracing::trace!(points = tree.ids.len(), "Built a KDBush index");

        tree
    }

    /// Call `visit` with the position in the tree of every point in a bounding box, edges
    /// included. The position indexes `ids` and `coords`. `node_size` is the one the tree was
    /// built with.
    pub fn range(
        &self,
        node_size: usize,
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
        visit: impl FnMut(usize),
    ) {
        self.range_where(
            node_size,
            [min_x, min_y, max_x, max_y],
            |_| true,
            |_| true,
            visit,
        );
    }

    /// Call `visit` with the position in the tree of every point in a bounding box, as the left,
    /// top, right and bottom edge, edges included, that `keep` accepts, but skip every node for
    /// which `enter` returns `false`, with the points under it. `enter` runs before a node is
    /// searched, so a skipped node costs one call. `enter` gets the number of a node: the root is
    /// 0, and the children of node `n` are `2n + 1` and `2n + 2`, as in `node_minimums`.
    ///
    /// `keep` gets the position of a point in the tree. The search stops reading a leaf node at its
    /// first point that `keep` rejects, so the points of every leaf must come in an order where
    /// the ones it accepts come first, as `sort_leaves_by_key` puts them. `node_size` is the one the
    /// tree was built with.
    pub fn range_where(
        &self,
        node_size: usize,
        [min_x, min_y, max_x, max_y]: [f64; 4],
        mut enter: impl FnMut(usize) -> bool,
        keep: impl Fn(usize) -> bool,
        mut visit: impl FnMut(usize),
    ) {
        if self.ids.is_empty() || !enter(0) {
            return;
        }

        // Each level of the tree halves the points, so the tree is less than 64 levels deep. The
        // search keeps at most one node per level and one more, so an array holds them without
        // allocating. A node is checked with `enter` before it goes on the stack.
        let mut stack = [(0, 0, 0, 0); 64];
        let mut length = 1;

        stack[0] = (0, self.ids.len() - 1, 0, 0);

        while length > 0 {
            length -= 1;

            let (axis, right, left, node) = stack[length];

            if right - left <= node_size {
                for at in left..=right {
                    // The points of a leaf that the search keeps come first
                    if !keep(at) {
                        break;
                    }

                    let [x, y] = self.coords[at];

                    if x >= min_x && x <= max_x && y >= min_y && y <= max_y {
                        visit(at);
                    }
                }
                continue;
            }

            let m = (left + right) >> 1;
            let [x, y] = self.coords[m];

            if x >= min_x && x <= max_x && y >= min_y && y <= max_y && keep(m) {
                visit(m);
            }

            let next_axis = (axis + 1) % 2;

            if ((axis == 0 && min_x <= x) || (axis != 0 && min_y <= y)) && enter(2 * node + 1) {
                stack[length] = (next_axis, m - 1, left, 2 * node + 1);
                length += 1;
            }

            if ((axis == 0 && max_x >= x) || (axis != 0 && max_y >= y)) && enter(2 * node + 2) {
                stack[length] = (next_axis, right, m + 1, 2 * node + 2);
                length += 1;
            }
        }
    }

    /// Sort the points of every leaf node by `keys`, lowest first, where `keys` holds a key for
    /// every point in the order of the tree and moves with them. A search with `range_where` that
    /// keeps the points up to a key then stops reading a leaf at its first point past that key.
    /// The order of the points within a leaf does not matter to a search, so the tree stays
    /// valid. `node_size` is the one the tree was built with.
    pub fn sort_leaves_by_key(&mut self, node_size: usize, keys: &mut [u8]) {
        if !self.ids.is_empty() {
            let mut leaf = Vec::with_capacity(node_size + 1);

            self.sort_leaves(node_size, keys, &mut leaf, 0, self.ids.len() - 1);
        }
    }

    /// Sort the points of the leaf nodes from `left` to `right`, both included, by `keys`. `leaf`
    /// is a buffer for the points of one leaf, which every leaf reuses.
    fn sort_leaves(
        &mut self,
        node_size: usize,
        keys: &mut [u8],
        leaf: &mut Vec<(u8, u32, [f64; 2])>,
        left: usize,
        right: usize,
    ) {
        if right - left > node_size {
            let m = (left + right) >> 1;

            self.sort_leaves(node_size, keys, leaf, left, m - 1);
            self.sort_leaves(node_size, keys, leaf, m + 1, right);

            return;
        }

        if keys[left..=right].is_sorted() {
            return;
        }

        leaf.clear();
        leaf.extend((left..=right).map(|at| (keys[at], self.ids[at], self.coords[at])));
        // Stable, so points with the same key keep their order
        leaf.sort_by_key(|&(key, _, _)| key);

        for (at, &(key, id, coords)) in (left..=right).zip(leaf.iter()) {
            keys[at] = key;
            self.ids[at] = id;
            self.coords[at] = coords;
        }
    }

    /// Find the lowest of `values`, which hold a value for every point in the order of the tree,
    /// under every node of the tree, by the number of the node that `range_where` gives. A number
    /// that no node has gets `u8::MAX`. `node_size` is the one the tree was built with.
    pub fn node_minimums(&self, node_size: usize, values: &[u8]) -> Vec<u8> {
        let mut minimums = vec![];

        if !self.ids.is_empty() {
            Self::fill_minimums(node_size, values, &mut minimums, 0, 0, self.ids.len() - 1);
        }

        minimums.shrink_to_fit();
        minimums
    }

    /// Find the lowest of `values` from `left` to `right`, both included, which node `node` holds,
    /// and write it and the ones of the nodes under it into `minimums`.
    fn fill_minimums(
        node_size: usize,
        values: &[u8],
        minimums: &mut Vec<u8>,
        node: usize,
        left: usize,
        right: usize,
    ) -> u8 {
        let minimum = if right - left <= node_size {
            values[left..=right]
                .iter()
                .copied()
                .min()
                .unwrap_or(u8::MAX)
        } else {
            let m = (left + right) >> 1;
            let below =
                Self::fill_minimums(node_size, values, minimums, 2 * node + 1, left, m - 1).min(
                    Self::fill_minimums(node_size, values, minimums, 2 * node + 2, m + 1, right),
                );

            values[m].min(below)
        };

        if minimums.len() <= node {
            minimums.resize(node + 1, u8::MAX);
        }

        minimums[node] = minimum;
        minimum
    }

    /// Find the IDs of the points within `radius` of the point `qx`, `qy` and write them into
    /// `result`. The search works in `stack`. The caller reuses both buffers between queries, which
    /// avoids allocating two vectors for every query, and the search clears them first.
    /// `node_size` is the one the tree was built with.
    pub fn within_into(
        &self,
        node_size: usize,
        qx: f64,
        qy: f64,
        radius: f64,
        stack: &mut Vec<(usize, usize, usize)>,
        result: &mut Vec<usize>,
    ) {
        stack.clear();
        result.clear();

        if self.ids.is_empty() {
            return;
        }

        stack.push((0, self.ids.len() - 1, 0));

        let r2 = radius * radius;

        while let Some((axis, right, left)) = stack.pop() {
            if right - left <= node_size {
                let leaf = self.coords[left..=right]
                    .iter()
                    .zip(&self.ids[left..=right]);

                for (&[x, y], &id) in leaf {
                    if KDBush::sq_dist(x, y, qx, qy) <= r2 {
                        result.push(id as usize);
                    }
                }

                continue;
            }

            let m = (left + right) >> 1;
            let [x, y] = self.coords[m];

            if KDBush::sq_dist(x, y, qx, qy) <= r2 {
                result.push(self.ids[m] as usize);
            }

            let next_axis = (axis + 1) % 2;

            if (axis == 0 && qx - radius <= x) || (axis != 0 && qy - radius <= y) {
                stack.push((next_axis, m - 1, left));
            }

            if (axis == 0 && qx + radius >= x) || (axis != 0 && qy + radius >= y) {
                stack.push((next_axis, right, m + 1));
            }
        }
    }

    /// Sort the points from `left` to `right`, both included, into a KD-tree that splits first along
    /// `axis` (0 for X or 1 for Y), with leaf nodes of up to `node_size` points.
    fn sort(&mut self, node_size: usize, left: usize, right: usize, axis: usize) {
        if right - left <= node_size {
            return;
        }

        let m = (left + right) >> 1;

        self.select(m, left, right, axis);

        self.sort(node_size, left, m - 1, 1 - axis);
        self.sort(node_size, m + 1, right, 1 - axis);
    }

    /// Move the points from `left` to `right`, both included, so the `k`-th one along `axis` (0 for X
    /// or 1 for Y) is in place, with smaller values before it and larger ones after it.
    fn select(&mut self, k: usize, left: usize, right: usize, axis: usize) {
        let mut left = left;
        let mut right = right;

        while right > left {
            if right - left > 600 {
                let n = right - left + 1;
                let m = k - left + 1;
                let z = (n as f64).ln();
                let s = 0.5 * ((2.0 * z) / 3.0).exp();
                let sds = if (m as f64) - (n as f64) / 2.0 < 0.0 {
                    -1.0
                } else {
                    1.0
                };
                let n_s = (n as f64) - s;
                let sd = 0.5 * ((z * s * n_s) / (n as f64)).sqrt() * sds;
                let new_left =
                    left.max(((k as f64) - ((m as f64) * s) / (n as f64) + sd).floor() as usize);
                let new_right = right
                    .min(((k as f64) + (((n - m) as f64) * s) / (n as f64) + sd).floor() as usize);

                self.select(k, new_left, new_right, axis);
            }

            let t = self.coords[k][axis];
            let mut i = left;
            let mut j = right;

            self.swap_item(left, k);

            if self.coords[right][axis] > t {
                self.swap_item(left, right);
            }

            while i < j {
                self.swap_item(i, j);

                i += 1;
                j -= 1;

                while self.coords[i][axis] < t {
                    i += 1;
                }

                while self.coords[j][axis] > t {
                    j -= 1;
                }
            }

            if self.coords[left][axis] == t {
                self.swap_item(left, j);
            } else {
                j += 1;
                self.swap_item(j, right);
            }

            if j <= k {
                left = j + 1;
            }
            if k <= j {
                right = j - 1;
            }
        }
    }

    /// Swap the points at two positions in the tree.
    fn swap_item(&mut self, i: usize, j: usize) {
        self.ids.swap(i, j);
        self.coords.swap(i, j);
    }

    /// Compute the square of the Euclidean distance between two points in a 2D space.
    fn sq_dist(ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
        let dx = ax - bx;
        let dy = ay - by;

        dx * dx + dy * dy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POINTS: [[f64; 2]; 100] = [
        [54.0, 1.0],
        [97.0, 21.0],
        [65.0, 35.0],
        [33.0, 54.0],
        [95.0, 39.0],
        [54.0, 3.0],
        [53.0, 54.0],
        [84.0, 72.0],
        [33.0, 34.0],
        [43.0, 15.0],
        [52.0, 83.0],
        [81.0, 23.0],
        [1.0, 61.0],
        [38.0, 74.0],
        [11.0, 91.0],
        [24.0, 56.0],
        [90.0, 31.0],
        [25.0, 57.0],
        [46.0, 61.0],
        [29.0, 69.0],
        [49.0, 60.0],
        [4.0, 98.0],
        [71.0, 15.0],
        [60.0, 25.0],
        [38.0, 84.0],
        [52.0, 38.0],
        [94.0, 51.0],
        [13.0, 25.0],
        [77.0, 73.0],
        [88.0, 87.0],
        [6.0, 27.0],
        [58.0, 22.0],
        [53.0, 28.0],
        [27.0, 91.0],
        [96.0, 98.0],
        [93.0, 14.0],
        [22.0, 93.0],
        [45.0, 94.0],
        [18.0, 28.0],
        [35.0, 15.0],
        [19.0, 81.0],
        [20.0, 81.0],
        [67.0, 53.0],
        [43.0, 3.0],
        [47.0, 66.0],
        [48.0, 34.0],
        [46.0, 12.0],
        [32.0, 38.0],
        [43.0, 12.0],
        [39.0, 94.0],
        [88.0, 62.0],
        [66.0, 14.0],
        [84.0, 30.0],
        [72.0, 81.0],
        [41.0, 92.0],
        [26.0, 4.0],
        [6.0, 76.0],
        [47.0, 21.0],
        [57.0, 70.0],
        [71.0, 82.0],
        [50.0, 68.0],
        [96.0, 18.0],
        [40.0, 31.0],
        [78.0, 53.0],
        [71.0, 90.0],
        [32.0, 14.0],
        [55.0, 6.0],
        [32.0, 88.0],
        [62.0, 32.0],
        [21.0, 67.0],
        [73.0, 81.0],
        [44.0, 64.0],
        [29.0, 50.0],
        [70.0, 5.0],
        [6.0, 22.0],
        [68.0, 3.0],
        [11.0, 23.0],
        [20.0, 42.0],
        [21.0, 73.0],
        [63.0, 86.0],
        [9.0, 40.0],
        [99.0, 2.0],
        [99.0, 76.0],
        [56.0, 77.0],
        [83.0, 6.0],
        [21.0, 72.0],
        [78.0, 30.0],
        [75.0, 53.0],
        [41.0, 11.0],
        [95.0, 20.0],
        [30.0, 38.0],
        [96.0, 82.0],
        [65.0, 48.0],
        [33.0, 18.0],
        [87.0, 28.0],
        [10.0, 10.0],
        [40.0, 34.0],
        [10.0, 20.0],
        [47.0, 29.0],
        [46.0, 78.0],
    ];

    const IDS: [u32; 100] = [
        97, 74, 95, 30, 77, 38, 76, 27, 80, 55, 72, 90, 88, 48, 43, 46, 65, 39, 62, 93, 9, 96, 47,
        8, 3, 12, 15, 14, 21, 41, 36, 40, 69, 56, 85, 78, 17, 71, 44, 19, 18, 13, 99, 24, 67, 33,
        37, 49, 54, 57, 98, 45, 23, 31, 66, 68, 0, 32, 5, 51, 75, 73, 84, 35, 81, 22, 61, 89, 1,
        11, 86, 52, 94, 16, 2, 6, 25, 92, 42, 20, 60, 58, 83, 79, 64, 10, 59, 53, 26, 87, 4, 63,
        50, 7, 28, 82, 70, 29, 34, 91,
    ];

    const COORDS: [f64; 200] = [
        10.0, 20.0, 6.0, 22.0, 10.0, 10.0, 6.0, 27.0, 20.0, 42.0, 18.0, 28.0, 11.0, 23.0, 13.0,
        25.0, 9.0, 40.0, 26.0, 4.0, 29.0, 50.0, 30.0, 38.0, 41.0, 11.0, 43.0, 12.0, 43.0, 3.0,
        46.0, 12.0, 32.0, 14.0, 35.0, 15.0, 40.0, 31.0, 33.0, 18.0, 43.0, 15.0, 40.0, 34.0, 32.0,
        38.0, 33.0, 34.0, 33.0, 54.0, 1.0, 61.0, 24.0, 56.0, 11.0, 91.0, 4.0, 98.0, 20.0, 81.0,
        22.0, 93.0, 19.0, 81.0, 21.0, 67.0, 6.0, 76.0, 21.0, 72.0, 21.0, 73.0, 25.0, 57.0, 44.0,
        64.0, 47.0, 66.0, 29.0, 69.0, 46.0, 61.0, 38.0, 74.0, 46.0, 78.0, 38.0, 84.0, 32.0, 88.0,
        27.0, 91.0, 45.0, 94.0, 39.0, 94.0, 41.0, 92.0, 47.0, 21.0, 47.0, 29.0, 48.0, 34.0, 60.0,
        25.0, 58.0, 22.0, 55.0, 6.0, 62.0, 32.0, 54.0, 1.0, 53.0, 28.0, 54.0, 3.0, 66.0, 14.0,
        68.0, 3.0, 70.0, 5.0, 83.0, 6.0, 93.0, 14.0, 99.0, 2.0, 71.0, 15.0, 96.0, 18.0, 95.0, 20.0,
        97.0, 21.0, 81.0, 23.0, 78.0, 30.0, 84.0, 30.0, 87.0, 28.0, 90.0, 31.0, 65.0, 35.0, 53.0,
        54.0, 52.0, 38.0, 65.0, 48.0, 67.0, 53.0, 49.0, 60.0, 50.0, 68.0, 57.0, 70.0, 56.0, 77.0,
        63.0, 86.0, 71.0, 90.0, 52.0, 83.0, 71.0, 82.0, 72.0, 81.0, 94.0, 51.0, 75.0, 53.0, 95.0,
        39.0, 78.0, 53.0, 88.0, 62.0, 84.0, 72.0, 77.0, 73.0, 99.0, 76.0, 73.0, 81.0, 88.0, 87.0,
        96.0, 98.0, 96.0, 82.0,
    ];

    /// The node size of the trees of the tests.
    const NODE_SIZE: usize = 10;

    /// Build the index over the test points, with leaf nodes of `NODE_SIZE` points.
    fn build_index() -> KDBush {
        KDBush::new(POINTS.to_vec(), NODE_SIZE)
    }

    /// Build a tree from points that are not sorted yet, to test the sorting on its own.
    fn unsorted(coords: Vec<[f64; 2]>) -> KDBush {
        KDBush {
            ids: (0..coords.len() as u32).collect(),
            coords,
        }
    }

    /// Get the IDs of the points in a bounding box, in the order the tree finds them.
    fn range_ids(index: &KDBush, min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Vec<usize> {
        let mut ids = vec![];

        index.range(NODE_SIZE, min_x, min_y, max_x, max_y, |position| {
            ids.push(index.ids[position] as usize)
        });

        ids
    }

    #[test]
    fn test_new_without_points() {
        let index = KDBush::new(vec![], NODE_SIZE);

        assert!(index.ids.is_empty());
        assert!(index.coords.is_empty());
    }

    #[test]
    fn test_range_without_points() {
        let index = KDBush::new(vec![], NODE_SIZE);

        assert!(range_ids(&index, 0.0, 0.0, 100.0, 100.0).is_empty());
    }

    #[test]
    fn test_node_minimums_without_points() {
        let index = KDBush::new(vec![], NODE_SIZE);

        assert!(index.node_minimums(NODE_SIZE, &[]).is_empty());
    }

    #[test]
    fn test_range_where_skips_the_points_of_the_nodes_it_does_not_enter() {
        let mut index = build_index();

        // The value of a point is its X, which is below 100, and the points it keeps come first
        let mut values: Vec<u8> = index.coords.iter().map(|&[x, _]| x as u8).collect();

        index.sort_leaves_by_key(NODE_SIZE, &mut values);

        let minimums = index.node_minimums(NODE_SIZE, &values);

        assert_eq!(minimums[0], *values.iter().min().unwrap());

        let mut entered_with = vec![];

        for limit in [0, 25, 50, 75, 100] {
            let mut entered = 0;
            let mut ids = vec![];

            index.range_where(
                NODE_SIZE,
                [20.0, 30.0, 80.0, 70.0],
                |node| {
                    entered += 1;
                    minimums[node] <= limit
                },
                |at| values[at] <= limit,
                |at| ids.push(index.ids[at] as usize),
            );

            // The same points as a search of every node, with a value up to the limit
            let mut expected: Vec<usize> = range_ids(&index, 20.0, 30.0, 80.0, 70.0)
                .into_iter()
                .filter(|&id| POINTS[id][0] as u8 <= limit)
                .collect();

            ids.sort_unstable();
            expected.sort_unstable();
            assert_eq!(ids, expected, "limit {limit}");

            entered_with.push(entered);
        }

        // A node whose points all have a higher value is skipped with the nodes under it
        assert!(entered_with[0] < entered_with[4]);
    }

    #[test]
    fn test_sort_leaves_by_key_keeps_what_a_search_finds() {
        let unsorted = build_index();
        let mut index = build_index();
        let mut keys: Vec<u8> = index.coords.iter().map(|&[_, y]| 100 - y as u8).collect();

        index.sort_leaves_by_key(NODE_SIZE, &mut keys);

        // Every key moved with its point
        for (at, &id) in index.ids.iter().enumerate() {
            assert_eq!(keys[at], 100 - POINTS[id as usize][1] as u8);
        }

        // The order within a leaf does not matter to a search
        for [min_x, min_y, max_x, max_y] in [[20.0, 30.0, 80.0, 70.0], [0.0, 0.0, 100.0, 100.0]] {
            let mut found = range_ids(&index, min_x, min_y, max_x, max_y);
            let mut expected = range_ids(&unsorted, min_x, min_y, max_x, max_y);

            found.sort_unstable();
            expected.sort_unstable();
            assert_eq!(found, expected);
        }
    }

    #[test]
    fn test_within_into() {
        let index = build_index();

        // The buffers still hold the result of an earlier query, which the new query clears
        let mut stack = vec![(1, 2, 3)];
        let mut result = vec![999];

        index.within_into(NODE_SIZE, 50.0, 50.0, 20.0, &mut stack, &mut result);

        let expected_ids = vec![60, 6, 25, 92, 42, 20, 45, 3, 71, 44, 18, 96];

        assert_eq!(result, expected_ids);

        let r2 = 20.0 * 20.0;

        for &i in &result {
            let p = POINTS[i];

            assert!(KDBush::sq_dist(p[0], p[1], 50.0, 50.0) <= r2);
        }

        for (i, p) in POINTS.iter().enumerate() {
            assert!(result.contains(&i) || KDBush::sq_dist(p[0], p[1], 50.0, 50.0) > r2);
        }

        // A second query with the same buffers gets the second query's result, not a mix of both
        index.within_into(NODE_SIZE, 10.0, 10.0, 5.0, &mut stack, &mut result);

        let r2 = 5.0 * 5.0;

        for &i in &result {
            let p = POINTS[i];

            assert!(KDBush::sq_dist(p[0], p[1], 10.0, 10.0) <= r2);
        }
    }

    #[test]
    fn test_within_without_points() {
        let index = KDBush::new(vec![], NODE_SIZE);

        let mut stack = vec![];
        let mut result = vec![];

        index.within_into(NODE_SIZE, 50.0, 50.0, 100.0, &mut stack, &mut result);

        assert!(result.is_empty());
    }

    #[test]
    fn test_new() {
        let index = build_index();

        assert_eq!(index.ids, IDS.to_vec());
        assert_eq!(index.coords.as_flattened(), COORDS);
    }

    #[test]
    fn test_range() {
        let index = build_index();

        let result = range_ids(&index, 20.0, 30.0, 50.0, 70.0);
        let expected_ids = vec![
            60, 20, 45, 3, 17, 71, 44, 19, 18, 15, 69, 90, 62, 96, 47, 8, 77, 72,
        ];

        assert_eq!(result, expected_ids);

        for &i in &result {
            let p = POINTS[i];

            assert!((20.0..=50.0).contains(&p[0]) && (30.0..=70.0).contains(&p[1]));
        }

        for (i, p) in POINTS.iter().enumerate() {
            assert!(
                result.contains(&i)
                    || !((20.0..=50.0).contains(&p[0]) && (30.0..=70.0).contains(&p[1]))
            );
        }
    }

    #[test]
    fn test_range_visits_positions_in_the_tree() {
        let index = build_index();

        index.range(NODE_SIZE, 20.0, 30.0, 50.0, 70.0, |position| {
            let [x, y] = index.coords[position];

            assert_eq!(POINTS[index.ids[position] as usize], [x, y]);
        });
    }

    #[test]
    fn test_sq_dist() {
        let result = KDBush::sq_dist(10.0, 10.0, 5.0, 5.0);

        assert_eq!(result, 50.0);
    }

    #[test]
    fn test_select_basic() {
        let mut kdbush = unsorted(vec![
            [2.0, 3.0],
            [1.0, 4.0],
            [5.0, 6.0],
            [7.0, 8.0],
            [9.0, 10.0],
        ]);
        kdbush.select(2, 0, 4, 0);

        assert_eq!(kdbush.coords[2][0], 5.0);
    }

    #[test]
    fn test_select_large_range() {
        let mut kdbush = unsorted(
            (0..2000)
                .map(|i| [(2 * i) as f64, (2 * i + 1) as f64])
                .collect(),
        );
        kdbush.select(1000, 0, 1999, 0);

        assert_eq!(kdbush.coords[1000][0], 2000.0);
    }

    #[test]
    fn test_select_axis_y() {
        let mut kdbush = unsorted(vec![
            [2.0, 3.0],
            [1.0, 4.0],
            [5.0, 6.0],
            [7.0, 8.0],
            [9.0, 10.0],
        ]);
        kdbush.select(2, 0, 4, 1);

        assert_eq!(kdbush.coords[2][1], 6.0);
    }

    #[test]
    fn test_select_single_element() {
        let mut kdbush = unsorted(vec![[2.0, 3.0]]);
        kdbush.select(0, 0, 0, 0);

        assert_eq!(kdbush.coords[0], [2.0, 3.0]);
    }
}
