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
supercluster = "4.0.0"
```

No feature is on by default, so the crate only has what clustering needs. You can turn features on:

- `geojson`: support for [GeoJSON](https://docs.rs/geojson) features.
- `serde`: serialization of the options and of a loaded supercluster. To read a supercluster back with `serde_json`, enable its `float_roundtrip` feature, or some numbers come back with a tiny difference. The data is not checked when it is read, so only read what a supercluster wrote.
- `tracing`: reports through the [`tracing`](https://docs.rs/tracing) crate. A load reports a summary at the `debug` level, one event per zoom level at the `trace` level, and a warning for points left out because their coordinates are missing or not finite. The events have fields, such as the number of points. Queries report nothing. Applications that only use the `log` crate get the same events as `log` records.

A `Supercluster` clusters any type and keeps the points as you load them, without copying.
A closure tells where each point is, so you do not convert or wrap them, and shared data that changes stays shared.

```rust
use supercluster::{Item, Supercluster, SuperclusterBuilder, SuperclusterError};

fn main() -> Result<(), SuperclusterError> {
    // Set the configuration settings
    let options = SuperclusterBuilder::new()
        .radius(40.0)
        .extent(512.0)
        .min_points(2)
        .max_zoom(16)
        .build();

    // Create a new instance with the specified configuration settings
    let mut supercluster = Supercluster::new(options);

    // Load the longitude and latitude of the points into the supercluster
    supercluster.load_with(
        vec![[-77.032, 38.913], [-77.033, 38.913], [0.0, 0.0]],
        |point| Some(*point),
    )?;

    // Get the clusters and the points that are not clustered of a tile
    let tile = supercluster.get_tile(0, 0, 0);

    for tile_item in &tile.items {
        match &tile_item.item {
            Item::Cluster(cluster) => println!("{} points at {}, {}", cluster.point_count, tile_item.x, tile_item.y),
            Item::Point { index, .. } => println!("point {} at {}, {}", index, tile_item.x, tile_item.y),
        }
    }

    Ok(())
}
```

### GeoJSON

With the `geojson` feature a list of GeoJSON features is loaded as it is, with `load_features`, and what the supercluster returns converts back into GeoJSON.

```toml
[dependencies]
supercluster = { version = "4.0.0", features = ["geojson"] }
```

```rust,ignore
use geojson::{Feature, Geometry};
use supercluster::{Supercluster, SuperclusterBuilder, SuperclusterError};

fn main() -> Result<(), SuperclusterError> {
    let features: Vec<Feature> = [[-77.032, 38.913], [-77.033, 38.913]]
        .into_iter()
        .map(|point| Feature::from(Geometry::new_point(point)))
        .collect();

    let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build());

    supercluster.load_features(features)?;

    let tile = supercluster.get_tile(0, 0, 0).to_feature_collection();

    println!("Tile: {:?}", tile);

    Ok(())
}
```

### Your own points

Your own types work the same way. The closure returns `None` for a point that has no coordinates, which leaves it out. A point whose coordinates are not finite numbers is left out too.

```rust
use supercluster::{Item, Supercluster, SuperclusterBuilder, SuperclusterError};

struct Station {
    name: String,
    lng: f64,
    lat: f64,
}

fn main() -> Result<(), SuperclusterError> {
    let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build());

    supercluster.load_with(
        vec![
            Station { name: "A".to_string(), lng: 4.9, lat: 52.37 },
            Station { name: "B".to_string(), lng: 4.91, lat: 52.37 },
        ],
        |station| Some([station.lng, station.lat]),
    )?;

    // The two stations are close, so the tile holds a cluster of them
    let tile = supercluster.get_tile(0, 0, 0);

    for tile_item in &tile.items {
        if let Item::Cluster(cluster) = &tile_item.item {
            // The stations of the cluster, as they were loaded
            for station in supercluster.get_leaves(cluster.id, 10, 0)? {
                println!("{} is in cluster {}", station.name, cluster.id);
            }
        }
    }

    Ok(())
}
```

### Filtering and mapping tiles

The `TileBuilder` trait builds the items of a tile while it filters the points inside the clusters, for example the ones that match a search, and maps every cluster or point to the type of your response.
A cluster moves to the center of the points that are left after the filter.
See the [`tile_builder`](https://github.com/chargetrip/supercluster-rs/blob/main/examples/tile_builder.rs) example.

### Lowering the zoom level to cluster on

`get_tile_with_cluster_max_zoom` and `get_cluster_expansion_zoom_with` take a maximum zoom level to cluster on, which can lower the `max_zoom` of the supercluster for a single request but not raise it.

## Upgrading from 3.x

Version 4 makes the supercluster generic over the type of the points.

- `Supercluster` is now `Supercluster<P>`, and `load(features)` is `load_features(features)` with the `geojson` feature. `load_with(points, coordinates)` loads any other type. Create the options with `SuperclusterBuilder::new()`.
- The `cluster_metadata` feature is gone, so remove it from your `Cargo.toml`. The GeoJSON features of clusters always have their `cluster`, `cluster_id`, `point_count` and `point_count_abbreviated` properties. The `geojson` feature holds all GeoJSON support and is not on by default anymore, so enable it with `supercluster = { version = "4.0.0", features = ["geojson"] }` if you use GeoJSON features.
- The queries return `Item`s and a `Tile` instead of GeoJSON. Call `to_feature()` or `to_feature_collection()` to get GeoJSON features, whose tile geometry holds the position in the tile.
- `get_tile` returns an empty `Tile` instead of the `TileNotFound` error. `SuperclusterError::TreeNotFound` and `TileNotFound` are gone, and unknown clusters give `ClusterNotFound` for `get_children`, `get_leaves` and `get_cluster_expansion_zoom`.
- `get_tile`, `get_tile_with_cluster_max_zoom` and `TileCoordinates` take whole numbers (`u32`) for the tile coordinates instead of `f64`.
- `get_leaves` takes the points of a cluster from an index, so it does not walk the clusters anymore. A `limit` of 0 gives no points.
- The fields of `Supercluster` are private. Use `options()` and `points()`. You cannot build `Cluster`, `Tile` or `TileItem` yourself, so we can add fields to them without breaking your code.
- `load_with` returns an error for a `max_zoom` above 30 or a `node_size` of 0.
- `FeatureBuilder` is gone. Plain coordinates can be loaded as they are, and `geojson` builds a feature with `Feature::from(Geometry::new_point([lng, lat]))`.
- `KDBush` is not public anymore.
- Points without coordinates, such as GeoJSON features that are not points, and points whose coordinates are not finite numbers are left out instead of causing a panic.
- The `log` feature is now the `tracing` feature. It reports through `tracing`, with fields, and turns the events into `log` records for applications that only use the `log` crate. It logs one summary per load and never logs queries, where version 3 logged every query.

## Benchmarks

We use the `criterion` crate to benchmark the performance of the `supercluster` crate.

Benchmarks help us understand the performance characteristics of supercluster and identify areas for optimization.

We have several benchmark scenarios to test different aspects of supercluster:

- **Getting a Tile**: Tests the performance of retrieving a tile from the `Supercluster`.
- **Getting Clusters**: Tests the performance of retrieving clusters for a given bounding box and zoom level.
- **Loading Points**: Tests the performance of loading points into the `Supercluster`.
- **One Million Points**: Tests loading a million points and getting a tile from them.

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
