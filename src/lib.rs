#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::missing_docs_in_private_items)]
#![warn(clippy::must_use_candidate)]
#![doc = include_str!("../README.md")]

/// Supercluster builder module.
/// This module contains the builder pattern for the supercluster configuration settings.
mod builder;

/// Supercluster error module.
/// This module contains the error types for the supercluster crate.
mod error;

/// GeoJSON feature module.
/// This module contains the GeoJSON support of the supercluster crate.
/// It is available with the `geojson` feature.
#[cfg(feature = "geojson")]
mod feature;

/// KDBush module.
/// This module contains the KDBush implementation the supercluster uses to find points.
mod kdbush;

/// Range module.
/// This module contains the range implementation for the supercluster crate.
mod range;

/// Supercluster module.
/// This module contains the supercluster implementation for the supercluster crate.
mod supercluster;

/// Tile module.
/// This module contains the items of a tile, the queries that build a tile, and the view whose
/// queries keep only the points that pass a filter.
mod tile;

/// Filter module.
/// This module contains the filters that `with_filter` takes: a closure that runs on each
/// query, the cached result of a closure, which queries reuse, and combined filters of those.
mod filter;

/// Sums module.
/// This module contains the running sums of positions that find the center of filtered clusters.
mod sums;

/// Test points module.
/// This module contains the points that the tests of several modules share.
#[cfg(test)]
mod test_points;

pub use builder::*;
pub use error::*;
#[cfg(feature = "geojson")]
pub use feature::*;
pub use filter::{AsFilter, CachedFilter, CombinedFilter, Filter};
pub use range::*;
pub use supercluster::*;
pub use tile::*;
