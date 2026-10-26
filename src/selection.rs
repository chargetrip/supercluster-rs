//! # Selection module
//!
//! Contains the selection of the points that pass a filter, which `Supercluster::select` finds once
//! so many queries can reuse it, and the bit sets the filtered queries share.

use std::{
    fmt::{self, Debug, Formatter},
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
    use crate::{Supercluster, SuperclusterOptions};

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
