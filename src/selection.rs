//! # Selection module
//!
//! Contains the selection of the points that pass a filter, which `Supercluster::select` finds once
//! so many queries can reuse it, its combinations, and the bit sets the filtered queries share.

use crate::sums::{sub, sum_bits, Positions, Sums};

use std::{
    borrow::Borrow,
    fmt::{self, Debug, Formatter},
    iter, mem,
    ops::Range,
    ptr,
    sync::{Arc, Weak},
};

/// The points of a supercluster that pass a filter, which `Supercluster::select` finds once, so
/// many queries can reuse them.
///
/// It holds one bit per point, and a count and a sum of the positions per 64 points, so it counts
/// the points of a cluster that pass, and finds their center, with a few lookups, whatever the size
/// of the cluster. A million points take about 445 KB. `and`, `or` and `and_not` combine
/// selections into a `Combination`.
///
/// A selection belongs to the supercluster that made it. With another supercluster the queries
/// do not panic, but their results mean nothing. See `Supercluster::select` for an example.
#[derive(Clone)]
pub struct Selection {
    /// One bit per point, in the order of `Supercluster::points`, set when the point passes.
    words: Vec<u64>,

    /// The number of bits set before every word, followed by the number in all of them.
    ranks: Vec<u32>,

    /// The running sums of the positions of the points that pass, with the positions of the
    /// supercluster that made it. A computed combination whose supercluster was dropped has none.
    sums: Option<Sums>,

    /// The positions of the points of the supercluster that made it, which `compute` sums, and
    /// which tell a query whether the running sums are of its own positions. The reference is weak,
    /// so a selection does not keep the positions after the supercluster is dropped.
    positions: Weak<Positions>,
}

impl Selection {
    /// Create a selection from one bit per point, set when the point passes, and the positions of
    /// the points of its supercluster, or `None` when it was dropped.
    pub(crate) fn new(words: Vec<u64>, positions: Option<&Arc<Positions>>) -> Self {
        let mut ranks = Vec::with_capacity(words.len() + 1);
        let mut rank = 0;

        ranks.push(rank);

        for word in &words {
            rank += word.count_ones();
            ranks.push(rank);
        }

        let sums = positions.map(|positions| {
            Sums::new(
                words.len(),
                |index| words[index],
                |leaf| positions.quantized(leaf),
            )
        });

        Selection {
            words,
            ranks,
            sums,
            positions: positions.map_or_else(Weak::new, Arc::downgrade),
        }
    }

    /// Get the running sums when they are of `positions`, the positions of the supercluster of the
    /// query. The positions of another supercluster do not add up with them. The weak reference
    /// keeps the place of the positions it points to, so new positions never take it.
    fn sums_of(&self, positions: &Positions) -> Option<&Sums> {
        self.sums
            .as_ref()
            .filter(|_| ptr::eq(self.positions.as_ptr(), positions))
    }

    /// Find the first point at the positions of `range` that passes, with a binary search over the
    /// counts, so a long run of points that fail costs about as little as a short one.
    fn first(&self, range: Range<usize>) -> Option<usize> {
        let before = self.rank(range.start);

        if self.rank(range.end) == before {
            return None;
        }

        // The first word with more points that pass up to its end than before the range holds it
        let start = range.start / 64;
        let end = range.end.div_ceil(64).min(self.words.len());
        let word = start + self.ranks[start + 1..=end].partition_point(|&rank| rank <= before);
        let mut bits = self.words[word];

        if word == start {
            bits &= !0u64 << (range.start % 64);
        }

        Some(word * 64 + bits.trailing_zeros() as usize)
    }

    /// Combine the points that pass both this selection and `other`: a selection, behind a
    /// reference, an `Arc` or a `Box`, or a combination, by value or by reference.
    ///
    /// Combining computes nothing: the combination borrows the selections, and a query evaluates
    /// it on the points of its tile only, so its cost grows with the points of the tile, not with
    /// all the points. Both selections must come from the same supercluster: with a selection of
    /// another one, the queries do not panic, but their results mean nothing.
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
    /// // The items borrow the selections, not the combination, so it can be a temporary
    /// let items = supercluster.tile_selected(0, 0, 0, fast.and(&available));
    ///
    /// assert_eq!(items[0].count(), 1);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn and<'s>(&'s self, other: impl Into<Combination<'s>>) -> Combination<'s> {
        Combination::from(self).and(other)
    }

    /// Combine the points that pass this selection, `other` or both, like `and`.
    pub fn or<'s>(&'s self, other: impl Into<Combination<'s>>) -> Combination<'s> {
        Combination::from(self).or(other)
    }

    /// Combine the points that pass this selection but not `other`, like `and`.
    pub fn and_not<'s>(&'s self, other: impl Into<Combination<'s>>) -> Combination<'s> {
        Combination::from(self).and_not(other)
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

/// Selections combined with `and`, `or` and `and_not`, which borrows them and computes nothing.
///
/// A query evaluates a combination on the points of its tile only, so its cost grows with the
/// points of the tile, not with all the points. Nothing is cached: every query evaluates it again,
/// so it always reads the current selections. `compute` turns it into a `Selection`, which a query
/// reads in constant time, to cache a combination that many queries share.
///
/// Its own `and`, `or` and `and_not` take it by value, so one expression combines any number of
/// selections, such as `a.or(&b).and(&c)`. To use a combination in another one and keep it, pass a
/// reference to it, which copies its steps.
#[derive(Clone)]
pub struct Combination<'s> {
    /// The steps that evaluate the combination, in postfix order.
    steps: Vec<Step<'s>>,
}

/// One step of the evaluation of a combination.
#[derive(Clone, Copy)]
enum Step<'s> {
    /// Take the bits of a selection.
    Take(&'s Selection),

    /// Keep the bits that the last two have both.
    And,

    /// Keep the bits that either of the last two has.
    Or,

    /// Keep the bits of the second to last that the last one does not have.
    AndNot,
}

// A selection behind a reference, an `Arc`, a `Box` or an `Rc`
impl<'s, T: Borrow<Selection> + ?Sized> From<&'s T> for Combination<'s> {
    fn from(selection: &'s T) -> Self {
        Combination {
            steps: vec![Step::Take(T::borrow(selection))],
        }
    }
}

// A combination that is reused, which copies its steps
impl<'s> From<&Combination<'s>> for Combination<'s> {
    fn from(combination: &Combination<'s>) -> Self {
        combination.clone()
    }
}

impl<'s> Combination<'s> {
    /// Combine the points that pass both this combination and `other`, like `Selection::and`.
    pub fn and(self, other: impl Into<Combination<'s>>) -> Combination<'s> {
        self.combine(other.into(), Step::And)
    }

    /// Combine the points that pass this combination, `other` or both, like `Selection::or`.
    pub fn or(self, other: impl Into<Combination<'s>>) -> Combination<'s> {
        self.combine(other.into(), Step::Or)
    }

    /// Combine the points that pass this combination but not `other`, like
    /// `Selection::and_not`.
    pub fn and_not(self, other: impl Into<Combination<'s>>) -> Combination<'s> {
        self.combine(other.into(), Step::AndNot)
    }

    /// Compute the bits of every point into a `Selection`, which a query reads in constant time,
    /// and which does not borrow the selections it combines, so a cache can keep it. It reads the
    /// positions of the points that pass, so a query finds the center of the points of a cluster
    /// that pass without reading them again.
    ///
    /// # Examples
    ///
    /// ```
    /// use supercluster::{Selection, Supercluster, SuperclusterOptions};
    ///
    /// // A station is a longitude, a latitude, its power in kW and whether it is available
    /// let stations = vec![(10.0, 50.0, 150, true), (10.001, 50.0, 150, false), (10.002, 50.0, 11, true)];
    /// let supercluster = Supercluster::load(SuperclusterOptions::default(), stations, |station| {
    ///     Some([station.0, station.1])
    /// })?;
    /// let fast = supercluster.select(|station| station.2 >= 50);
    /// let available = supercluster.select(|station| station.3);
    ///
    /// // A cache can keep it, because it does not borrow `fast` and `available`
    /// let cached: Selection = fast.and(&available).compute();
    ///
    /// assert_eq!(supercluster.tile_selected(0, 0, 0, &cached)[0].count(), 1);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn compute(&self) -> Selection {
        let words = self
            .steps
            .iter()
            .map(|step| match step {
                Step::Take(selection) => selection.words.len(),
                _ => 0,
            })
            .max()
            .unwrap_or_default();
        let mut scratch = Scratch::default();

        let words = mem::take(evaluate(&self.steps, 0..words, &mut scratch));

        // The sums are built now from the positions of the supercluster of the first selection
        // whose supercluster is still there. When none is, there are none, and a query sums the
        // bits itself. Queries use them only on that supercluster, so either way is safe
        let positions = self.steps.iter().find_map(|step| match step {
            Step::Take(selection) => selection.positions.upgrade(),
            _ => None,
        });

        Selection::new(words, positions.as_ref())
    }

    /// Combine this combination with `other` in one more step.
    fn combine(mut self, other: Combination<'s>, step: Step<'s>) -> Combination<'s> {
        self.steps.extend(other.steps);
        self.steps.push(step);

        self
    }
}

// Written by hand, because a derive would print the selections it combines.
impl Debug for Combination<'_> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let selections = self
            .steps
            .iter()
            .filter(|step| matches!(step, Step::Take(_)))
            .count();

        formatter
            .debug_struct("Combination")
            .field("selections", &selections)
            .finish_non_exhaustive()
    }
}

/// The points that `tile_selected` and `cluster_expansion_zoom_selected` keep: the ones of a
/// `Selection`, behind a reference, an `Arc` or a `Box`, or of a `Combination` of selections, by
/// value or by reference. `'s` is the lifetime of the selections, which the items of a tile borrow.
/// Only this crate implements it.
pub trait Select<'s>: private::Sealed<'s> {}

impl<'s, T: Borrow<Selection> + ?Sized> Select<'s> for &'s T {}

impl<'s> Select<'s> for Combination<'s> {}

impl<'s> Select<'s> for &Combination<'s> {}

/// The trait that keeps other crates from implementing `Select`, with what the queries need.
pub(crate) mod private {
    use std::ops::Range;

    use super::{Matches, Scratch};

    /// What a query needs from the points that it keeps, whose selections live for `'s`.
    pub trait Sealed<'s> {
        /// Count the points at the positions of `range` that pass, and find which ones, or get
        /// `None` if no point passes. `scratch` holds buffers that a query reuses for every item.
        fn matches(&self, range: Range<usize>, scratch: &mut Scratch)
            -> Option<(u32, Matches<'s>)>;

        /// Check whether a point at the positions of `range` passes. `scratch` holds buffers
        /// that a query reuses for every check.
        fn any(&self, range: Range<usize>, scratch: &mut Scratch) -> bool;
    }
}

impl<'s, T: Borrow<Selection> + ?Sized> private::Sealed<'s> for &'s T {
    fn matches(&self, range: Range<usize>, _: &mut Scratch) -> Option<(u32, Matches<'s>)> {
        let selection = T::borrow(self);
        let count = selection.count(range.clone());

        Matches::found(count, range.len(), || Matches::Shared(selection))
    }

    fn any(&self, range: Range<usize>, _: &mut Scratch) -> bool {
        T::borrow(self).count(range) > 0
    }
}

impl<'s> private::Sealed<'s> for Combination<'s> {
    fn matches(&self, range: Range<usize>, scratch: &mut Scratch) -> Option<(u32, Matches<'s>)> {
        // A combination of a single selection reads the counts of the selection
        if let [Step::Take(selection)] = self.steps[..] {
            return selection.matches(range, scratch);
        }

        let length = range.len();
        let (count, first) = evaluate_range(&self.steps, range, scratch);

        // Only an item with some points that fail takes the words, so the buffer serves the next
        // items otherwise
        Matches::found(count, length, || Matches::Local {
            words: mem::take(&mut scratch.stack[0]),
            first,
        })
    }

    fn any(&self, range: Range<usize>, scratch: &mut Scratch) -> bool {
        if let [Step::Take(selection)] = self.steps[..] {
            return selection.any(range, scratch);
        }

        evaluate_range(&self.steps, range, scratch).0 > 0
    }
}

impl<'s> private::Sealed<'s> for &Combination<'s> {
    fn matches(&self, range: Range<usize>, scratch: &mut Scratch) -> Option<(u32, Matches<'s>)> {
        (**self).matches(range, scratch)
    }

    fn any(&self, range: Range<usize>, scratch: &mut Scratch) -> bool {
        (**self).any(range, scratch)
    }
}

/// Buffers to evaluate combinations, which a query reuses for every item it evaluates.
#[derive(Default)]
pub struct Scratch {
    /// The words of every step that waits for the next one, the last one on top.
    stack: Vec<Vec<u64>>,
}

/// Evaluate a combination on the words that hold the positions of `range`, from the word of its
/// first position, into the first buffer of `scratch`. Returns the number of points of `range`
/// that pass, and the position of the first bit of the words.
fn evaluate_range(steps: &[Step<'_>], range: Range<usize>, scratch: &mut Scratch) -> (u32, usize) {
    let words = range.start / 64..range.end.div_ceil(64);
    let first = words.start * 64;
    let count = count_set(evaluate(steps, words, scratch), first, range);

    (count, first)
}

/// Evaluate a combination on the words of `words`, and get the result, which is the first buffer of
/// `scratch`. A word that a selection lacks counts as no bit set, so a selection of another
/// supercluster does not make it panic. Every step works on a whole buffer of words, which the
/// processor runs fast.
fn evaluate<'b>(
    steps: &[Step<'_>],
    words: Range<usize>,
    scratch: &'b mut Scratch,
) -> &'b mut Vec<u64> {
    // The number of buffers on the stack that hold words of a step
    let mut top = 0;
    let mut steps = steps.iter().peekable();

    while let Some(&step) = steps.next() {
        let Step::Take(selection) = step else {
            // An operation after two buffers combines the top one into the one below it
            top -= 1;

            let (lower, upper) = scratch.stack.split_at_mut(top);

            apply(step, &mut lower[top - 1], &upper[0]);

            continue;
        };

        let source = words_of(selection, &words);

        // An operation right after a selection combines its words straight into the buffer below,
        // without copying them into a buffer of their own first
        if let Some(&operation) = steps.next_if(|step| top > 0 && !matches!(step, Step::Take(_))) {
            apply(operation, &mut scratch.stack[top - 1], source);

            continue;
        }

        if scratch.stack.len() == top {
            scratch.stack.push(vec![]);
        }

        let buffer = &mut scratch.stack[top];

        buffer.clear();
        buffer.extend_from_slice(source);
        buffer.resize(words.len(), 0);
        top += 1;
    }

    &mut scratch.stack[0]
}

/// Get the words of a selection at the positions `words`, which are fewer when the selection ends
/// before them.
fn words_of<'s>(selection: &'s Selection, words: &Range<usize>) -> &'s [u64] {
    let end = words.end.min(selection.words.len());
    let start = words.start.min(end);

    &selection.words[start..end]
}

/// Combine the words of `right` into `left` with an operation, one loop per operation, which the
/// compiler turns into operations on many words at once. A word that `right` lacks counts as no
/// bit set.
fn apply(operation: Step<'_>, left: &mut [u64], right: &[u64]) {
    let pairs = left.iter_mut().zip(right);

    match operation {
        Step::And => {
            pairs.for_each(|(left, right)| *left &= right);

            let lacking = right.len().min(left.len());

            left[lacking..].fill(0);
        }
        Step::Or => pairs.for_each(|(left, right)| *left |= right),
        Step::AndNot => pairs.for_each(|(left, right)| *left &= !right),
        Step::Take(_) => {}
    }
}

/// Count the bits set at the positions of `range` in `words`, whose first bit is the one of
/// position `first`, which is not after the start of the range.
fn count_set(words: &[u64], first: usize, range: Range<usize>) -> u32 {
    if range.is_empty() {
        return 0;
    }

    // Unlike `range_words`, it checks the range once, at the first and the last word, not at every
    // word, and counts the words between as they are. A combination counts the points of every
    // item it evaluates, and the check per word made that measurably slower
    let start = range.start - first;
    let end = range.end - first;
    let (first_word, last_word) = (start / 64, (end - 1) / 64);
    let low = u64::MAX << (start % 64);
    let high = u64::MAX >> (63 - (end - 1) % 64);

    if first_word == last_word {
        return (words[first_word] & low & high).count_ones();
    }

    let middle: u32 = words[first_word + 1..last_word]
        .iter()
        .map(|word| word.count_ones())
        .sum();

    (words[first_word] & low).count_ones() + middle + (words[last_word] & high).count_ones()
}

/// Get the words that hold the positions of `range` in `words`, whose first bit is the one of
/// position `first`, each with the position of its first bit. The bits are inverted with `invert`,
/// then the ones outside the range are cleared. A word past the end of `words` has no bit set.
pub(crate) fn range_words(
    words: &[u64],
    first: usize,
    range: Range<usize>,
    invert: bool,
) -> impl Iterator<Item = (usize, u64)> + '_ {
    let (start, end) = (range.start - first, range.end - first);

    (start / 64..end.div_ceil(64)).map(move |index| {
        let low = index * 64;
        let mut bits = words.get(index).copied().unwrap_or_default();

        if invert {
            bits = !bits;
        }

        if start > low {
            bits &= !0u64 << (start - low);
        }

        if end < low + 64 {
            bits &= (1u64 << (end - low)) - 1;
        }

        (first + low, bits)
    })
}

/// Which points of a filtered item pass the filter.
#[derive(Clone)]
pub enum Matches<'a> {
    /// Every point of the item passes.
    All,

    /// Bits for some positions, set when the point there passes.
    Local {
        /// The bits.
        words: Vec<u64>,

        /// The position of the first bit of the words.
        first: usize,
    },

    /// The points of a selection pass.
    Shared(&'a Selection),
}

impl<'a> Matches<'a> {
    /// Turn the number of points that pass, out of `length`, into what a query keeps: `None` when
    /// no point passes, `All` when every point passes, and what `some` returns otherwise.
    pub(crate) fn found(
        count: u32,
        length: usize,
        some: impl FnOnce() -> Matches<'a>,
    ) -> Option<(u32, Matches<'a>)> {
        match count {
            0 => None,
            _ if count as usize == length => Some((count, Matches::All)),
            _ => Some((count, some())),
        }
    }

    /// Find the position of the first point at the positions of `range` that passes.
    pub(crate) fn first(&self, range: Range<usize>) -> Option<usize> {
        match self {
            Matches::All => (!range.is_empty()).then_some(range.start),
            Matches::Local { words, first } => range_words(words, *first, range, false)
                .find(|&(_, bits)| bits != 0)
                .map(|(low, bits)| low + bits.trailing_zeros() as usize),
            Matches::Shared(selection) => selection.first(range),
        }
    }

    /// Iterate over the positions in `range` of the points that pass, a word at a time, so a long
    /// run of points that fail costs one step per 64 of them.
    pub(crate) fn passing(&self, range: Range<usize>) -> impl Iterator<Item = usize> + '_ {
        // Every point passes: the inverted bits of no words are all set
        let (words, first, invert) = match self {
            Matches::All => (&[][..], range.start, true),
            Matches::Local { words, first } => (&words[..], *first, false),
            Matches::Shared(selection) => (&selection.words[..], 0, false),
        };

        range_words(words, first, range, invert).flat_map(|(low, mut bits)| {
            iter::from_fn(move || {
                let bit = bits.trailing_zeros() as usize;

                bits &= bits.wrapping_sub(1);
                (bit < 64).then_some(low + bit)
            })
        })
    }

    /// Sum the integer positions of the `count` points at the positions of `range` that pass, with
    /// the positions of the supercluster of the query.
    pub(crate) fn sum(&self, range: Range<usize>, count: u32, positions: &Positions) -> [u128; 2] {
        match self {
            Matches::All => positions.sum_all(range),
            Matches::Shared(selection) => match selection.sums_of(positions) {
                Some(sums) => sums.sum(
                    range,
                    |index| selection.words[index],
                    |leaf| positions.quantized(leaf),
                ),
                // A selection of another supercluster has no running sums of these positions, so
                // its bits are summed here, like the bits of a combination
                None => sum_passing(&selection.words, 0, range, count, positions),
            },
            Matches::Local { words, first } => sum_passing(words, *first, range, count, positions),
        }
    }

    /// Check whether a point at the positions of `range` passes.
    pub(crate) fn any(&self, range: Range<usize>) -> bool {
        match self {
            Matches::All => !range.is_empty(),
            Matches::Local { words, first } => {
                range_words(words, *first, range, false).any(|(_, bits)| bits != 0)
            }
            Matches::Shared(selection) => selection.count(range) > 0,
        }
    }
}

/// Sum the integer positions of the `count` points at the positions of `range` whose bit is set in
/// `words`, where bit 0 is the point at `first`. When more than half are set, it sums the points
/// whose bit is not set and takes them from the sum of all, so it reads at most half of the points.
fn sum_passing(
    words: &[u64],
    first: usize,
    range: Range<usize>,
    count: u32,
    positions: &Positions,
) -> [u128; 2] {
    let position = |leaf| positions.quantized(leaf);

    if count as usize * 2 > range.len() {
        let failing = sum_bits(words, first, range.clone(), true, position);

        sub(positions.sum_all(range), failing)
    } else {
        sum_bits(words, first, range, false, position)
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use crate::{test_points::load_spread_points, Item, Supercluster, SuperclusterOptions};

    /// Keeps about one point in three, here and there.
    fn every_third(point: &[f64; 2]) -> bool {
        (point[0] * 1e6) as i64 % 3 == 0
    }

    /// Keeps the points of the northern half.
    fn north(point: &[f64; 2]) -> bool {
        point[1] > 0.0
    }

    /// Keeps the points of the eastern half.
    fn east(point: &[f64; 2]) -> bool {
        point[0] > 0.0
    }

    /// The position, count, number of points and expansion zoom of every item of a tile, and the
    /// expansion zoom that `cluster_expansion_zoom_selected` finds for every cluster.
    type Summary = Vec<(f64, f64, u32, usize, Option<u8>)>;

    /// Summarize the items of the tile `z`/`x`/`y` that `selection` keeps.
    fn summarize<'s, S: Select<'s> + Copy>(
        supercluster: &'s Supercluster<[f64; 2]>,
        (z, x, y): (u8, u32, u32),
        selection: S,
    ) -> Summary {
        supercluster
            .tile_selected(z, x, y, selection)
            .iter()
            .map(|item| {
                let expansion_zoom = match item.item {
                    Item::Cluster(cluster) => Some(
                        supercluster
                            .cluster_expansion_zoom_selected(cluster.id, selection)
                            .unwrap(),
                    ),
                    Item::Point { .. } => None,
                };

                assert_eq!(item.expansion_zoom(), expansion_zoom);

                (
                    item.x,
                    item.y,
                    item.count(),
                    item.points().count(),
                    expansion_zoom,
                )
            })
            .collect()
    }

    /// Check that a combination gives the results of a selection, evaluated by the queries and
    /// computed: the same number of points, and the same items in every tile up to zoom level 4.
    fn assert_same_results(
        supercluster: &Supercluster<[f64; 2]>,
        combination: &Combination<'_>,
        selection: &Selection,
    ) {
        let computed = combination.compute();

        assert_eq!(format!("{computed:?}"), format!("{selection:?}"));

        for z in 0..=4 {
            for x in 0..1 << z {
                for y in 0..1 << z {
                    let expected = summarize(supercluster, (z, x, y), selection);

                    assert_eq!(
                        summarize(supercluster, (z, x, y), combination),
                        expected,
                        "tile {z}/{x}/{y}"
                    );
                    assert_eq!(
                        summarize(supercluster, (z, x, y), &computed),
                        expected,
                        "tile {z}/{x}/{y}"
                    );
                }
            }
        }
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

    #[test]
    fn test_and_matches_select_with_both_filters() {
        let supercluster = load_spread_points(500);
        let (third, northern) = (supercluster.select(every_third), supercluster.select(north));
        let selected = supercluster.select(|point| every_third(point) && north(point));

        assert_same_results(&supercluster, &third.and(&northern), &selected);
    }

    #[test]
    fn test_or_matches_select_with_either_filter() {
        let supercluster = load_spread_points(500);
        let (third, northern) = (supercluster.select(every_third), supercluster.select(north));
        let selected = supercluster.select(|point| every_third(point) || north(point));

        assert_same_results(&supercluster, &third.or(&northern), &selected);
    }

    #[test]
    fn test_and_not_matches_select_with_the_first_filter_only() {
        let supercluster = load_spread_points(500);
        let (third, northern) = (supercluster.select(every_third), supercluster.select(north));
        let selected = supercluster.select(|point| every_third(point) && !north(point));

        assert_same_results(&supercluster, &third.and_not(&northern), &selected);
    }

    #[test]
    fn test_and_in_one_expression() {
        let supercluster = load_spread_points(500);
        let (third, northern, eastern) = (
            supercluster.select(every_third),
            supercluster.select(north),
            supercluster.select(east),
        );

        // A combination takes the next selection by value, so a chain needs no variables
        let combination = third.or(&northern).and(&eastern);
        let selected =
            supercluster.select(|point| (every_third(point) || north(point)) && east(point));

        assert_same_results(&supercluster, &combination, &selected);
    }

    #[test]
    fn test_and_of_a_combination() {
        let supercluster = load_spread_points(500);
        let (third, northern, eastern) = (
            supercluster.select(every_third),
            supercluster.select(north),
            supercluster.select(east),
        );
        let combination = third.and(northern.and_not(&eastern));
        let selected =
            supercluster.select(|point| every_third(point) && north(point) && !east(point));

        assert_same_results(&supercluster, &combination, &selected);
    }

    #[test]
    fn test_and_with_a_borrowed_combination() {
        let supercluster = load_spread_points(500);
        let (third, northern, eastern) = (
            supercluster.select(every_third),
            supercluster.select(north),
            supercluster.select(east),
        );
        let either = third.or(&northern);

        // A borrowed combination is copied into the new one, so it stays usable
        let combination = eastern.and(&either);
        let selected =
            supercluster.select(|point| east(point) && (every_third(point) || north(point)));

        assert_same_results(&supercluster, &combination, &selected);
        assert_eq!(format!("{either:?}"), "Combination { selections: 2, .. }");
    }

    #[test]
    fn test_tile_selected_with_a_temporary_combination() {
        let supercluster = load_spread_points(500);
        let (third, northern) = (supercluster.select(every_third), supercluster.select(north));
        let selected = supercluster.select(|point| every_third(point) && north(point));

        // The items borrow the selections, not the combination, so they outlive it
        let items = supercluster.tile_selected(0, 0, 0, third.and(&northern));
        let expected = supercluster.tile_selected(0, 0, 0, &selected);
        let summary = |items: &[crate::FilteredTileItem<'_, [f64; 2]>]| -> Vec<(f64, f64, u32)> {
            items
                .iter()
                .map(|item| (item.x, item.y, item.count()))
                .collect()
        };

        assert!(!items.is_empty());
        assert_eq!(summary(&items), summary(&expected));
    }

    #[test]
    fn test_tile_selected_with_selections_behind_an_arc_or_a_box() {
        let supercluster = load_spread_points(500);
        let third = Arc::new(supercluster.select(every_third));
        let northern = Box::new(supercluster.select(north));
        let selected = supercluster.select(|point| every_third(point) && north(point));

        // The queries and the operators take them as they are
        assert_eq!(
            summarize(&supercluster, (1, 0, 0), &third),
            summarize(&supercluster, (1, 0, 0), third.as_ref())
        );
        assert_same_results(&supercluster, &third.and(&northern), &selected);
    }

    #[test]
    fn test_selection_does_not_keep_the_positions_of_a_dropped_supercluster() {
        let supercluster = load_spread_points(500);
        let selection = supercluster.select(every_third);
        let positions = Arc::downgrade(supercluster.positions());

        drop(supercluster);

        // The positions are freed, and computing a combination still works
        assert!(positions.upgrade().is_none());

        let computed = selection.and(&selection).compute();

        assert_eq!(format!("{computed:?}"), format!("{selection:?}"));

        // With the same points loaded again, it has no running sums, so a query sums its bits with
        // the positions of the new supercluster, and its tiles are the ones of a fresh selection
        let reloaded = load_spread_points(500);
        let fresh = reloaded.select(every_third);
        let tiles = |selection: &Selection| -> Vec<([f64; 2], f64, f64, u32)> {
            reloaded
                .tile_selected(0, 0, 0, selection)
                .iter()
                .map(|item| (item.coordinates(), item.x, item.y, item.count()))
                .collect()
        };

        assert_eq!(tiles(&computed), tiles(&fresh));
    }

    #[test]
    fn test_compute_of_a_single_selection() {
        let supercluster = load_spread_points(130);
        let selection = supercluster.select(every_third);

        assert_same_results(&supercluster, &Combination::from(&selection), &selection);
    }

    #[test]
    fn test_debug_of_a_combination() {
        let supercluster = load_spread_points(10);
        let (third, northern, all) = (
            supercluster.select(every_third),
            supercluster.select(north),
            supercluster.select(|_| true),
        );

        // A combination computes nothing, so it tells how many selections it combines
        assert_eq!(
            format!("{:?}", third.or(&northern).and(&all)),
            "Combination { selections: 3, .. }"
        );
    }

    #[test]
    fn test_and_with_a_selection_of_another_supercluster() {
        let small = load_spread_points(10);
        let large = load_spread_points(500);
        let small_selection = small.select(|_| true);
        let large_selection = large.select(|_| true);

        // The words that the smaller selection lacks count as no bit set
        assert_eq!(
            format!("{:?}", large_selection.and(&small_selection).compute()),
            "Selection { selected: 10, .. }"
        );
        assert_eq!(
            format!("{:?}", large_selection.and_not(&small_selection).compute()),
            "Selection { selected: 490, .. }"
        );

        // The results mean nothing, but neither computing nor a query panics
        small_selection.or(&large_selection).compute();
        small.tile_selected(0, 0, 0, small_selection.and(&large_selection));
        large.tile_selected(0, 0, 0, large_selection.and(&small_selection));
    }

    #[test]
    fn test_and_without_points() {
        let supercluster = load_spread_points(0);
        let selection = supercluster.select(|_| true);
        let combination = selection.and(&selection);

        assert_eq!(
            format!("{:?}", combination.compute()),
            "Selection { selected: 0, .. }"
        );
        assert!(supercluster.tile_selected(0, 0, 0, &combination).is_empty());
    }
}
