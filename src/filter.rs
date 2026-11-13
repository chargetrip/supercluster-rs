//! # Filter module
//!
//! Contains the filters that `with_filter` takes: a `Filter`, which runs a closure on the
//! points of each query, a `CachedFilter`, which `Supercluster::cache_filter` finds once so many
//! queries can reuse it, and their combined filters, with the bit sets the filtered queries share.

use crate::sums::{sub, sum_bits, Positions, Sums};

use std::{
    borrow::Borrow,
    fmt::{self, Debug, Formatter},
    iter,
    marker::PhantomData,
    mem,
    ops::Range,
    ptr,
    sync::{Arc, Weak},
};

/// The points of a supercluster that pass a closure, found once by `Supercluster::cache_filter` so
/// that many queries can reuse them. It is a snapshot and does not follow the data the closure
/// read, so make a new one when that data changes.
///
/// It holds one bit per point, and a count and a sum of the positions for every 64 points. A query
/// counts the points of a cluster that pass, and finds their center, with a few lookups, however
/// large the cluster is. A million points take about 445 KB. `and`, `or` and `and_not` combine
/// cached filters into a `CombinedFilter`.
///
/// A cached filter works with the supercluster that made it and with its clones, which hold the
/// same points in the same order. `with_filter` on any other supercluster returns
/// `SuperclusterError::ForeignCachedFilter`, even if it was loaded from the same points. See
/// `Supercluster::cache_filter` for an example.
#[derive(Clone)]
pub struct CachedFilter {
    /// One bit per point, in the order of `Supercluster::points`, set when the point passes.
    words: Vec<u64>,

    /// The number of bits set before every word, followed by the number in all of them.
    ranks: Vec<u32>,

    /// The running sums of the positions of the points that pass, with the positions of the
    /// supercluster that made it. A cached filter without positions belongs to no supercluster, so
    /// no query reads them, and it has an empty table.
    sums: Sums,

    /// The positions of the points of the supercluster that made it, which `cache` sums, and
    /// which tell a query whether the cached filter belongs to its supercluster. The reference is weak,
    /// so a cached filter does not keep the positions after the supercluster is dropped.
    positions: Weak<Positions>,
}

impl CachedFilter {
    /// Create a cached filter from one bit per point, set when the point passes, and the positions
    /// of the points of its supercluster, or `None` when it was dropped.
    pub(crate) fn new(words: Vec<u64>, positions: Option<&Arc<Positions>>) -> Self {
        let mut ranks = Vec::with_capacity(words.len() + 1);
        let mut rank = 0;

        ranks.push(rank);

        for word in &words {
            rank += word.count_ones();
            ranks.push(rank);
        }

        let sums = match positions {
            Some(positions) => Sums::new(
                words.len(),
                |index| words[index],
                |leaf| positions.quantized(leaf),
            ),
            None => Sums::new(0, |_| 0, |_| [0, 0]),
        };

        CachedFilter {
            words,
            ranks,
            sums,
            positions: positions.map_or_else(Weak::new, Arc::downgrade),
        }
    }

    /// Check whether the cached filter belongs to the supercluster with the positions `positions`.
    /// The weak reference keeps the place of the positions it points to, so the positions of a new
    /// supercluster never take it, and a cached filter without positions belongs to none.
    fn belongs_to(&self, positions: &Positions) -> bool {
        ptr::eq(self.positions.as_ptr(), positions)
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

    /// Combine the points that pass both this cached filter and `other`: a cached filter, behind a
    /// reference, an `Arc` or a `Box`, or a combined filter, by value or by reference.
    ///
    /// Combining runs nothing: the combined filter borrows the cached filters, and a query
    /// evaluates it on the points of its tile only, so its cost grows with the points of the tile,
    /// not with all the points. Both cached filters must come from the supercluster that
    /// `with_filter` is called on, or it returns `SuperclusterError::ForeignCachedFilter`.
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
    /// // Cache once per attribute, then combine them for each request
    /// let fast = supercluster.cache_filter(|station| station.2 >= 50);
    /// let available = supercluster.cache_filter(|station| station.3);
    ///
    /// // The items borrow the cached filters, not the combined filter, so it can be a temporary
    /// let items = supercluster.with_filter(fast.and(&available))?.tile(0, 0, 0).items;
    ///
    /// assert_eq!(items[0].count(), 1);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn and<'s>(&'s self, other: impl Into<CombinedFilter<'s>>) -> CombinedFilter<'s> {
        CombinedFilter::from(self).and(other)
    }

    /// Combine the points that pass this cached filter, `other` or both, like `and`.
    pub fn or<'s>(&'s self, other: impl Into<CombinedFilter<'s>>) -> CombinedFilter<'s> {
        CombinedFilter::from(self).or(other)
    }

    /// Combine the points that pass this cached filter but not `other`, like `and`.
    pub fn and_not<'s>(&'s self, other: impl Into<CombinedFilter<'s>>) -> CombinedFilter<'s> {
        CombinedFilter::from(self).and_not(other)
    }

    /// Get the number of points of the supercluster that pass, which it counts once, when it is
    /// made.
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
    /// assert_eq!(supercluster.cache_filter(|point| point.2).count(), 2);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    #[must_use]
    pub fn count(&self) -> u32 {
        self.ranks[self.words.len()]
    }

    /// Count the points at the positions of `range` that pass, and find which ones, or get `None`
    /// if no point passes.
    fn matches(&self, range: Range<usize>) -> Option<(u32, Matches<'_>)> {
        let count = self.count_in(range.clone());

        Matches::found(count, range.len(), || Matches::Shared(self))
    }

    /// Count the points at the positions of `range` that pass.
    pub(crate) fn count_in(&self, range: Range<usize>) -> u32 {
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
impl Debug for CachedFilter {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CachedFilter")
            .field("passing", &self.count())
            .finish_non_exhaustive()
    }
}

/// A filter that combines cached filters with `and`, `or` and `and_not`, which each query works out
/// again, on the points of its tile only. It borrows the cached filters and runs nothing when it is
/// made.
///
/// Its cost grows with the points of the tile, not with all the points. Nothing is cached, so it
/// always reads the current cached filters. `cache` turns it into a `CachedFilter`, which a query
/// reads in constant time, to keep a combined filter that many queries share. It combines cached
/// filters only, because a `Filter` runs its closure on each query instead.
///
/// Its own `and`, `or` and `and_not` take it by value, so one expression combines any number of
/// cached filters, such as `a.or(&b).and(&c)`. To use a combined filter in another one and keep it,
/// pass a reference to it, which copies its steps.
#[derive(Clone)]
pub struct CombinedFilter<'s> {
    /// The steps that evaluate the combined filter, in postfix order.
    steps: Vec<Step<'s>>,
}

/// One step of the evaluation of a combined filter.
#[derive(Clone, Copy)]
enum Step<'s> {
    /// Take the bits of a cached filter.
    Take(&'s CachedFilter),

    /// Keep the bits that the last two have both.
    And,

    /// Keep the bits that either of the last two has.
    Or,

    /// Keep the bits of the second to last that the last one does not have.
    AndNot,
}

// A cached filter behind a reference, an `Arc`, a `Box` or an `Rc`
impl<'s, T: Borrow<CachedFilter> + ?Sized> From<&'s T> for CombinedFilter<'s> {
    fn from(cached: &'s T) -> Self {
        CombinedFilter {
            steps: vec![Step::Take(T::borrow(cached))],
        }
    }
}

// A combined filter that is reused, which copies its steps
impl<'s> From<&CombinedFilter<'s>> for CombinedFilter<'s> {
    fn from(combined: &CombinedFilter<'s>) -> Self {
        combined.clone()
    }
}

impl<'s> CombinedFilter<'s> {
    /// Combine the points that pass both this combined filter and `other`, like
    /// `CachedFilter::and`.
    #[must_use]
    pub fn and(self, other: impl Into<CombinedFilter<'s>>) -> CombinedFilter<'s> {
        self.combine(other.into(), Step::And)
    }

    /// Combine the points that pass this combined filter, `other` or both, like `CachedFilter::or`.
    #[must_use]
    pub fn or(self, other: impl Into<CombinedFilter<'s>>) -> CombinedFilter<'s> {
        self.combine(other.into(), Step::Or)
    }

    /// Combine the points that pass this combined filter but not `other`, like
    /// `CachedFilter::and_not`.
    #[must_use]
    pub fn and_not(self, other: impl Into<CombinedFilter<'s>>) -> CombinedFilter<'s> {
        self.combine(other.into(), Step::AndNot)
    }

    /// Evaluate the combined filter on every point into a `CachedFilter`, which a query reads in
    /// constant time, and which does not borrow the cached filters it combines, so a cache can keep
    /// it. It reads the positions of the points that pass, so a query finds the center of the
    /// points of a cluster that pass without reading them again. When the cached filters come from
    /// different superclusters, or their supercluster was dropped, `with_filter` rejects the result
    /// with `SuperclusterError::ForeignCachedFilter`.
    ///
    /// # Examples
    ///
    /// ```
    /// use supercluster::{CachedFilter, Supercluster, SuperclusterOptions};
    ///
    /// // A station is a longitude, a latitude, its power in kW and whether it is available
    /// let stations = vec![(10.0, 50.0, 150, true), (10.001, 50.0, 150, false), (10.002, 50.0, 11, true)];
    /// let supercluster = Supercluster::load(SuperclusterOptions::default(), stations, |station| {
    ///     Some([station.0, station.1])
    /// })?;
    /// let fast = supercluster.cache_filter(|station| station.2 >= 50);
    /// let available = supercluster.cache_filter(|station| station.3);
    ///
    /// // A cache can keep it, because it does not borrow `fast` and `available`
    /// let cached: CachedFilter = fast.and(&available).cache();
    ///
    /// assert_eq!(supercluster.with_filter(&cached)?.tile(0, 0, 0).items[0].count(), 1);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    #[must_use]
    pub fn cache(&self) -> CachedFilter {
        let words = self
            .steps
            .iter()
            .map(|step| match step {
                Step::Take(cached) => cached.words.len(),
                _ => 0,
            })
            .max()
            .unwrap_or_default();
        let mut scratch = Scratch::default();

        let words = mem::take(evaluate(&self.steps, 0..words, &mut scratch));

        // The sums are built now from the positions of the supercluster of the cached filters. When
        // the cached filters come from different superclusters, or theirs was dropped, the result
        // has no positions, so it belongs to no supercluster and `with_filter` rejects it
        let mut cached = self.cached_filters();
        let positions = cached
            .next()
            .filter(|first| cached.all(|other| other.positions.ptr_eq(&first.positions)))
            .and_then(|first| first.positions.upgrade());

        CachedFilter::new(words, positions.as_ref())
    }

    /// Iterate over the cached filters the combined filter combines.
    fn cached_filters(&self) -> impl Iterator<Item = &'s CachedFilter> + '_ {
        self.steps.iter().filter_map(|step| match step {
            Step::Take(cached) => Some(*cached),
            _ => None,
        })
    }

    /// Combine this combined filter with `other` in one more step.
    fn combine(mut self, other: CombinedFilter<'s>, step: Step<'s>) -> CombinedFilter<'s> {
        self.steps.extend(other.steps);
        self.steps.push(step);

        self
    }
}

// Written by hand, because a derive would print the cached filters it combines.
impl Debug for CombinedFilter<'_> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let cached_filters = self.cached_filters().count();

        formatter
            .debug_struct("CombinedFilter")
            .field("cached_filters", &cached_filters)
            .finish_non_exhaustive()
    }
}

/// A filter that runs a closure on the points of each query that gets it, which
/// `Supercluster::filter` makes.
///
/// It runs nothing when it is made and caches nothing, so every query reads the current data. A
/// query runs the closure on the points under its tile only, so a zoomed-out tile, which covers
/// the most points, costs the most. `Supercluster::cache_filter` runs a closure once on every point
/// instead, for many queries to reuse.
///
/// It holds only the closure, and a query runs it on its own points, so it works with every
/// supercluster of the same type of points. See `Supercluster::filter` for an example.
pub struct Filter<P, F> {
    /// The closure, which returns `true` for a point that passes.
    filter: F,

    /// The type of the points the closure takes, which the filter holds none of.
    points: PhantomData<fn(&P)>,
}

impl<P, F> Filter<P, F> {
    /// Create a filter that runs `filter` on the points of each query.
    pub(crate) fn new(filter: F) -> Self {
        Filter {
            filter,
            points: PhantomData,
        }
    }
}

// Written by hand, because a closure has nothing to print.
impl<P, F> Debug for Filter<P, F> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Filter").finish_non_exhaustive()
    }
}

/// What `with_filter` keeps the points of: a `Filter`, by value or by reference, a `CachedFilter`,
/// behind a reference, an `Arc` or a `Box`, or a `CombinedFilter`, by value or by reference. `'s`
/// is the lifetime of the cached filters, which the items of a tile borrow, and `P` is the type of
/// the points. Only this crate implements it.
pub trait AsFilter<'s, P>: private::Sealed<'s, P> {}

impl<'s, P, F: Fn(&P) -> bool> AsFilter<'s, P> for Filter<P, F> {}

impl<'s, P, F: Fn(&P) -> bool> AsFilter<'s, P> for &Filter<P, F> {}

impl<'s, P, T: Borrow<CachedFilter> + ?Sized> AsFilter<'s, P> for &'s T {}

impl<'s, P> AsFilter<'s, P> for CombinedFilter<'s> {}

impl<'s, P> AsFilter<'s, P> for &CombinedFilter<'s> {}

/// The trait that keeps other crates from implementing `AsFilter`, with what the queries need.
pub(crate) mod private {
    use std::ops::Range;

    use super::{Matches, Positions, Scratch};

    /// What a query needs from the filter it gets, whose cached filters live for `'s`, on points of
    /// the type `P`.
    pub trait Sealed<'s, P> {
        /// Count the points at the positions of `range` in `points`, the points of the
        /// supercluster of the query, that pass, and find which ones, or get `None` if no point
        /// passes. `scratch` holds buffers that a query reuses for every item.
        fn matches(
            &self,
            points: &[P],
            range: Range<usize>,
            scratch: &mut Scratch,
        ) -> Option<(u32, Matches<'s>)>;

        /// Check whether a point at the positions of `range` in `points` passes. `scratch` holds
        /// buffers that a query reuses for every check.
        fn any(&self, points: &[P], range: Range<usize>, scratch: &mut Scratch) -> bool;

        /// Check whether the filter belongs to the supercluster with the positions `positions`,
        /// which a query checks before it reads them.
        fn belongs_to(&self, positions: &Positions) -> bool;
    }
}

impl<'s, P, F: Fn(&P) -> bool> private::Sealed<'s, P> for Filter<P, F> {
    fn matches(
        &self,
        points: &[P],
        range: Range<usize>,
        scratch: &mut Scratch,
    ) -> Option<(u32, Matches<'s>)> {
        let first = range.start;

        // One buffer for the bits of every cluster, which an item takes only when some of its
        // points fail, so a cluster whose points all pass allocates nothing
        match &points[range] {
            // A point needs no bits
            [point] => (self.filter)(point).then_some((1, Matches::All)),
            points => {
                let bits = scratch.result();

                bits.clear();
                bits.extend(filter_bits(points, &self.filter));

                let count = bits.iter().map(|word| word.count_ones()).sum();

                Matches::found(count, points.len(), || Matches::Local {
                    words: mem::take(bits),
                    first,
                })
            }
        }
    }

    fn any(&self, points: &[P], range: Range<usize>, _: &mut Scratch) -> bool {
        points[range].iter().any(&self.filter)
    }

    // The closure runs on the points of the query, so it fits every supercluster
    fn belongs_to(&self, _: &Positions) -> bool {
        true
    }
}

impl<'s, P, F: Fn(&P) -> bool> private::Sealed<'s, P> for &Filter<P, F> {
    fn matches(
        &self,
        points: &[P],
        range: Range<usize>,
        scratch: &mut Scratch,
    ) -> Option<(u32, Matches<'s>)> {
        (**self).matches(points, range, scratch)
    }

    fn any(&self, points: &[P], range: Range<usize>, scratch: &mut Scratch) -> bool {
        (**self).any(points, range, scratch)
    }

    fn belongs_to(&self, positions: &Positions) -> bool {
        (**self).belongs_to(positions)
    }
}

impl<'s, P, T: Borrow<CachedFilter> + ?Sized> private::Sealed<'s, P> for &'s T {
    fn matches(&self, _: &[P], range: Range<usize>, _: &mut Scratch) -> Option<(u32, Matches<'s>)> {
        T::borrow(*self).matches(range)
    }

    fn any(&self, _: &[P], range: Range<usize>, _: &mut Scratch) -> bool {
        T::borrow(self).count_in(range) > 0
    }

    fn belongs_to(&self, positions: &Positions) -> bool {
        T::borrow(self).belongs_to(positions)
    }
}

impl<'s, P> private::Sealed<'s, P> for CombinedFilter<'s> {
    fn matches(
        &self,
        _: &[P],
        range: Range<usize>,
        scratch: &mut Scratch,
    ) -> Option<(u32, Matches<'s>)> {
        // A combined filter of a single cached filter reads the counts of the cached filter
        if let [Step::Take(cached)] = self.steps[..] {
            return cached.matches(range);
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

    fn any(&self, _: &[P], range: Range<usize>, scratch: &mut Scratch) -> bool {
        if let [Step::Take(cached)] = self.steps[..] {
            return cached.count_in(range) > 0;
        }

        evaluate_range(&self.steps, range, scratch).0 > 0
    }

    fn belongs_to(&self, positions: &Positions) -> bool {
        self.cached_filters()
            .all(|cached| cached.belongs_to(positions))
    }
}

impl<'s, P> private::Sealed<'s, P> for &CombinedFilter<'s> {
    fn matches(
        &self,
        points: &[P],
        range: Range<usize>,
        scratch: &mut Scratch,
    ) -> Option<(u32, Matches<'s>)> {
        <CombinedFilter<'s> as private::Sealed<'s, P>>::matches(self, points, range, scratch)
    }

    fn any(&self, points: &[P], range: Range<usize>, scratch: &mut Scratch) -> bool {
        <CombinedFilter<'s> as private::Sealed<'s, P>>::any(self, points, range, scratch)
    }

    fn belongs_to(&self, positions: &Positions) -> bool {
        <CombinedFilter<'s> as private::Sealed<'s, P>>::belongs_to(self, positions)
    }
}

/// Buffers to evaluate combined filters, which a query reuses for every item it evaluates.
#[derive(Default)]
pub struct Scratch {
    /// The words of every step that waits for the next one, the last one on top.
    stack: Vec<Vec<u64>>,
}

impl Scratch {
    /// Get the first buffer, which holds the words of a result.
    pub(crate) fn result(&mut self) -> &mut Vec<u64> {
        if self.stack.is_empty() {
            self.stack.push(vec![]);
        }

        &mut self.stack[0]
    }

    /// Take back the words of matches that an item does not keep, so the next item reuses them.
    pub(crate) fn give_back(&mut self, matches: Matches<'_>) {
        if let Matches::Local { words, .. } = matches {
            *self.result() = words;
        }
    }
}

/// Evaluate a combined filter on the words that hold the positions of `range`, from the word of its
/// first position, into the first buffer of `scratch`. Returns the number of points of `range`
/// that pass, and the position of the first bit of the words.
fn evaluate_range(steps: &[Step<'_>], range: Range<usize>, scratch: &mut Scratch) -> (u32, usize) {
    let words = range.start / 64..range.end.div_ceil(64);
    let first = words.start * 64;
    let count = count_set(evaluate(steps, words, scratch), first, range);

    (count, first)
}

/// Evaluate a combined filter on the words of `words`, and get the result, which is the first
/// buffer of `scratch`. A word that a cached filter lacks counts as no bit set, so computing a
/// combined filter of cached filters of different superclusters does not make it panic. Every step
/// works on a whole buffer of words, which the processor runs fast.
fn evaluate<'b>(
    steps: &[Step<'_>],
    words: Range<usize>,
    scratch: &'b mut Scratch,
) -> &'b mut Vec<u64> {
    // The number of buffers on the stack that hold words of a step
    let mut top = 0;
    let mut steps = steps.iter().peekable();

    while let Some(&step) = steps.next() {
        let Step::Take(cached) = step else {
            // An operation after two buffers combines the top one into the one below it
            top -= 1;

            let (lower, upper) = scratch.stack.split_at_mut(top);

            apply(step, &mut lower[top - 1], &upper[0]);

            continue;
        };

        let source = words_of(cached, &words);

        // An operation right after a cached filter combines its words straight into the buffer
        // below, without copying them into a buffer of their own first
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

/// Get the words of a cached filter at the positions `words`, which are fewer when the cached
/// filter ends before them.
fn words_of<'s>(cached: &'s CachedFilter, words: &Range<usize>) -> &'s [u64] {
    let end = words.end.min(cached.words.len());
    let start = words.start.min(end);

    &cached.words[start..end]
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
    // word, and counts the words between as they are. A combined filter counts the points of every
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

    /// The points of a cached filter pass.
    Shared(&'a CachedFilter),
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
            Matches::Shared(cached) => cached.first(range),
        }
    }

    /// Iterate over the positions in `range` of the points that pass, a word at a time, so a long
    /// run of points that fail costs one step per 64 of them.
    pub(crate) fn passing(&self, range: Range<usize>) -> impl Iterator<Item = usize> + '_ {
        // Every point passes: the inverted bits of no words are all set
        let (words, first, invert) = match self {
            Matches::All => (&[][..], range.start, true),
            Matches::Local { words, first } => (&words[..], *first, false),
            Matches::Shared(cached) => (&cached.words[..], 0, false),
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
            // A query only gets a cached filter of its own supercluster, whose sums are of these
            // positions
            Matches::Shared(cached) => cached.sums.sum(
                range,
                |index| cached.words[index],
                |leaf| positions.quantized(leaf),
            ),
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
            Matches::Shared(cached) => cached.count_in(range) > 0,
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
    mut filter: impl FnMut(&P) -> bool + 'p,
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

    use crate::{
        test_points::load_spread_points, Item, Supercluster, SuperclusterError, SuperclusterOptions,
    };

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
    /// expansion zoom that `FilteredView::cluster_expansion_zoom` finds for every cluster.
    type Summary = Vec<(i32, i32, u32, usize, Option<u8>)>;

    /// Summarize the items of the tile `z`/`x`/`y` that `cached filter` keeps.
    fn summarize<'s, S: AsFilter<'s, [f64; 2]> + Copy>(
        supercluster: &'s Supercluster<[f64; 2]>,
        (z, x, y): (u8, u32, u32),
        cached_filter: S,
    ) -> Summary {
        supercluster
            .with_filter(cached_filter)
            .unwrap()
            .tile(z, x, y)
            .items
            .iter()
            .map(|item| {
                let expansion_zoom = match item.item() {
                    Item::Cluster(cluster) => Some(
                        supercluster
                            .with_filter(cached_filter)
                            .unwrap()
                            .cluster_expansion_zoom(cluster.id())
                            .unwrap(),
                    ),
                    Item::Point { .. } => None,
                };

                assert_eq!(item.expansion_zoom(), expansion_zoom);

                (
                    item.x(),
                    item.y(),
                    item.count(),
                    item.leaves().count(),
                    expansion_zoom,
                )
            })
            .collect()
    }

    /// Check that a combined filter gives the results of a cached filter, evaluated by the queries
    /// and cached_combination: the same number of points, and the same items in every tile up to
    /// zoom level 4.
    fn assert_same_results(
        supercluster: &Supercluster<[f64; 2]>,
        combined: &CombinedFilter<'_>,
        cached_filter: &CachedFilter,
    ) {
        let cached_combined = combined.cache();

        assert_eq!(format!("{cached_combined:?}"), format!("{cached_filter:?}"));

        for z in 0..=4 {
            for x in 0..1 << z {
                for y in 0..1 << z {
                    let expected = summarize(supercluster, (z, x, y), cached_filter);

                    assert_eq!(
                        summarize(supercluster, (z, x, y), combined),
                        expected,
                        "tile {z}/{x}/{y}"
                    );
                    assert_eq!(
                        summarize(supercluster, (z, x, y), &cached_combined),
                        expected,
                        "tile {z}/{x}/{y}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_debug_of_a_cached_filter() {
        // 130 points, so the bits spread over three words
        let points: Vec<[f64; 2]> = (0..130).map(|i| [f64::from(i), 0.0]).collect();
        let supercluster =
            Supercluster::load(SuperclusterOptions::default(), points, |point| Some(*point))
                .unwrap();
        let cached_filter = supercluster.cache_filter(|point| point[0] % 3.0 == 0.0);

        // The output counts the points that pass instead of printing every bit
        assert_eq!(
            format!("{cached_filter:?}"),
            "CachedFilter { passing: 44, .. }"
        );
    }

    #[test]
    fn test_and_matches_cache_filter_with_both_filters() {
        let supercluster = load_spread_points(500);
        let (third, northern) = (
            supercluster.cache_filter(every_third),
            supercluster.cache_filter(north),
        );
        let selected = supercluster.cache_filter(|point| every_third(point) && north(point));

        assert_same_results(&supercluster, &third.and(&northern), &selected);
    }

    #[test]
    fn test_or_matches_cache_filter_with_either_filter() {
        let supercluster = load_spread_points(500);
        let (third, northern) = (
            supercluster.cache_filter(every_third),
            supercluster.cache_filter(north),
        );
        let selected = supercluster.cache_filter(|point| every_third(point) || north(point));

        assert_same_results(&supercluster, &third.or(&northern), &selected);
    }

    #[test]
    fn test_and_not_matches_cache_filter_with_the_first_filter_only() {
        let supercluster = load_spread_points(500);
        let (third, northern) = (
            supercluster.cache_filter(every_third),
            supercluster.cache_filter(north),
        );
        let selected = supercluster.cache_filter(|point| every_third(point) && !north(point));

        assert_same_results(&supercluster, &third.and_not(&northern), &selected);
    }

    #[test]
    fn test_and_in_one_expression() {
        let supercluster = load_spread_points(500);
        let (third, northern, eastern) = (
            supercluster.cache_filter(every_third),
            supercluster.cache_filter(north),
            supercluster.cache_filter(east),
        );

        // A combined filter takes the next cached filter by value, so a chain needs no variables
        let combined = third.or(&northern).and(&eastern);
        let selected =
            supercluster.cache_filter(|point| (every_third(point) || north(point)) && east(point));

        assert_same_results(&supercluster, &combined, &selected);
    }

    #[test]
    fn test_and_of_a_combined_filter() {
        let supercluster = load_spread_points(500);
        let (third, northern, eastern) = (
            supercluster.cache_filter(every_third),
            supercluster.cache_filter(north),
            supercluster.cache_filter(east),
        );
        let combined = third.and(northern.and_not(&eastern));
        let selected =
            supercluster.cache_filter(|point| every_third(point) && north(point) && !east(point));

        assert_same_results(&supercluster, &combined, &selected);
    }

    #[test]
    fn test_and_with_a_borrowed_combined_filter() {
        let supercluster = load_spread_points(500);
        let (third, northern, eastern) = (
            supercluster.cache_filter(every_third),
            supercluster.cache_filter(north),
            supercluster.cache_filter(east),
        );
        let either = third.or(&northern);

        // A borrowed combined filter is copied into the new one, so it stays usable
        let combined = eastern.and(&either);
        let selected =
            supercluster.cache_filter(|point| east(point) && (every_third(point) || north(point)));

        assert_same_results(&supercluster, &combined, &selected);
        assert_eq!(
            format!("{either:?}"),
            "CombinedFilter { cached_filters: 2, .. }"
        );
    }

    #[test]
    fn test_filtered_tile_with_a_temporary_combined_filter() {
        let supercluster = load_spread_points(500);
        let (third, northern) = (
            supercluster.cache_filter(every_third),
            supercluster.cache_filter(north),
        );
        let selected = supercluster.cache_filter(|point| every_third(point) && north(point));

        // The items borrow the cached filters, not the view or the combined filter, so they outlive
        // both
        let items = supercluster
            .with_filter(third.and(&northern))
            .unwrap()
            .tile(0, 0, 0)
            .items;
        let expected = supercluster
            .with_filter(&selected)
            .unwrap()
            .tile(0, 0, 0)
            .items;
        let summary = |items: &[crate::TileItem<'_, [f64; 2]>]| -> Vec<(i32, i32, u32)> {
            items
                .iter()
                .map(|item| (item.x(), item.y(), item.count()))
                .collect()
        };

        assert!(!items.is_empty());
        assert_eq!(summary(&items), summary(&expected));
    }

    #[test]
    fn test_filtered_tile_with_cached_filters_behind_an_arc_or_a_box() {
        let supercluster = load_spread_points(500);
        let third = Arc::new(supercluster.cache_filter(every_third));
        let northern = Box::new(supercluster.cache_filter(north));
        let selected = supercluster.cache_filter(|point| every_third(point) && north(point));

        // The queries and the operators take them as they are
        assert_eq!(
            summarize(&supercluster, (1, 0, 0), &third),
            summarize(&supercluster, (1, 0, 0), third.as_ref())
        );
        assert_same_results(&supercluster, &third.and(&northern), &selected);
    }

    #[test]
    fn test_cache_filter_does_not_keep_the_positions_of_a_dropped_supercluster() {
        let supercluster = load_spread_points(500);
        let cached_filter = supercluster.cache_filter(every_third);
        let positions = Arc::downgrade(supercluster.positions());

        drop(supercluster);

        // The positions are freed, and computing a combined filter still works
        assert!(positions.upgrade().is_none());

        let cached_combined = cached_filter.and(&cached_filter).cache();

        assert_eq!(format!("{cached_combined:?}"), format!("{cached_filter:?}"));

        // A supercluster with the same points loaded again is another one, which rejects both
        let reloaded = load_spread_points(500);

        for cached_filter in [&cached_filter, &cached_combined] {
            assert!(matches!(
                reloaded.with_filter(cached_filter),
                Err(SuperclusterError::ForeignCachedFilter)
            ));
        }
    }

    #[test]
    fn test_filtered_tile_with_a_cached_filter_of_a_clone() {
        let supercluster = load_spread_points(500);
        let cached_filter = supercluster.cache_filter(every_third);
        let clone = supercluster.clone();

        // A clone shares the positions, so it accepts the cached filters of the original
        assert_eq!(
            summarize(&clone, (0, 0, 0), &cached_filter),
            summarize(&supercluster, (0, 0, 0), &cached_filter)
        );
    }

    #[test]
    fn test_cache_of_a_single_cached_filter() {
        let supercluster = load_spread_points(130);
        let cached_filter = supercluster.cache_filter(every_third);

        assert_same_results(
            &supercluster,
            &CombinedFilter::from(&cached_filter),
            &cached_filter,
        );
    }

    #[test]
    fn test_debug_of_a_combined_filter() {
        let supercluster = load_spread_points(10);
        let (third, northern, all) = (
            supercluster.cache_filter(every_third),
            supercluster.cache_filter(north),
            supercluster.cache_filter(|_| true),
        );

        // A combined filter computes nothing, so it tells how many cached filters it combines
        assert_eq!(
            format!("{:?}", third.or(&northern).and(&all)),
            "CombinedFilter { cached_filters: 3, .. }"
        );
    }

    #[test]
    fn test_and_with_a_cached_filter_of_another_supercluster() {
        let small = load_spread_points(10);
        let large = load_spread_points(500);
        let small_cached_filter = small.cache_filter(|_| true);
        let large_cached_filter = large.cache_filter(|_| true);

        // The words that the smaller cached filter lacks count as no bit set
        assert_eq!(
            format!(
                "{:?}",
                large_cached_filter.and(&small_cached_filter).cache()
            ),
            "CachedFilter { passing: 10, .. }"
        );
        assert_eq!(
            format!(
                "{:?}",
                large_cached_filter.and_not(&small_cached_filter).cache()
            ),
            "CachedFilter { passing: 490, .. }"
        );

        // Computing does not panic, and the result belongs to no supercluster, so every query
        // rejects it, as it rejects the combined filter itself
        let cached_combined = small_cached_filter.or(&large_cached_filter).cache();

        for supercluster in [&small, &large] {
            assert!(matches!(
                supercluster.with_filter(&cached_combined),
                Err(SuperclusterError::ForeignCachedFilter)
            ));
            assert!(matches!(
                supercluster.with_filter(small_cached_filter.and(&large_cached_filter)),
                Err(SuperclusterError::ForeignCachedFilter)
            ));
        }
    }

    #[test]
    fn test_and_without_points() {
        let supercluster = load_spread_points(0);
        let cached_filter = supercluster.cache_filter(|_| true);
        let combined = cached_filter.and(&cached_filter);

        assert_eq!(
            format!("{:?}", combined.cache()),
            "CachedFilter { passing: 0, .. }"
        );
        assert!(supercluster
            .with_filter(&combined)
            .unwrap()
            .tile(0, 0, 0)
            .items
            .is_empty());
    }
}
