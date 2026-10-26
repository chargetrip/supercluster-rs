//! # Selection module
//!
//! Contains the selection of the points that pass a filter, which `Supercluster::select` finds once
//! so many queries can reuse it, and the bit sets the filtered queries share.

use std::{
    fmt::{self, Debug, Formatter},
    iter,
    ops::Range,
};

/// The points of a supercluster that pass a filter, which `Supercluster::select` finds once, so
/// many queries can reuse them.
///
/// It holds one bit per point, and a count per 64 points, so it counts the points of a cluster
/// that pass with two lookups, whatever the size of the cluster. The cost of `tile_selected` grows
/// with the items of the tile, not with the points in them. A million points take about 190 KB.
///
/// A selection belongs to the supercluster that made it. With another supercluster the queries
/// do not panic, but their results mean nothing. See `Supercluster::select` for an example.
#[derive(Clone)]
pub struct Selection {
    /// One bit per point, in the order of `Supercluster::points`, set when the point passes.
    words: Vec<u64>,

    /// The number of bits set before every word, followed by the number in all of them.
    ranks: Vec<u32>,
}

impl Selection {
    /// Create a selection from one bit per point, set when the point passes.
    pub(crate) fn new(words: Vec<u64>) -> Self {
        let mut ranks = Vec::with_capacity(words.len() + 1);
        let mut rank = 0;

        ranks.push(rank);

        for word in &words {
            rank += word.count_ones();
            ranks.push(rank);
        }

        Selection { words, ranks }
    }

    /// Get a selection of the points that pass both this selection and `other`.
    ///
    /// Combining works on the bits instead of the points, so it costs much less than running
    /// `select` with both filters. Both selections must come from the same supercluster: with a
    /// selection of another one, the result does not panic, but means nothing.
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
    ///
    /// // Select once per attribute, then combine them for each request
    /// let fast = supercluster.select(|station| station.2 >= 50);
    /// let available = supercluster.select(|station| station.3);
    ///
    /// // The items borrow the selection, so keep it while they are used
    /// let fast_and_available = fast.and(&available);
    /// let items = supercluster.tile_selected(0, 0, 0, &fast_and_available);
    ///
    /// assert_eq!(items[0].count(), 1);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn and(&self, other: &Selection) -> Selection {
        self.combine(other, |word, other| word & other)
    }

    /// Get a selection of the points that pass this selection, `other` or both, like `and`.
    pub fn or(&self, other: &Selection) -> Selection {
        self.combine(other, |word, other| word | other)
    }

    /// Get a selection of the points that pass this selection but not `other`, like `and`.
    pub fn and_not(&self, other: &Selection) -> Selection {
        self.combine(other, |word, other| word & !other)
    }

    /// Combine the bits of this selection with the ones of `other`, a word at a time. The result
    /// has as many words as this selection, and a word that `other` lacks counts as no bit set, so
    /// a selection of another supercluster does not make it panic.
    fn combine(&self, other: &Selection, operation: impl Fn(u64, u64) -> u64) -> Selection {
        let others = other.words.iter().chain(iter::repeat(&0));

        Selection::new(
            self.words
                .iter()
                .zip(others)
                .map(|(&word, &other)| operation(word, other))
                .collect(),
        )
    }

    /// Get one bit per point, set when the point passes.
    pub(crate) fn words(&self) -> &[u64] {
        &self.words
    }

    /// Count the points at the positions of `range` that pass.
    pub(crate) fn count(&self, range: Range<usize>) -> u32 {
        self.rank(range.end) - self.rank(range.start)
    }

    /// Count the points before `position` that pass.
    fn rank(&self, position: usize) -> u32 {
        let word = position / 64;

        match self.words.get(word) {
            Some(bits) => self.ranks[word] + (bits & ((1 << (position % 64)) - 1)).count_ones(),
            None => self.ranks[self.words.len()],
        }
    }
}

// Written by hand, because a derive would print every bit.
impl Debug for Selection {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Selection")
            .field("selected", &self.ranks[self.words.len()])
            .finish_non_exhaustive()
    }
}

/// Run the filter on every point and yield the results as bits, 64 points per word. It does not
/// branch on what the filter returns, because the processor cannot predict a filter that keeps
/// points here and there, and a wrong guess per point costs more than the filter.
pub(crate) fn filter_bits<'p, P>(
    points: &'p [P],
    filter: impl Fn(&P) -> bool + 'p,
) -> impl Iterator<Item = u64> + 'p {
    points.chunks(64).map(move |chunk| {
        chunk.iter().enumerate().fold(0, |word, (bit, point)| {
            word | (u64::from(filter(point)) << bit)
        })
    })
}

/// Iterate over the positions of a range whose bit is set, a word at a time, so a long run of
/// bits that are not set costs one step per 64 of them. The bit of position `p` is bit
/// `p - offset` of the words, and without words every position is set.
pub(crate) struct SetPositions<'b> {
    /// The bits, or `None` when every position is set.
    words: Option<&'b [u64]>,

    /// The position of the first bit of the words.
    offset: usize,

    /// The next position to look at.
    next: usize,

    /// The end of the range, which is not part of it.
    end: usize,
}

impl<'b> SetPositions<'b> {
    /// Iterate over the positions of `range` whose bit is set in `words`, where the first bit of
    /// the words is the one of position `offset`, which is not after the start of the range.
    pub(crate) fn new(words: Option<&'b [u64]>, offset: usize, range: Range<usize>) -> Self {
        SetPositions {
            words,
            offset,
            next: range.start,
            end: range.end,
        }
    }
}

impl Iterator for SetPositions<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        while self.next < self.end {
            let Some(words) = self.words else {
                self.next += 1;

                return Some(self.next - 1);
            };

            let bit = self.next - self.offset;
            let word = words.get(bit / 64).map_or(0, |word| word >> (bit % 64));

            if word == 0 {
                // No bit is set in the rest of the word
                self.next += 64 - bit % 64;

                continue;
            }

            let position = self.next + word.trailing_zeros() as usize;

            if position >= self.end {
                break;
            }

            self.next = position + 1;

            return Some(position);
        }

        self.next = self.end;

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{Supercluster, SuperclusterOptions};

    /// Load points spread over the world with a fixed formula, so every run uses the same points.
    fn load_spread_points(count: usize) -> Supercluster<[f64; 2]> {
        let points = (0..count)
            .map(|i| {
                let lng = (i as f64 * 0.618_033_988_749_895) % 1.0;
                let lat = (i as f64 * 0.754_877_666_246_693) % 1.0;

                [-180.0 + 360.0 * lng, -80.0 + 160.0 * lat]
            })
            .collect();

        Supercluster::load(SuperclusterOptions::default(), points, |point| Some(*point)).unwrap()
    }

    /// Keeps about one point in three, here and there.
    fn every_third(point: &[f64; 2]) -> bool {
        (point[0] * 1e6) as i64 % 3 == 0
    }

    /// Keeps the points of the northern half.
    fn north(point: &[f64; 2]) -> bool {
        point[1] > 0.0
    }

    /// Check that two selections give the same results: the same number of points, and the same
    /// items, counts, positions and expansion zooms in every tile up to zoom level 4.
    fn assert_same_results(supercluster: &Supercluster<[f64; 2]>, a: &Selection, b: &Selection) {
        assert_eq!(format!("{a:?}"), format!("{b:?}"));

        for z in 0..=4 {
            for x in 0..1 << z {
                for y in 0..1 << z {
                    let summarize = |selection| -> Vec<_> {
                        supercluster
                            .tile_selected(z, x, y, selection)
                            .iter()
                            .map(|item| (item.x, item.y, item.count(), item.expansion_zoom()))
                            .collect()
                    };

                    assert_eq!(summarize(a), summarize(b), "tile {z}/{x}/{y}");
                }
            }
        }
    }

    #[test]
    fn test_and_matches_select_with_both_filters() {
        let supercluster = load_spread_points(500);
        let combined = supercluster
            .select(every_third)
            .and(&supercluster.select(north));
        let selected = supercluster.select(|point| every_third(point) && north(point));

        assert_same_results(&supercluster, &combined, &selected);
    }

    #[test]
    fn test_or_matches_select_with_either_filter() {
        let supercluster = load_spread_points(500);
        let combined = supercluster
            .select(every_third)
            .or(&supercluster.select(north));
        let selected = supercluster.select(|point| every_third(point) || north(point));

        assert_same_results(&supercluster, &combined, &selected);
    }

    #[test]
    fn test_and_not_matches_select_with_the_first_filter_only() {
        let supercluster = load_spread_points(500);
        let combined = supercluster
            .select(every_third)
            .and_not(&supercluster.select(north));
        let selected = supercluster.select(|point| every_third(point) && !north(point));

        assert_same_results(&supercluster, &combined, &selected);
    }

    #[test]
    fn test_and_with_a_selection_of_another_supercluster() {
        let small = load_spread_points(10);
        let large = load_spread_points(500);
        let small_selection = small.select(|_| true);
        let large_selection = large.select(|_| true);

        // The words that the smaller selection lacks count as no bit set
        assert_eq!(
            format!("{:?}", large_selection.and(&small_selection)),
            "Selection { selected: 10, .. }"
        );
        assert_eq!(
            format!("{:?}", large_selection.and_not(&small_selection)),
            "Selection { selected: 490, .. }"
        );

        // The result means nothing, but combining does not panic
        small_selection.or(&large_selection);
    }

    #[test]
    fn test_and_without_points() {
        let supercluster = load_spread_points(0);
        let selection = supercluster.select(|_| true);

        assert_eq!(
            format!("{:?}", selection.and(&selection)),
            "Selection { selected: 0, .. }"
        );
    }

    #[test]
    fn test_debug_of_a_selection() {
        // 130 points, so the bits spread over three words
        let points: Vec<[f64; 2]> = (0..130).map(|i| [f64::from(i), 0.0]).collect();
        let supercluster =
            Supercluster::load(SuperclusterOptions::default(), points, |point| Some(*point))
                .unwrap();
        let selection = supercluster.select(|point| point[0] % 3.0 == 0.0);

        // The output counts the points that pass instead of printing every bit
        assert_eq!(format!("{selection:?}"), "Selection { selected: 44, .. }");
    }
}
