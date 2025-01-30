#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::missing_docs_in_private_items)]

//! # Supercluster
//!
//! A high-performance Rust crate for geospatial and non-geospatial point clustering.
//!
//! ## Documentation
//!
//! For more in-depth details, please refer to the full [documentation](https://docs.rs/supercluster).
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
//! supercluster = "x.x.x"
//! ```
//!
//! Below is an example of how to create and run a supercluster using the crate.
//! This example demonstrates how to build supercluster options, create a new supercluster, and get a tile.
//! For more detailed information and advanced usage, please refer to the full [documentation](https://docs.rs/supercluster).
//!
//! ```rust
//! use supercluster::{ CoordinateSystem, Supercluster, SuperclusterError };
//!
//! fn main() -> Result<(), SuperclusterError> {
//!     // Set the configuration settings
//!     let options = Supercluster::builder()
//!         .radius(40.0)
//!         .extent(512.0)
//!         .min_points(2)
//!         .max_zoom(16)
//!         .coordinate_system(CoordinateSystem::LatLng)
//!         .build();
//!
//!     // Create a new instance with the specified configuration settings
//!     let mut cluster = Supercluster::new(options);
//!
//!     // Create a FeatureCollection Object
//!     // [GeoJSON Format Specification § 5](https://tools.ietf.org/html/rfc7946#section-5)
//!     let features = Supercluster::feature_builder()
//!         .add_point(vec![0.0, 0.0])
//!         .build();
//!
//!     // Load a FeatureCollection Object into the Supercluster instance
//!     let index = cluster.load(features)?;
//!
//!     if let Err(err) = index.get_tile(0, 0.0, 0.0) {
//!        println!("Error: {}", err);
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Features
//!
//! - `load(points)`: Loads a [FeatureCollection](https://datatracker.ietf.org/doc/html/rfc7946#section-3.3) Object. Each feature should be a [Feature Object](https://datatracker.ietf.org/doc/html/rfc7946#section-3.2).
//! - `get_clusters(bbox, zoom)`: For the given `bbox` array (`[west_lng, south_lat, east_lng, north_lat]`) and `zoom`, returns an array of clusters and points as [Feature Object](https://datatracker.ietf.org/doc/html/rfc7946#section-3.2) objects.
//! - `get_tile(z, x, y)`: For a given zoom and x/y coordinates, returns a [FeatureCollection](https://datatracker.ietf.org/doc/html/rfc7946#section-3.3) Object.
//! - `get_children(cluster_id)`: Returns the children of a cluster (on the next zoom level) given its id (`cluster_id` value from feature properties).
//! - `get_leaves(cluster_id, limit, offset)`: Returns all the points of a cluster (given its `cluster_id`), with pagination support.
//! - `get_cluster_expansion_zoom(cluster_id)`: Returns the zoom on which the cluster expands into several children (useful for "click to zoom" feature) given the cluster's `cluster_id`.
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

/// KDBush module.
/// This module contains the KDBush implementation for the supercluster crate.
pub mod kdbush;

/// Range module.
/// This module contains the range implementation for the supercluster crate.
pub mod range;

/// Supercluster module.
/// This module contains the supercluster implementation for the supercluster crate.
pub mod supercluster;

pub use builder::*;
pub use error::*;
pub use kdbush::*;
pub use range::*;
pub use supercluster::*;
