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
use supercluster::{Item, Supercluster, SuperclusterError, SuperclusterOptions};

fn main() -> Result<(), SuperclusterError> {
    // Set the configuration settings
    let options = SuperclusterOptions::builder()
        .radius(40.0)
        .extent(512.0)
        .min_points(2)
        .max_zoom(16)
        .build()?;

    // Load the longitude and latitude of the points into a new supercluster
    let supercluster = Supercluster::load(
        options,
        vec![[-77.032, 38.913], [-77.033, 38.913], [0.0, 0.0]],
        |point| Some(*point),
    )?;

    // Get the clusters and the points that are not clustered of a tile
    let tile = supercluster.tile(0, 0, 0);

    for tile_item in &tile.items {
        match &tile_item.item {
            Item::Cluster(cluster) => println!("{} points at {}, {}", cluster.point_count, tile_item.x, tile_item.y),
            Item::Point { index, .. } => println!("point {} at {}, {}", index, tile_item.x, tile_item.y),
        }
    }

    Ok(())
}
```

`SuperclusterOptions::default()` gives the default options, and the builder checks every value it sets. A supercluster does not change once it is loaded. To cluster other points, load a new one.

### GeoJSON

With the `geojson` feature a list of GeoJSON features is loaded as it is, with `load_features`, and what the supercluster returns converts back into GeoJSON.

```toml
[dependencies]
supercluster = { version = "4.0.0", features = ["geojson"] }
```

```rust,ignore
use geojson::{Feature, Geometry};
use supercluster::{Supercluster, SuperclusterError, SuperclusterOptions};

fn main() -> Result<(), SuperclusterError> {
    let features: Vec<Feature> = [[-77.032, 38.913], [-77.033, 38.913]]
        .into_iter()
        .map(|point| Feature::from(Geometry::new_point(point)))
        .collect();

    let supercluster = Supercluster::load_features(SuperclusterOptions::default(), features)?;

    let tile = supercluster.tile(0, 0, 0).to_feature_collection();

    println!("Tile: {:?}", tile);

    Ok(())
}
```

### Your own points

Your own types work the same way. The closure returns `None` for a point that has no coordinates, which leaves it out. A point whose coordinates are not finite numbers is left out too.

```rust
use supercluster::{Item, Supercluster, SuperclusterError, SuperclusterOptions};

struct Station {
    name: String,
    lng: f64,
    lat: f64,
}

fn main() -> Result<(), SuperclusterError> {
    let supercluster = Supercluster::load(
        SuperclusterOptions::default(),
        vec![
            Station { name: "A".to_string(), lng: 4.9, lat: 52.37 },
            Station { name: "B".to_string(), lng: 4.91, lat: 52.37 },
        ],
        |station| Some([station.lng, station.lat]),
    )?;

    // The two stations are close, so the tile holds a cluster of them
    let tile = supercluster.tile(0, 0, 0);

    for tile_item in &tile.items {
        if let Item::Cluster(cluster) = &tile_item.item {
            // The stations of the cluster, as they were loaded, ten at most
            for station in supercluster.leaves(cluster.id)?.take(10) {
                println!("{} is in cluster {}", station.name, cluster.id.get());
            }
        }
    }

    Ok(())
}
```

A cluster ID is a `ClusterId`. Send its number to a client with `get()` or `usize::from`, and turn the number the client sends back into an ID with `ClusterId::new` or `ClusterId::from`.

### Filtering tiles

`tile_filtered` keeps only the points that pass a filter, for example the ones that match a search. A cluster without any point left is left out, and a cluster with a few points left moves to their center. Map the items to the type of your response with an iterator.

```rust
use supercluster::{Supercluster, SuperclusterError, SuperclusterOptions};

struct Station {
    lng: f64,
    lat: f64,
    available: bool,
}

fn main() -> Result<(), SuperclusterError> {
    let supercluster = Supercluster::load(
        SuperclusterOptions::default(),
        vec![
            Station { lng: 4.9, lat: 52.37, available: true },
            Station { lng: 4.91, lat: 52.37, available: false },
        ],
        |station| Some([station.lng, station.lat]),
    )?;

    // The cluster holds the available station only, and moves onto it
    let positions: Vec<[f64; 2]> = supercluster
        .tile_filtered(0, 0, 0, |station| station.available)
        .into_iter()
        .map(|tile_item| [tile_item.x, tile_item.y])
        .collect();

    assert_eq!(positions.len(), 1);

    Ok(())
}
```

`cluster_expansion_zoom_filtered` finds the zoom level where a cluster expands, counting only the points that pass a filter. See the [`filtered_tile`](https://github.com/chargetrip/supercluster-rs/blob/main/examples/filtered_tile.rs) example.

### Lowering the zoom level to cluster on

`with_cluster_max_zoom` returns a view of the supercluster that clusters up to a lower zoom level, for a single request. It can lower the `max_zoom` of the options but not raise it. Above that zoom level, the view returns the points instead of clusters. The view has the same queries: `clusters`, `tile`, `tile_filtered`, `cluster_expansion_zoom` and `cluster_expansion_zoom_filtered`.

## Upgrading from 3.x

Version 4 makes the supercluster generic over the type of the points.

- `Supercluster` is now `Supercluster<P>`, and it is built in one step. `Supercluster::load(options, points, coordinates)` replaces `Supercluster::new(options)` followed by `load(features)`, and with the `geojson` feature `Supercluster::load_features(options, features)` loads GeoJSON features. A supercluster does not change once it is loaded, so load a new one to cluster other points.
- Create the options with `SuperclusterOptions::builder()` instead of `Supercluster::builder()`, or use `SuperclusterOptions::default()`. `build()` returns a `Result`, so add a `?` after it. It gives an error for a `max_zoom` above 30, a `min_zoom` above the `max_zoom`, a radius or extent that is not a finite number above 0, a `node_size` of 0 and a `DataRange` that cannot scale the data. `SuperclusterOptions` and the builder cannot be built with a struct literal anymore, and deserializing the options with the `serde` feature checks the same values.
- Create a `DataRange` with `DataRange::new(min_x, min_y, max_x, max_y)`. Its `offset` and `scale` fields are gone.
- Import everything from the crate root, such as `supercluster::Supercluster`. The modules are not public anymore.
- The queries lose their `get_` prefix, as the Rust API guidelines ask: `get_clusters`, `get_children`, `get_leaves`, `get_tile` and `get_cluster_expansion_zoom` are now `clusters`, `children`, `leaves`, `tile` and `cluster_expansion_zoom`.
- The queries return `Item`s and a `Tile` instead of GeoJSON. Call `to_feature()` or `to_feature_collection()` to get GeoJSON features, whose tile geometry holds the position in the tile.
- A cluster ID is a `ClusterId` instead of a `usize`. `children`, `leaves` and `cluster_expansion_zoom` take one. `ClusterId::new` or `ClusterId::from` turns a number into an ID, and `get()` or `usize::from` turns it back.
- `leaves` returns an iterator over the points of a cluster instead of taking a `limit` and an `offset`. Use `skip(offset)` and `take(limit)` on it.
- `cluster_expansion_zoom` returns a `Result` with a `u8`, like the other zoom levels, instead of a `usize`.
- The `cluster_metadata` feature is gone, so remove it from your `Cargo.toml`. The GeoJSON features of clusters always have their `cluster`, `cluster_id`, `point_count` and `point_count_abbreviated` properties. The `geojson` feature holds all GeoJSON support and is not on by default anymore, so enable it with `supercluster = { version = "4.0.0", features = ["geojson"] }` if you use GeoJSON features.
- `tile` returns an empty `Tile` instead of the `TileNotFound` error. `SuperclusterError::TreeNotFound` and `TileNotFound` are gone, and unknown clusters give `ClusterNotFound` for `children`, `leaves` and `cluster_expansion_zoom`.
- `tile` takes whole numbers (`u32`) for the tile coordinates instead of `f64`.
- `tile` does not show items from the opposite edge of the data on the first and last tile for the `Cartesian` coordinate system, because the two ends of its range are not next to each other. `LatLng` still does.
- The fields of `Supercluster` are private. Use `options()` and `points()`. You cannot build `Cluster`, `Tile` or `TileItem` yourself, so we can add fields to them without breaking your code.
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
