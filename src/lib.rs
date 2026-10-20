#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::missing_docs_in_private_items)]

//! # Supercluster
//!
//! A high-performance Rust crate for geospatial and non-geospatial point clustering.
//!
//!
//! ## Reference implementation
//! [![test](https://github.com/chargetrip/supercluster-rs/actions/workflows/test.yml/badge.svg)](https://github.com/chargetrip/supercluster-rs/actions/workflows/test.yml)
//! [![docs](https://docs.rs/supercluster/badge.svg)](https://docs.rs/supercluster)
//! [![crate](https://img.shields.io/crates/v/supercluster.svg)](https://crates.io/crates/supercluster)
//! ![downloads](https://img.shields.io/crates/d/supercluster)
//! ![GitHub](https://img.shields.io/github/license/chargetrip/supercluster-rs)
//! [![codecov](https://codecov.io/gh/chargetrip/supercluster-rs/graph/badge.svg?token=0S31CZY2ZJ)](https://codecov.io/gh/chargetrip/supercluster-rs)
//!
//! ![Features](https://cloud.githubusercontent.com/assets/25395/11857351/43407b46-a40c-11e5-8662-e99ab1cd2cb7.gif)
//!
//! ## Documentation
//!
//! If you encounter any issues or have questions that are not addressed in the documentation, feel free to [submit an issue](https://github.com/chargetrip/supercluster-rs/issues).
//! This crate was initially inspired by Mapbox's supercluster [blog post](https://blog.mapbox.com/clustering-millions-of-points-on-a-map-with-supercluster-272046ec5c97).
//!
//! ## Usage
//!
//! To use the `supercluster` crate in your project, add it to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! supercluster = "4.0.0"
//! ```
//!
//! ## Features
//!
//! - `geojson`: support for [GeoJSON](https://docs.rs/geojson) features, see the `feature` module.
//! - `serde`: serialization of the options and of a loaded supercluster.
//! - `tracing`: reports through the [`tracing`](https://docs.rs/tracing) crate. A load reports a
//!   summary at the `debug` level, one event per zoom level at the `trace` level, and a warning for
//!   points left out because their coordinates are missing or not finite. The events have fields,
//!   such as the number of points. Queries report nothing. Applications that only use the `log`
//!   crate get the same events as `log` records.
//!
//! ## Points
//!
//! A [`Supercluster`] clusters any type and keeps the points as you load them, without copying. A
//! closure tells where each point is, so you do not convert or wrap them, and shared data that
//! changes stays shared.
//!
//! ```rust
//! use supercluster::{Item, Supercluster, SuperclusterBuilder, SuperclusterError};
//!
//! fn main() -> Result<(), SuperclusterError> {
//!     // Set the configuration settings
//!     let options = SuperclusterBuilder::new()
//!         .radius(40.0)
//!         .extent(512.0)
//!         .min_points(2)
//!         .max_zoom(16)
//!         .build();
//!
//!     // Create a new instance with the specified configuration settings
//!     let mut supercluster = Supercluster::new(options);
//!
//!     // Load the longitude and latitude of the points into the supercluster
//!     supercluster.load_with(
//!         vec![[-77.032, 38.913], [-77.033, 38.913], [0.0, 0.0]],
//!         |point| Some(*point),
//!     )?;
//!
//!     // Get the clusters and the points that are not clustered of a tile
//!     let tile = supercluster.get_tile(0, 0.0, 0.0);
//!
//!     assert_eq!(tile.items.len(), 2);
//!
//!     for tile_item in &tile.items {
//!         match &tile_item.item {
//!             Item::Cluster(cluster) => println!("{} points at {}, {}", cluster.point_count, tile_item.x, tile_item.y),
//!             Item::Point { index, .. } => println!("point {} at {}, {}", index, tile_item.x, tile_item.y),
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! Your own types work the same way. The closure returns `None` for a point that has no
//! coordinates, which leaves it out. A point whose coordinates are not finite numbers is left out
//! too.
//!
//! ```rust
//! use supercluster::{Item, Supercluster, SuperclusterBuilder};
//!
//! struct Station {
//!     name: String,
//!     lng: f64,
//!     lat: f64,
//! }
//!
//! let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build());
//!
//! supercluster
//!     .load_with(
//!         vec![
//!             Station { name: "A".to_string(), lng: 4.9, lat: 52.37 },
//!             Station { name: "B".to_string(), lng: 4.91, lat: 52.37 },
//!         ],
//!         |station| Some([station.lng, station.lat]),
//!     )
//!     .unwrap();
//!
//! // The two stations are close, so the tile holds a cluster of them
//! let tile = supercluster.get_tile(0, 0.0, 0.0);
//! let Item::Cluster(cluster) = &tile.items[0].item else {
//!     panic!("expected a cluster");
//! };
//!
//! // The stations of the cluster, as they were loaded
//! let mut names: Vec<&str> = supercluster
//!     .get_leaves(cluster.id, 10, 0)
//!     .unwrap()
//!     .iter()
//!     .map(|station| station.name.as_str())
//!     .collect();
//!
//! names.sort();
//!
//! assert_eq!(names, ["A", "B"]);
//! ```
//!
//! ## Filtering and mapping tiles
//!
//! The [`TileBuilder`] trait builds the items of a tile while it filters the points inside the
//! clusters, for example the ones that match a search, and maps every cluster or point to the type
//! of your response.
//!
//! ## Benchmarks
//!
//! We use the `criterion` crate to benchmark the performance of the `supercluster` crate.
//!
//! Benchmarks help us understand the performance characteristics of supercluster and identify areas for optimization.
//!
//! We have several benchmark scenarios to test different aspects of supercluster:
//!
//! - **Getting a Tile**: Tests the performance of retrieving a tile from the `Supercluster`.
//! - **Getting Clusters**: Tests the performance of retrieving clusters for a given bounding box and zoom level.
//! - **Loading Points**: Tests the performance of loading points into the `Supercluster`.
//!
//! For more detailed benchmark scenarios, please refer to the [`benches`](https://github.com/chargetrip/supercluster-rs/tree/main/benches) directory in the repository.
//!
//! ## Safety
//!
//! This crate uses `#![forbid(unsafe_code)]` to ensure everything is implemented in 100% safe Rust.
//!
//! ## Contributing
//!
//! 🎈 Thanks for your help improving the project! We are so happy to have you!
//!
//! We have a [contributing guide](https://github.com/chargetrip/supercluster-rs/blob/main/CONTRIBUTING.md) to help you get involved in the project.
//!
//! ## Sponsors
//!
//! <a href="https://www.chargetrip.com" target="_blank">
//! <img src="https://dka575ofm4ao0.cloudfront.net/pages-transactional_logos/retina/149188/Chargetrip_Combined_-_Black.png" width="240" alt="Chargetrip">
//! </a>

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
