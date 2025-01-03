# Supercluster

A high-performance Rust crate for geospatial and non-geospatial point clustering.

This crate is inspired by Mapbox's supercluster [blog post](https://blog.mapbox.com/clustering-millions-of-points-on-a-map-with-supercluster-272046ec5c97).

## Reference implementation

[![test](https://github.com/chargetrip/supercluster-rs/actions/workflows/test.yml/badge.svg)](https://github.com/chargetrip/supercluster-rs/actions/workflows/test.yml)
[![docs](https://docs.rs/supercluster/badge.svg)](https://docs.rs/supercluster)
[![crate](https://img.shields.io/crates/v/supercluster.svg)](https://crates.io/crates/supercluster)
![downloads](https://img.shields.io/crates/d/supercluster)
![GitHub](https://img.shields.io/github/license/chargetrip/supercluster-rs)

![Features](https://cloud.githubusercontent.com/assets/25395/11857351/43407b46-a40c-11e5-8662-e99ab1cd2cb7.gif)

## Features

- `load(points)`: Loads a [FeatureCollection](https://datatracker.ietf.org/doc/html/rfc7946#section-3.3) Object. Each feature should be a [Feature Object](https://datatracker.ietf.org/doc/html/rfc7946#section-3.2).

- `get_clusters(bbox, zoom)`: For the given `bbox` array (`[west_lng, south_lat, east_lng, north_lat]`) and `zoom`, returns an array of clusters and points as [Feature Object](https://datatracker.ietf.org/doc/html/rfc7946#section-3.2) objects.

- `get_tile(z, x, y)`: For a given zoom and x/y coordinates, returns a [FeatureCollection](https://datatracker.ietf.org/doc/html/rfc7946#section-3.3) Object.

- `get_children(cluster_id)`: Returns the children of a cluster (on the next zoom level) given its id (`cluster_id` value from feature properties).

- `get_leaves(cluster_id, limit, offset)`: Returns all the points of a cluster (given its `cluster_id`), with pagination support.

- `get_cluster_expansion_zoom(cluster_id)`: Returns the zoom on which the cluster expands into several children (useful for "click to zoom" feature) given the cluster's `cluster_id`.

## Options

| Option              | Description                                                       |
|---------------------|-------------------------------------------------------------------|
| `min_zoom`          | Minimum zoom level at which clusters are generated.               |
| `max_zoom`          | Maximum zoom level at which clusters are generated.               |
| `min_points`        | Minimum number of points to form a cluster.                       |
| `radius`            | Cluster radius, in pixels.                                        |
| `extent`            | (Tiles) Tile extent. Radius is calculated relative to this value. |
| `node_size`         | Size of the KD-tree leaf node. Affects performance.               |
| `coordinate_system` | Type of coordinate system for clustering.                         |

## Safety

This crate uses `#![forbid(unsafe_code)]` to ensure everything is implemented in 100% safe Rust.

## Documentation

For more in-depth details, please refer to the full [documentation](https://docs.rs/supercluster).

If you encounter any issues or have questions that are not addressed in the documentation, feel free to [submit an issue](https://github.com/chargetrip/supercluster-rs/issues).

## Usage

Run the following Cargo command in your project directory:

```bash
cargo add supercluster
```

```rust
use geojson::FeatureCollection;
use supercluster::{ CoordinateSystem, Supercluster, Options };

fn main() {
  let options = Options {
      max_zoom: 16,
      min_zoom: 0,
      min_points: 2,
      radius: 40.0,
      node_size: 64,
      extent: 512.0,
      coordinate_system: CoordinateSystem::LatLng,
  };

  // Create a new instance with the specified configuration settings
  let mut cluster = Supercluster::new(options);

  // Load a FeatureCollection Object into the Supercluster instance

  // [GeoJSON Format Specification § 5](https://tools.ietf.org/html/rfc7946#section-5)
  let feature_collection = FeatureCollection {
    bbox: None,
    // [GeoJSON Format Specification § 3.2](https://datatracker.ietf.org/doc/html/rfc7946#section-3.2)
    features: vec![],
    foreign_members: None,
  };
  let index = cluster.load(feature_collection);

  // Retrieve a FeatureCollection Object within a tile at the given zoom level and tile coordinates
  let tile = index.get_tile(0, 0.0, 0.0).expect("cannot get a tile");

  ...
}
```

## Contributing

Build:

```bash
cargo build
```

Test:

```bash
cargo test
```

Run [clippy](https://github.com/rust-lang/rust-clippy):

```bash
cargo clippy --all-targets --all-features --no-deps -- -D warnings
```

Run [rustfmt](https://github.com/rust-lang/rustfmt):

```bash
cargo fmt
```

Generate documentation in HTML format:

```bash
cargo doc --open
```

## Sponsors

[![Chargetrip logo](https://chargetrip-files.s3.eu-central-1.amazonaws.com/logo-1.png)](https://www.chargetrip.com)
