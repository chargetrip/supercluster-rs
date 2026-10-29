//! # Test points module
//!
//! Contains the points that the tests of several modules share.

use crate::{Supercluster, SuperclusterOptions};

/// Create points spread over the world with a fixed formula, so every run uses the same points.
pub(crate) fn create_spread_points(count: usize) -> Vec<[f64; 2]> {
    (0..count)
        .map(|i| {
            let lng = (i as f64 * 0.618_033_988_749_895) % 1.0;
            let lat = (i as f64 * 0.754_877_666_246_693) % 1.0;

            [-180.0 + 360.0 * lng, -80.0 + 160.0 * lat]
        })
        .collect()
}

/// Load the points spread over the world with the default options.
pub(crate) fn load_spread_points(count: usize) -> Supercluster<[f64; 2]> {
    Supercluster::load(
        SuperclusterOptions::default(),
        create_spread_points(count),
        |point| Some(*point),
    )
    .unwrap()
}
