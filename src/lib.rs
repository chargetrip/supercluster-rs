#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::missing_docs_in_private_items)]
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

/// Filtered tile module.
/// This module contains the queries that build a tile from the points that pass a filter.
mod filtered_tile;

/// Selection module.
/// This module contains the selection of the points that pass a filter, which queries reuse.
mod selection;

/// Test points module.
/// This module contains the points that the tests of several modules share.
#[cfg(test)]
mod test_points;

pub use builder::*;
pub use error::*;
#[cfg(feature = "geojson")]
pub use feature::*;
pub use filtered_tile::*;
pub use range::*;
pub use selection::{Combination, Select, Selection};
pub use supercluster::*;
