//! # Selection module
//!
//! Contains the selection of the points that pass a filter, which `Supercluster::select` finds once
//! so many queries can reuse it, its combinations, and the bit sets the filtered queries share.

use std::{
    fmt::{self, Debug, Formatter},
    mem,
    ops::Range,
};

/// The points of a supercluster that pass a filter, which `Supercluster::select` finds once, so
/// many queries can reuse them.
///
/// It holds one bit per point, and a count per 64 points, so it counts the points of a cluster
/// that pass with two lookups, whatever the size of the cluster. A million points take about
/// 190 KB. `and`, `or` and `and_not` combine selections into a `Combination`.
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

    /// Combine the points that pass both this selection and `other`, a selection or a
    /// combination.
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
    /// // The items borrow the combination, so keep it while they are used
    /// let fast_and_available = fast.and(&available);
    /// let items = supercluster.tile_selected(0, 0, 0, &fast_and_available);
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
/// selections, such as `a.or(&b).and(&c)`.
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

impl<'s> From<&'s Selection> for Combination<'s> {
    fn from(selection: &'s Selection) -> Self {
        Combination {
            steps: vec![Step::Take(selection)],
        }
    }
}

impl<'s> Combination<'s> {
    /// Combine the points that pass both this combination and `other`, a selection or a
    /// combination, like `Selection::and`.
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
    /// and which does not borrow the selections it combines, so a cache can keep it.
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

        Selection::new(mem::take(evaluate(&self.steps, 0..words, &mut scratch)))
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

/// The points that a query keeps: the ones of a `Selection`, or of a `Combination` of selections.
/// Only this crate implements it.
pub trait Select: private::Sealed {}

impl Select for Selection {}

impl Select for Combination<'_> {}

/// The trait that keeps other crates from implementing `Select`, with what the queries need.
pub(crate) mod private {
    use std::ops::Range;

    use super::{Matches, Scratch};

    /// What a query needs from the points that it keeps.
    pub trait Sealed {
        /// Count the points at the positions of `range` that pass, and find which ones, or get
        /// `None` if no point passes. `scratch` holds buffers that a query reuses for every item.
        fn matches(&self, range: Range<usize>, scratch: &mut Scratch)
            -> Option<(u32, Matches<'_>)>;

        /// Check whether a point at the positions of `range` passes. `scratch` holds buffers
        /// that a query reuses for every check.
        fn any(&self, range: Range<usize>, scratch: &mut Scratch) -> bool;
    }
}

impl private::Sealed for Selection {
    fn matches(&self, range: Range<usize>, _: &mut Scratch) -> Option<(u32, Matches<'_>)> {
        let count = self.count(range.clone());

        match count {
            0 => None,
            _ if count as usize == range.len() => Some((count, Matches::All)),
            _ => Some((count, Matches::Shared(self))),
        }
    }

    fn any(&self, range: Range<usize>, _: &mut Scratch) -> bool {
        self.count(range) > 0
    }
}

impl private::Sealed for Combination<'_> {
    fn matches(&self, range: Range<usize>, scratch: &mut Scratch) -> Option<(u32, Matches<'_>)> {
        // A combination of a single selection reads the counts of the selection
        if let [Step::Take(selection)] = self.steps[..] {
            return selection.matches(range, scratch);
        }

        let length = range.len();
        let (count, first) = evaluate_range(&self.steps, range, scratch);

        match count {
            0 => None,
            _ if count as usize == length => Some((count, Matches::All)),
            _ => {
                // Only an item with some points that fail takes the words, so the buffer serves
                // the next items otherwise
                let words = mem::take(&mut scratch.stack[0]);

                Some((count, Matches::Local { words, first }))
            }
        }
    }

    fn any(&self, range: Range<usize>, scratch: &mut Scratch) -> bool {
        if let [Step::Take(selection)] = self.steps[..] {
            return selection.count(range) > 0;
        }

        evaluate_range(&self.steps, range, scratch).0 > 0
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

    let start = range.start - first;
    let end = range.end - first;
    let (first_word, last_word) = (start / 64, (end - 1) / 64);

    // The bits of the first and last word that are part of the range
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

impl Matches<'_> {
    /// Iterate over the positions in `range` of the points that pass.
    pub(crate) fn passing(&self, range: Range<usize>) -> SetPositions<'_> {
        match self {
            Matches::All => SetPositions::new(None, 0, range),
            Matches::Local { words, first } => SetPositions::new(Some(words), *first, range),
            Matches::Shared(selection) => SetPositions::new(Some(&selection.words), 0, range),
        }
    }

    /// Check whether a point at the positions of `range` passes.
    pub(crate) fn any(&self, range: Range<usize>) -> bool {
        match self {
            Matches::Shared(selection) => selection.count(range) > 0,
            _ => self.passing(range).next().is_some(),
        }
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
    fn summarize<S: Select>(
        supercluster: &Supercluster<[f64; 2]>,
        (z, x, y): (u8, u32, u32),
        selection: &S,
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
        small.tile_selected(0, 0, 0, &small_selection.and(&large_selection));
        large.tile_selected(0, 0, 0, &large_selection.and(&small_selection));
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
