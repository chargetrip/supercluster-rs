# Supercluster

A high-performance Rust crate for geospatial and non-geospatial point clustering.

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

This crate was initially inspired by Mapbox's supercluster [blog post](https://blog.mapbox.com/clustering-millions-of-points-on-a-map-with-supercluster-272046ec5c97).

## Usage

To use the `supercluster` crate in your project, add it to your `Cargo.toml`:

```toml
[dependencies]
supercluster = "2.1.0"
```

You can also include additional features, such as logging, by specifying them in your `Cargo.toml`:

```toml
[dependencies]
supercluster = { version = "2.1.0", features = ["logger"] }
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

    // Create a a list of features
    let features = Supercluster::feature_builder()
        .add_point(vec![0.0, 0.0])
        .build();

    // Load a list of features into the supercluster
    let index = cluster.load(features)?;

    index.get_tile(0, 0.0, 0.0)?;

    Ok(())
}
```

## Benchmarks

We use the `criterion` crate to benchmark the performance of the `supercluster` crate.

Benchmarks help us understand the performance characteristics of supercluster and identify areas for optimization.

We have several benchmark scenarios to test different aspects of supercluster:

- **Loading a Feature Collection**: Tests the performance of loading a `FeatureCollection` into the `Supercluster`.
- **Getting a Tile**: Tests the performance of retrieving a tile from the `Supercluster`.
- **Getting Clusters**: Tests the performance of retrieving clusters for a given bounding box and zoom level.

For more detailed benchmark scenarios, please refer to the [`benches`](https://github.com/chargetrip/supercluster-rs/tree/main/benches) directory in the repository.

## Safety

This crate uses `#![forbid(unsafe_code)]` to ensure everything is implemented in 100% safe Rust.

## Contributing

🎈 Thanks for your help improving the project! We are so happy to have you!

We have a [contributing guide](https://github.com/chargetrip/supercluster-rs/blob/main/CONTRIBUTING.md) to help you get involved in the project.

## Sponsors

<a href="https://www.chargetrip.com" target="_blank">
    <img src="https://dka575ofm4ao0.cloudfront.net/pages-transactional_logos/retina/149188/Chargetrip_Combined_-_Black.png" width="240" alt="Chargetrip">
</a>
