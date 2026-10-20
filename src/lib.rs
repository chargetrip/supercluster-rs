#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::missing_docs_in_private_items)]
#![doc = include_str!("../README.md")]

/// Supercluster builder module.
/// This module contains the builder pattern for the supercluster configuration settings.
pub mod builder;

/// Supercluster error module.
/// This module contains the error types for the supercluster crate.
pub mod error;

/// GeoJSON feature module.
/// This module contains the GeoJSON support of the supercluster crate.
/// It is available with the `geojson` feature.
#[cfg(feature = "geojson")]
pub mod feature;

/// KDBush module.
/// This module contains the KDBush implementation the supercluster uses to find points.
mod kdbush;

/// Range module.
/// This module contains the range implementation for the supercluster crate.
pub mod range;

/// Supercluster module.
/// This module contains the supercluster implementation for the supercluster crate.
pub mod supercluster;

/// Tile builder module.
/// This module contains the tile builder, which filters and maps the items of a tile.
pub mod tile_builder;

pub use builder::*;
pub use error::*;
#[cfg(feature = "geojson")]
pub use feature::*;
pub use range::*;
pub use supercluster::*;
pub use tile_builder::*;
