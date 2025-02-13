//! # KDBush module
//!
//! Contains the static spatial index for 2D points based on a flat KD-tree.

use serde::{Deserialize, Serialize};

/// Array of coordinates with longitude as first value and latitude as second one.
type Point = [f64; 2];

/// Static spatial index for 2D points based on a flat KD-tree.
/// The KD-tree is used to perform range and within queries on the points.
/// The index is built from a list of 2D points and can be queried to find points within a specified bounding box or radius.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KDBush {
    /// Node size for the KD-tree. Determines the number of points in a leaf node.
    /// Affects the performance of the index.
    pub node_size: usize,

    /// A list of point IDs used to reference points in the KD-tree.
    /// The IDs are used to retrieve the original points from the index.
    pub ids: Vec<usize>,

    /// A flat array containing the X and Y coordinates of all points in interleaved order.
    /// The coordinates are stored as [x1, y1, x2, y2, ..., xn, yn].
    pub coords: Vec<f64>,

    /// A list of 2D points represented as an array of [longitude, latitude] coordinates.
    /// The points are used to build the KD-tree index.
    pub points: Vec<Point>,

    /// A list of additional data associated with the points (e.g., properties).
    /// The data is stored in the same order as the points.
    pub data: Vec<f64>,
}

impl KDBush {
    /// Create a new KDBush index with the specified node size and the size hint for allocating memory.
    ///
    /// # Arguments
    ///
    /// - `size_hint`: An estimate of the number of points that will be added to the index.
    /// - `node_size`: The maximum number of points in a leaf node of the KD-tree.
    ///
    /// # Returns
    ///
    /// A new `KDBush` instance with the given configuration.
    pub fn new(size_hint: usize, node_size: usize) -> Self {
        KDBush {
            node_size,
            ids: Vec::with_capacity(size_hint),
            points: Vec::with_capacity(size_hint),
            coords: Vec::with_capacity(size_hint),
            data: Vec::with_capacity(size_hint),
        }
    }

    /// Add a 2D point to the KDBush index.
    ///
    /// # Arguments
    ///
    /// - `x`: The X-coordinate of the point (longitude).
    /// - `y`: The Y-coordinate of the point (latitude).
    pub fn add_point(&mut self, x: f64, y: f64) {
        self.points.push([x, y]);
    }

    /// Build the KD-tree index from the added points.
    ///
    /// This method constructs the KD-tree index based on the points added to the KDBush instance.
    /// After calling this method, the index will be ready for range and within queries.
    pub fn build_index(&mut self) {
        #[cfg(feature = "logger")]
        log::debug!("Building KDBush index with {} points", self.points.len());

        self.coords = vec![0.0; 2 * self.points.len()];

        for (i, point) in self.points.iter().enumerate() {
            self.ids.push(i);

            self.coords[i * 2] = point[0];
            self.coords[i * 2 + 1] = point[1];
        }

        self.sort(0, self.ids.len() - 1, 0);

        #[cfg(feature = "logger")]
        log::debug!("KDBush index built successfully");
    }

    /// Find all point indices within the specified bounding box defined by minimum and maximum coordinates.
    ///
    /// # Arguments
    ///
    /// - `min_x`: The minimum X-coordinate (longitude) of the bounding box.
    /// - `min_y`: The minimum Y-coordinate (latitude) of the bounding box.
    /// - `max_x`: The maximum X-coordinate (longitude) of the bounding box.
    /// - `max_y`: The maximum Y-coordinate (latitude) of the bounding box.
    ///
    /// # Returns
    ///
    /// A vector of point indices that fall within the specified bounding box.
    pub fn range(&self, min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Vec<usize> {
        #[cfg(feature = "logger")]
        log::debug!(
            "Finding points within range [{}, {}, {}, {}]",
            min_x,
            min_y,
            max_x,
            max_y
        );

        let mut stack = vec![(0, self.ids.len() - 1, 0)];
        let mut result: Vec<usize> = Vec::new();
        let mut x: f64;
        let mut y: f64;

        while let Some((axis, right, left)) = stack.pop() {
            if right - left <= self.node_size {
                for i in left..=right {
                    x = self.coords[i * 2];
                    y = self.coords[i * 2 + 1];

                    if x >= min_x && x <= max_x && y >= min_y && y <= max_y {
                        result.push(self.ids[i]);
                    }
                }
                continue;
            }

            let m = (left + right) >> 1;
            x = self.coords[m * 2];
            y = self.coords[m * 2 + 1];

            if x >= min_x && x <= max_x && y >= min_y && y <= max_y {
                result.push(self.ids[m]);
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

    /// Find all point indices within a given radius from a query point specified by coordinates.
    ///
    /// # Arguments
    ///
    /// - `qx`: The X-coordinate (longitude) of the query point.
    /// - `qy`: The Y-coordinate (latitude) of the query point.
    /// - `radius`: The radius around the query point.
    ///
    /// # Returns
    ///
    /// A vector of point indices that fall within the specified radius from the query point.
    pub fn within(&self, qx: f64, qy: f64, radius: f64) -> Vec<usize> {
        #[cfg(feature = "logger")]
        log::debug!(
            "Finding points within radius {} from query point [{}, {}]",
            radius,
            qx,
            qy
        );

        let mut stack = vec![(0, self.ids.len() - 1, 0)];
        let mut result: Vec<usize> = Vec::new();
        let r2 = radius * radius;

        while let Some((axis, right, left)) = stack.pop() {
            if right - left <= self.node_size {
                for i in left..=right {
                    let x = self.coords[i * 2];
                    let y = self.coords[i * 2 + 1];
                    let dst = KDBush::sq_dist(x, y, qx, qy);

                    if dst <= r2 {
                        result.push(self.ids[i]);
                    }
                }

                continue;
            }

            let m = (left + right) >> 1;
            let x = self.coords[m * 2];
            let y = self.coords[m * 2 + 1];

            if KDBush::sq_dist(x, y, qx, qy) <= r2 {
                result.push(self.ids[m]);
            }

            let next_axis = (axis + 1) % 2;

            if (axis == 0 && qx - radius <= x) || (axis != 0 && qy - radius <= y) {
                stack.push((next_axis, m - 1, left));
            }

            if (axis == 0 && qx + radius >= x) || (axis != 0 && qy + radius >= y) {
                stack.push((next_axis, right, m + 1));
            }
        }

        result
    }

    /// Sort points in the KD-tree along a specified axis.
    ///
    /// This method sorts the points in the KD-tree along a specified axis (0 for X or 1 for Y).
    ///
    /// # Arguments
    ///
    /// - `left`: The left index for the range of points to be sorted.
    /// - `right`: The right index for the range of points to be sorted.
    /// - `axis`: The axis along which the points should be sorted (0 for X or 1 for Y).
    fn sort(&mut self, left: usize, right: usize, axis: usize) {
        if right - left <= self.node_size {
            return;
        }

        let m = (left + right) >> 1;

        self.select(m, left, right, axis);

        self.sort(left, m - 1, 1 - axis);
        self.sort(m + 1, right, 1 - axis);
    }

    /// Select the k-th element along a specified axis within a range of indices.
    ///
    /// This method selects the k-th element along the specified axis (0 for X or 1 for Y)
    /// within the given range of indices.
    ///
    /// # Arguments
    ///
    /// - `k`: The index of the element to be selected.
    /// - `left`: The left index for the range of points.
    /// - `right`: The right index for the range of points.
    /// - `axis`: The axis along which the selection should be performed (0 for X or 1 for Y).
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
    ///
    /// # Arguments
    ///
    /// - `a`: The first value.
    /// - `b`: The second value.
    ///
    /// # Returns
    ///
    /// The maximum of the two values.
    fn get_max(a: usize, b: usize) -> usize {
        if a > b {
            a
        } else {
            b
        }
    }

    /// Return the minimum of two values.
    ///
    /// # Arguments
    ///
    /// - `a`: The first value.
    /// - `b`: The second value.
    ///
    /// # Returns
    ///
    /// The minimum of the two values.
    fn get_min(a: usize, b: usize) -> usize {
        if a < b {
            a
        } else {
            b
        }
    }

    /// Swap the elements at two specified indices in the KD-tree data structures.
    ///
    /// # Arguments
    ///
    /// - `i`: The index of the first element.
    /// - `j`: The index of the second element.
    fn swap_item(&mut self, i: usize, j: usize) {
        self.ids.swap(i, j);

        self.coords.swap(2 * i, 2 * j);
        self.coords.swap(2 * i + 1, 2 * j + 1);
    }

    /// Compute the square of the Euclidean distance between two points in a 2D space.
    ///
    /// # Arguments
    ///
    /// - `ax`: The x-coordinate of the first point.
    /// - `ay`: The y-coordinate of the first point.
    /// - `bx`: The x-coordinate of the second point.
    /// - `by`: The y-coordinate of the second point.
    ///
    /// # Returns
    ///
    /// The square of the Euclidean distance between the two points.
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

    pub const IDS: [usize; 100] = [
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
    fn test_build_index() {
        let mut index = KDBush::new(POINTS.len(), 10);

        for point in POINTS.iter() {
            index.add_point(point[0], point[1]);
        }

        index.build_index();

        assert_eq!(index.node_size, 10);
        assert!(!index.points.is_empty());

        let expected_ids: Vec<usize> = IDS.to_vec();
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
    fn test_within() {
        let mut index = KDBush::new(POINTS.len(), 10);

        for point in POINTS.iter() {
            index.add_point(point[0], point[1]);
        }

        index.build_index();

        let result = index.within(50.0, 50.0, 20.0);
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
