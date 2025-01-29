# Supercluster

A high-performance Rust crate for geospatial and non-geospatial point clustering.

This crate was initially inspired by Mapbox's supercluster [blog post](https://blog.mapbox.com/clustering-millions-of-points-on-a-map-with-supercluster-272046ec5c97).

## Reference implementation

[![test](https://github.com/chargetrip/supercluster-rs/actions/workflows/test.yml/badge.svg)](https://github.com/chargetrip/supercluster-rs/actions/workflows/test.yml)
[![docs](https://docs.rs/supercluster/badge.svg)](https://docs.rs/supercluster)
[![crate](https://img.shields.io/crates/v/supercluster.svg)](https://crates.io/crates/supercluster)
![downloads](https://img.shields.io/crates/d/supercluster)
![GitHub](https://img.shields.io/github/license/chargetrip/supercluster-rs)
[![codecov](https://codecov.io/gh/chargetrip/supercluster-rs/graph/badge.svg?token=0S31CZY2ZJ)](https://codecov.io/gh/chargetrip/supercluster-rs)

![Features](https://cloud.githubusercontent.com/assets/25395/11857351/43407b46-a40c-11e5-8662-e99ab1cd2cb7.gif)

## Documentation

For more in-depth details, please refer to the full [documentation](https://docs.rs/supercluster).

If you encounter any issues or have questions that are not addressed in the documentation, feel free to [submit an issue](https://github.com/chargetrip/supercluster-rs/issues).

## Usage

To use the `supercluster` crate in your project, add it to your `Cargo.toml`:

```toml
[dependencies]
supercluster = "x.x.x"
```

Below is an example of how to create and run a supercluster using the crate.
This example demonstrates how to build supercluster options, create a new supercluster, and get a tile.
For more detailed information and advanced usage, please refer to the full [documentation](https://docs.rs/supercluster).

```rust
use supercluster::{ CoordinateSystem, Supercluster, SuperclusterError };

fn main() -> Result<(), SuperclusterError> {
    // Set the configuration settings
    let options = Supercluster::builder()
        .radius(40.0)
        .extent(512.0)
        .min_points(2)
        .max_zoom(16)
        .coordinate_system(CoordinateSystem::LatLng)
        .build();

    // Create a new instance with the specified configuration settings
    let mut cluster = Supercluster::new(options);

    // Create a FeatureCollection Object
    // [GeoJSON Format Specification § 5](https://tools.ietf.org/html/rfc7946#section-5)
    let features = Supercluster::feature_builder()
        .add_point(vec![0.0, 0.0])
        .build();

    // Load a FeatureCollection Object into the Supercluster instance
    let index = cluster.load(features)?;

    index.get_tile(0, 0.0, 0.0)?;

    Ok(())
}
```

## Features

- `load(points)`: Loads a [FeatureCollection](https://datatracker.ietf.org/doc/html/rfc7946#section-3.3) Object. Each feature should be a [Feature Object](https://datatracker.ietf.org/doc/html/rfc7946#section-3.2).

- `get_clusters(bbox, zoom)`: For the given `bbox` array (`[west_lng, south_lat, east_lng, north_lat]`) and `zoom`, returns an array of clusters and points as [Feature Object](https://datatracker.ietf.org/doc/html/rfc7946#section-3.2) objects.

- `get_tile(z, x, y)`: For a given zoom and x/y coordinates, returns a [FeatureCollection](https://datatracker.ietf.org/doc/html/rfc7946#section-3.3) Object.

- `get_children(cluster_id)`: Returns the children of a cluster (on the next zoom level) given its id (`cluster_id` value from feature properties).

- `get_leaves(cluster_id, limit, offset)`: Returns all the points of a cluster (given its `cluster_id`), with pagination support.

- `get_cluster_expansion_zoom(cluster_id)`: Returns the zoom on which the cluster expands into several children (useful for "click to zoom" feature) given the cluster's `cluster_id`.

## Safety

This crate uses `#![forbid(unsafe_code)]` to ensure everything is implemented in 100% safe Rust.

## Contributing

Contributions from the community are always welcome! Here are some ways you can contribute:

### Reporting Bugs

If you encounter any bugs, please [submit an issue](https://github.com/chargetrip/supercluster-rs/issues) with detailed information about the problem and steps to reproduce it.

### Feature Requests

If you have ideas for new features, feel free to [submit an issue](https://github.com/chargetrip/supercluster-rs/issues) with a detailed description of the feature and its potential use cases.

### Build

To build the project, run:

```bash
cargo build
```

### Test

To run the tests, use:

```bash
cargo test
```

### Lint

Run [clippy](https://github.com/rust-lang/rust-clippy) to lint the code:

```bash
cargo clippy --all-targets --all-features --no-deps -- -D warnings
```

### Format

Run [rustfmt](https://github.com/rust-lang/rustfmt) to format the code:

```bash
cargo fmt
```

### Documentation

Generate documentation in HTML format:

```bash
cargo doc --open
```

## Sponsors

[![Chargetrip logo](https://chargetrip-files.s3.eu-central-1.amazonaws.com/logo-1.png)](https://www.chargetrip.com)
