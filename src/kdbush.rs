//! # KDBush module
//!
//! Contains the static spatial index for 2D points based on a flat KD-tree.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Static spatial index for 2D points based on a flat KD-tree.
/// The KD-tree is used to perform range and within queries on the points.
/// The index is built from a list of 2D points and can be queried to find points within a specified bounding box or radius.
/// It holds up to `u32::MAX` points.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize))]
pub struct KDBush {
    /// Node size for the KD-tree. Determines the number of points in a leaf node.
    /// Affects the performance of the index.
    pub node_size: usize,

    /// A list of point IDs used to reference points in the KD-tree.
    /// The IDs are used to retrieve the original points from the index.
    pub ids: Vec<u32>,

    /// A flat array containing the X and Y coordinates of all points in interleaved order.
    /// The coordinates are stored as `[x1, y1, x2, y2, ..., xn, yn]`.
    pub coords: Vec<f64>,

    /// The values the supercluster keeps for each point or cluster, in the order the points were added.
    /// The KD-tree does not read them.
    pub data: Vec<f64>,
}

impl KDBush {
    /// Create an empty index with room for `size` points, whose leaf nodes hold up to `node_size`
    /// points.
    pub fn new(size: usize, node_size: usize) -> Self {
        KDBush {
            node_size,
            ids: Vec::new(),
            coords: Vec::with_capacity(2 * size),
            data: Vec::new(),
        }
    }

    /// Add a 2D point to the KDBush index.
    pub fn add_point(&mut self, x: f64, y: f64) {
        self.coords.push(x);
        self.coords.push(y);
    }

    /// Build the KD-tree index from the added points.
    ///
    /// This method constructs the KD-tree index based on the points added to the KDBush instance.
    /// After calling this method, the index will be ready for range and within queries.
    pub fn build_index(&mut self) {
        self.ids = (0..(self.coords.len() / 2) as u32).collect();

        if !self.ids.is_empty() {
            self.sort(0, self.ids.len() - 1, 0);
        }

        #[cfg(feature = "tracing")]
        tracing::trace!(points = self.ids.len(), "Built a KDBush index");
    }

    /// Find the indices of the points in a bounding box, edges included.
    pub fn range(&self, min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Vec<usize> {
        if self.ids.is_empty() {
            return Vec::new();
        }

        let mut stack = vec![(0, self.ids.len() - 1, 0)];
        let mut result: Vec<usize> = Vec::new();

        while let Some((axis, right, left)) = stack.pop() {
            if right - left <= self.node_size {
                for ([x, y], id) in self.leaf(left, right) {
                    if x >= min_x && x <= max_x && y >= min_y && y <= max_y {
                        result.push(id as usize);
                    }
                }
                continue;
            }

            let m = (left + right) >> 1;
            let x = self.coords[m * 2];
            let y = self.coords[m * 2 + 1];

            if x >= min_x && x <= max_x && y >= min_y && y <= max_y {
                result.push(self.ids[m] as usize);
            }

            let next_axis = (axis + 1) % 2;

            if (axis == 0 && min_x <= x) || (axis != 0 && min_y <= y) {
                stack.push((next_axis, m - 1, left));
            }

            if (axis == 0 && max_x >= x) || (axis != 0 && max_y >= y) {
                stack.push((next_axis, right, m + 1));
            }
        }

        result
    }

    /// Find the indices of the points within `radius` of the point `qx`, `qy` and write them into
    /// `result`. The search works in `stack`. The caller reuses both buffers between queries, which
    /// avoids allocating two vectors for every query, and the search clears them first.
    pub fn within_into(
        &self,
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
            if right - left <= self.node_size {
                for ([x, y], id) in self.leaf(left, right) {
                    if KDBush::sq_dist(x, y, qx, qy) <= r2 {
                        result.push(id as usize);
                    }
                }

                continue;
            }

            let m = (left + right) >> 1;
            let x = self.coords[m * 2];
            let y = self.coords[m * 2 + 1];

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

    /// Iterate over the coordinates and IDs of the points from `left` to `right`, both included,
    /// which make up a leaf of the KD-tree. Every query scans whole leaves, and slicing a leaf once
    /// saves checking the bounds of every point in it.
    fn leaf(&self, left: usize, right: usize) -> impl Iterator<Item = ([f64; 2], u32)> + '_ {
        let (coords, _) = self.coords[left * 2..=right * 2 + 1].as_chunks::<2>();

        coords
            .iter()
            .copied()
            .zip(self.ids[left..=right].iter().copied())
    }

    /// Sort the points from `left` to `right`, both included, into a KD-tree that splits first along
    /// `axis` (0 for X or 1 for Y).
    fn sort(&mut self, left: usize, right: usize, axis: usize) {
        if right - left <= self.node_size {
            return;
        }

        let m = (left + right) >> 1;

        self.select(m, left, right, axis);

        self.sort(left, m - 1, 1 - axis);
        self.sort(m + 1, right, 1 - axis);
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
                let new_left = KDBush::get_max(
                    left,
                    ((k as f64) - ((m as f64) * s) / (n as f64) + sd).floor() as usize,
                );
                let new_right = KDBush::get_min(
                    right,
                    ((k as f64) + (((n - m) as f64) * s) / (n as f64) + sd).floor() as usize,
                );

                self.select(k, new_left, new_right, axis);
            }

            let t = self.coords[2 * k + axis];
            let mut i = left;
            let mut j = right;

            self.swap_item(left, k);

            if self.coords[2 * right + axis] > t {
                self.swap_item(left, right);
            }

            while i < j {
                self.swap_item(i, j);

                i += 1;
                j -= 1;

                while self.coords[2 * i + axis] < t {
                    i += 1;
                }

                while self.coords[2 * j + axis] > t {
                    j -= 1;
                }
            }

            if self.coords[2 * left + axis] == t {
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

    /// Return the maximum of two values.
    fn get_max(a: usize, b: usize) -> usize {
        if a > b {
            a
        } else {
            b
        }
    }

    /// Return the minimum of two values.
    fn get_min(a: usize, b: usize) -> usize {
        if a < b {
            a
        } else {
            b
        }
    }

    /// Swap the elements at two specified indices in the KD-tree data structures.
    fn swap_item(&mut self, i: usize, j: usize) {
        self.ids.swap(i, j);

        self.coords.swap(2 * i, 2 * j);
        self.coords.swap(2 * i + 1, 2 * j + 1);
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

    pub const POINTS: [[f64; 2]; 100] = [
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

    pub const IDS: [u32; 100] = [
        97, 74, 95, 30, 77, 38, 76, 27, 80, 55, 72, 90, 88, 48, 43, 46, 65, 39, 62, 93, 9, 96, 47,
        8, 3, 12, 15, 14, 21, 41, 36, 40, 69, 56, 85, 78, 17, 71, 44, 19, 18, 13, 99, 24, 67, 33,
        37, 49, 54, 57, 98, 45, 23, 31, 66, 68, 0, 32, 5, 51, 75, 73, 84, 35, 81, 22, 61, 89, 1,
        11, 86, 52, 94, 16, 2, 6, 25, 92, 42, 20, 60, 58, 83, 79, 64, 10, 59, 53, 26, 87, 4, 63,
        50, 7, 28, 82, 70, 29, 34, 91,
    ];

    pub const COORDS: [f64; 200] = [
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

    #[test]
    fn test_build_index_without_points() {
        let mut index = KDBush::new(0, 10);

        index.build_index();

        assert!(index.ids.is_empty());
        assert!(index.coords.is_empty());
    }

    #[test]
    fn test_range_without_points() {
        let mut index = KDBush::new(0, 10);

        index.build_index();

        assert!(index.range(0.0, 0.0, 100.0, 100.0).is_empty());
    }

    #[test]
    fn test_within_into() {
        let mut index = KDBush::new(POINTS.len(), 10);

        for point in POINTS.iter() {
            index.add_point(point[0], point[1]);
        }

        index.build_index();

        // The buffers still hold the result of an earlier query, which the new query clears
        let mut stack = vec![(1, 2, 3)];
        let mut result = vec![999];

        index.within_into(50.0, 50.0, 20.0, &mut stack, &mut result);

        let expected_ids = vec![60, 6, 25, 92, 42, 20, 45, 3, 71, 44, 18, 96];

        assert_eq!(result, expected_ids);

        let r2 = 20.0 * 20.0;

        for &i in &result {
            let p = POINTS[i];

            if KDBush::sq_dist(p[0], p[1], 50.0, 50.0) > r2 {
                panic!();
            }
        }

        for (i, p) in POINTS.iter().enumerate() {
            if !result.contains(&i) && KDBush::sq_dist(p[0], p[1], 50.0, 50.0) <= r2 {
                panic!();
            }
        }

        // A second query with the same buffers gets the second query's result, not a mix of both
        index.within_into(10.0, 10.0, 5.0, &mut stack, &mut result);

        let r2 = 5.0 * 5.0;

        for &i in &result {
            let p = POINTS[i];

            if KDBush::sq_dist(p[0], p[1], 10.0, 10.0) > r2 {
                panic!();
            }
        }
    }

    #[test]
    fn test_within_without_points() {
        let mut index = KDBush::new(0, 10);

        index.build_index();

        let mut stack = vec![];
        let mut result = vec![];

        index.within_into(50.0, 50.0, 100.0, &mut stack, &mut result);

        assert!(result.is_empty());
    }

    #[test]
    fn test_build_index() {
        let mut index = KDBush::new(POINTS.len(), 10);

        for point in POINTS.iter() {
            index.add_point(point[0], point[1]);
        }

        index.build_index();

        assert_eq!(index.node_size, 10);
        assert_eq!(index.coords.len(), 2 * POINTS.len());

        let expected_ids: Vec<u32> = IDS.to_vec();
        let expected_coords: Vec<f64> = COORDS.to_vec();

        assert_eq!(index.ids, expected_ids);
        assert_eq!(index.coords, expected_coords)
    }

    #[test]
    fn test_range() {
        let mut index = KDBush::new(POINTS.len(), 10);

        for point in POINTS.iter() {
            index.add_point(point[0], point[1]);
        }

        index.build_index();

        let result = index.range(20.0, 30.0, 50.0, 70.0);
        let expected_ids = vec![
            60, 20, 45, 3, 17, 71, 44, 19, 18, 15, 69, 90, 62, 96, 47, 8, 77, 72,
        ];

        assert_eq!(result, expected_ids);

        for &i in &result {
            let p = POINTS[i];

            if p[0] < 20.0 || p[0] > 50.0 || p[1] < 30.0 || p[1] > 70.0 {
                panic!();
            }
        }

        for (i, p) in POINTS.iter().enumerate() {
            if !(result.contains(&i) || p[0] < 20.0 || p[0] > 50.0 || p[1] < 30.0 || p[1] > 70.0) {
                panic!();
            }
        }
    }

    #[test]
    fn test_sq_dist() {
        let result = KDBush::sq_dist(10.0, 10.0, 5.0, 5.0);

        assert_eq!(result, 50.0);
    }

    #[test]
    fn test_get_max_a_more_than_b() {
        let result = KDBush::get_max(10, 5);

        assert_eq!(result, 10);
    }

    #[test]
    fn test_get_max_b_more_than_a() {
        let result = KDBush::get_max(5, 10);

        assert_eq!(result, 10);
    }

    #[test]
    fn test_get_min_a_less_than_b() {
        let result = KDBush::get_min(5, 10);

        assert_eq!(result, 5);
    }

    #[test]
    fn test_get_min_b_less_than_a() {
        let result = KDBush::get_min(10, 5);

        assert_eq!(result, 5);
    }

    #[test]
    fn test_select_basic() {
        let mut kdbush = KDBush::new(10, 1);
        kdbush.coords = vec![2.0, 3.0, 1.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        kdbush.ids = vec![0, 1, 2, 3, 4];
        kdbush.select(2, 0, 4, 0);

        assert_eq!(kdbush.coords[4], 5.0);
    }

    #[test]
    fn test_select_large_range() {
        let mut kdbush = KDBush::new(10, 1);
        kdbush.coords = (0..4000).map(|x| x as f64).collect();
        kdbush.ids = (0..4000).collect();
        kdbush.select(1000, 0, 1999, 0);

        assert_eq!(kdbush.coords[2000], 2000.0);
    }

    #[test]
    fn test_select_axis_y() {
        let mut kdbush = KDBush::new(10, 1);
        kdbush.coords = vec![2.0, 3.0, 1.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        kdbush.ids = vec![0, 1, 2, 3, 4];
        kdbush.select(2, 0, 4, 1);

        assert_eq!(kdbush.coords[4], 5.0);
    }

    #[test]
    fn test_select_single_element() {
        let mut kdbush = KDBush::new(10, 1);
        kdbush.coords = vec![2.0];
        kdbush.ids = vec![0];
        kdbush.select(0, 0, 0, 0);

        assert_eq!(kdbush.coords[0], 2.0);
    }
}
