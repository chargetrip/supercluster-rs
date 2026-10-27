//! # Sums module
//!
//! Contains the running sums of the positions of points, which find the exact center of the points
//! of a cluster that pass a filter without reading all of them. The positions are summed as
//! integers, so every way of filtering finds the same center, whatever order it adds them in.

use std::{
    fmt::{self, Debug, Formatter},
    ops::{Add, Deref, Range, Sub},
};

use crate::selection::range_words;

/// The number of bits after the binary point of a position turned into an integer. A block of
/// 4096 positions of at most 2^50 each sums to less than 2^63, so the sum of a block fits a `u64`.
const FRACTION_BITS: i32 = 50;

/// The number of words of 64 points in a block, whose running sums start again from 0.
const BLOCK_WORDS: usize = 64;

/// The normalized area the integers cover: the area of the coordinate system, from 0 to 1, and its
/// width again on every side. A point further out counts as on the edge of the area, for centers
/// only, so a point far away does not make every center less precise.
const AREA: [f64; 2] = [-1.0, 2.0];

/// The highest integer a position turns into, which keeps every sum within its type.
const TOP: u64 = 1 << FRACTION_BITS;

/// Turns the normalized positions of a supercluster into integers, over the bounds of the positions
/// within `AREA`, so that sums of them are exact.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Quantizer {
    /// The lowest X and Y, which turn into 0.
    origin: [f64; 2],

    /// The integer units per normalized unit, or 0 when all the positions are the same.
    factor: f64,
}

impl Quantizer {
    /// Create a quantizer that spans the positions within `AREA`, so the widest axis turns into
    /// integers from 0 to 2^50.
    pub(crate) fn new(positions: &[[f64; 2]]) -> Self {
        let mut origin = [AREA[1]; 2];
        let mut limit = [AREA[0]; 2];

        for position in positions {
            for axis in 0..2 {
                let value = position[axis].clamp(AREA[0], AREA[1]);

                origin[axis] = origin[axis].min(value);
                limit[axis] = limit[axis].max(value);
            }
        }

        if positions.is_empty() {
            return Quantizer {
                origin: [0.0, 0.0],
                factor: 0.0,
            };
        }

        // At most the width of the area, so the span is always finite
        let span = f64::max(limit[0] - origin[0], limit[1] - origin[1]);
        let factor = if span > 0.0 {
            2f64.powi(FRACTION_BITS) / span
        } else {
            0.0
        };

        Quantizer { origin, factor }
    }

    /// Turn a normalized position into integers.
    pub(crate) fn quantize(&self, [x, y]: [f64; 2]) -> [u64; 2] {
        // Turning a float into an integer gives 0 below 0, and a position past the area stops at
        // `TOP`, so a point far away counts as at the edge, with one comparison per axis
        [
            (((x - self.origin[0]) * self.factor).round() as u64).min(TOP),
            (((y - self.origin[1]) * self.factor).round() as u64).min(TOP),
        ]
    }

    /// Turn the sum of `count` positions into the normalized position of their center.
    pub(crate) fn center(&self, sum: [u128; 2], count: u32) -> [f64; 2] {
        if self.factor == 0.0 || count == 0 {
            return self.origin;
        }

        // A float holds the sum to about one part in 2^53, which for a mean of at most 2^50 is a
        // quarter of a unit at most, less than the rounding of the integers, and it needs no
        // 128-bit division
        let count = f64::from(count);

        [
            self.origin[0] + sum[0] as f64 / count / self.factor,
            self.origin[1] + sum[1] as f64 / count / self.factor,
        ]
    }
}

/// The normalized positions of the points that were clustered, in leaf order, with what turns
/// them into integers and their running sums. A supercluster holds them in an `Arc`, and its
/// selections keep a weak reference. So a computed combination reaches the positions it sums
/// without the supercluster, a query tells whether the running sums of a selection are of its own
/// positions, and a selection does not keep them after the supercluster is dropped. Only the
/// positions are written with `serde`, and reading builds the rest again, so it always matches
/// them.
pub(crate) struct Positions {
    /// The position of every point that was clustered.
    positions: Vec<[f64; 2]>,

    /// Turns the positions into integers.
    quantizer: Quantizer,

    /// The running sums of all the positions, which give the sum of the positions of a cluster.
    sums: Sums,
}

impl Positions {
    /// Keep the positions, with a quantizer that spans them and their running sums.
    pub(crate) fn new(positions: Vec<[f64; 2]>) -> Self {
        let quantizer = Quantizer::new(&positions);
        let count = positions.len();
        let sums = Sums::new(
            count.div_ceil(64),
            |index| every_word(count, index),
            |leaf| quantizer.quantize(positions[leaf]),
        );

        Positions {
            positions,
            quantizer,
            sums,
        }
    }

    /// Sum the integer positions of the points at the positions `range`.
    pub(crate) fn sum_all(&self, range: Range<usize>) -> [u128; 2] {
        self.sums.sum(
            range,
            |index| every_word(self.positions.len(), index),
            |leaf| self.quantized(leaf),
        )
    }

    /// Get the position of the point at `leaf` as integers, for exact sums. A point past the last
    /// one, which only a selection of another supercluster reaches, counts as 0.
    pub(crate) fn quantized(&self, leaf: usize) -> [u64; 2] {
        self.positions
            .get(leaf)
            .map_or([0, 0], |&position| self.quantizer.quantize(position))
    }

    /// Turn the sum of the integer positions of `count` points into the normalized position of
    /// their center.
    pub(crate) fn center(&self, sum: [u128; 2], count: u32) -> [f64; 2] {
        self.quantizer.center(sum, count)
    }
}

// Written by hand, so a supercluster prints its positions as before, without the running sums.
impl Debug for Positions {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        self.positions.fmt(formatter)
    }
}

// Read like the list of positions it holds.
impl Deref for Positions {
    type Target = [[f64; 2]];

    fn deref(&self) -> &Self::Target {
        &self.positions
    }
}

/// Running sums of the integer positions of the points whose bit is set, which give the sum over
/// any range of points with two lookups, and the points of the two words at its ends.
#[derive(Clone, Debug)]
pub(crate) struct Sums {
    /// The sum before every block of words, and after the last one.
    blocks: Vec<[u128; 2]>,

    /// The sum before every word, from the start of its block, and after the last word.
    words: Vec<[u64; 2]>,
}

impl Sums {
    /// Sum the positions of the points of `word_count` words of 64 bits. `word` gets a word, and
    /// `position` gets the integer position of a point.
    pub(crate) fn new(
        word_count: usize,
        word: impl Fn(usize) -> u64,
        position: impl Fn(usize) -> [u64; 2],
    ) -> Self {
        let mut blocks = Vec::with_capacity(word_count / BLOCK_WORDS + 1);
        let mut words = Vec::with_capacity(word_count + 1);
        let mut block = [0u128; 2];
        let mut offset = [0u64; 2];

        for index in 0..=word_count {
            if index % BLOCK_WORDS == 0 {
                block = add(block, offset.map(u128::from));
                offset = [0, 0];
                blocks.push(block);
            }

            words.push(offset);

            if index < word_count {
                offset = add(offset, sum_word(word(index), index * 64, &position));
            }
        }

        Sums { blocks, words }
    }

    /// Sum the positions of the points of `range` whose bit is set. `word` and `position` are the
    /// ones the sums were made from.
    pub(crate) fn sum(
        &self,
        range: Range<usize>,
        word: impl Fn(usize) -> u64,
        position: impl Fn(usize) -> [u64; 2],
    ) -> [u128; 2] {
        let end = self.before(range.end, &word, &position);
        let start = self.before(range.start, &word, &position);

        sub(end, start)
    }

    /// Get the sum before `at`, from the running sum of the word around it and the points of that
    /// word on the shorter side. A point past the last word counts as no bit set.
    fn before(
        &self,
        at: usize,
        word: &impl Fn(usize) -> u64,
        position: &impl Fn(usize) -> [u64; 2],
    ) -> [u128; 2] {
        let last = self.words.len() - 1;
        let index = at / 64;

        if index >= last {
            return self.running(last);
        }

        let shift = at % 64;
        let bits = word(index);
        let below = (1u64 << shift) - 1;

        if shift <= 32 {
            add(
                self.running(index),
                sum_word(bits & below, index * 64, position).map(u128::from),
            )
        } else {
            let after = sum_word(bits & !below, index * 64, position).map(u128::from);
            let next = self.running(index + 1);

            sub(next, after)
        }
    }

    /// Get the sum before the word at `index`.
    fn running(&self, index: usize) -> [u128; 2] {
        add(
            self.blocks[index / BLOCK_WORDS],
            self.words[index].map(u128::from),
        )
    }
}

/// Get the word of 64 points at `index` with the bit of each of the first `count` points set.
fn every_word(count: usize, index: usize) -> u64 {
    match count.saturating_sub(index * 64) {
        remaining @ 0..64 => (1 << remaining) - 1,
        _ => u64::MAX,
    }
}

/// Sum the positions of the set bits of `bits`, where bit 0 is the point at `first`.
pub(crate) fn sum_word(
    mut bits: u64,
    first: usize,
    position: &impl Fn(usize) -> [u64; 2],
) -> [u64; 2] {
    let mut sum = [0, 0];

    while bits != 0 {
        sum = add(sum, position(first + bits.trailing_zeros() as usize));
        bits &= bits - 1;
    }

    sum
}

/// Sum the positions of the points of `range` whose bit is set in `words`, or is not set with
/// `invert`, where bit 0 of the words is the point at `first`.
pub(crate) fn sum_bits(
    words: &[u64],
    first: usize,
    range: Range<usize>,
    invert: bool,
    position: impl Fn(usize) -> [u64; 2],
) -> [u128; 2] {
    range_words(words, first, range, invert).fold([0, 0], |sum, (low, bits)| {
        add(sum, sum_word(bits, low, &position).map(u128::from))
    })
}

/// Add two sums, of X and of Y.
pub(crate) fn add<T: Add<Output = T> + Copy>(a: [T; 2], b: [T; 2]) -> [T; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

/// Take a sum from another, of X and of Y.
pub(crate) fn sub<T: Sub<Output = T> + Copy>(a: [T; 2], b: [T; 2]) -> [T; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The integer position of a point, which is its index on both axes.
    fn position(leaf: usize) -> [u64; 2] {
        [leaf as u64, 2 * leaf as u64]
    }

    /// Sum the positions of the set bits of a range one by one.
    fn brute_force(words: &[u64], range: Range<usize>) -> [u128; 2] {
        range
            .filter(|&leaf| words[leaf / 64] >> (leaf % 64) & 1 == 1)
            .fold([0, 0], |sum, leaf| add(sum, position(leaf).map(u128::from)))
    }

    #[test]
    fn test_sum_of_every_range() {
        // 130 words, so the sums span three blocks, with bits here and there
        let words: Vec<u64> = (0..130u64)
            .map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (i << 7))
            .collect();
        let sums = Sums::new(words.len(), |index| words[index], position);
        let points = words.len() * 64;

        for (start, end) in [
            (0, points),
            (5, 9),
            (63, 65),
            (100, 4200),
            (4095, 4097),
            (8000, points),
        ] {
            assert_eq!(
                sums.sum(start..end, |index| words[index], position),
                brute_force(&words, start..end),
                "{start}..{end}"
            );
        }
    }

    #[test]
    fn test_sum_past_the_last_word() {
        let words = [u64::MAX];
        let sums = Sums::new(words.len(), |index| words[index], position);

        // A selection of a smaller supercluster ends early, and the rest counts as no bit set
        assert_eq!(
            sums.sum(0..200, |index| words[index], position),
            brute_force(&words, 0..64)
        );
    }

    #[test]
    fn test_sum_bits_inverted() {
        let words = [0b1010, u64::MAX];
        let all = sum_bits(&words, 10, 10..140, false, position);
        let set = sum_bits(&words, 10, 12..100, false, position);
        let not_set = sum_bits(&words, 10, 12..100, true, position);
        let every = (12..100).fold([0, 0], |sum, leaf| add(sum, position(leaf).map(u128::from)));

        assert_eq!(add(set, not_set), every);
        assert_eq!(
            all,
            add(
                sum_bits(&words, 10, 10..12, false, position),
                sum_bits(&words, 10, 12..140, false, position)
            )
        );
    }

    #[test]
    fn test_quantizer_center() {
        let quantizer = Quantizer::new(&[[0.25, 0.5], [0.75, 0.5]]);
        let [a, b] = [[0.25, 0.5], [0.75, 0.5]].map(|point| quantizer.quantize(point));
        let center = quantizer.center(add(a.map(u128::from), b.map(u128::from)), 2);

        assert!((center[0] - 0.5).abs() < 1e-15 && (center[1] - 0.5).abs() < 1e-15);

        // Points at one position have no span, so they all turn into the origin
        let quantizer = Quantizer::new(&[[0.3, 0.3], [0.3, 0.3]]);

        assert_eq!(quantizer.center([0, 0], 2), [0.3, 0.3]);
    }
}
