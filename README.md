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

Each item has `count()` and `points()` for the points that pass, `coordinates` for its position in your coordinate system, and `expansion_zoom()` for the zoom level where a cluster splits, counting only the points that pass. The filter runs once for every point of every item, and `expansion_zoom()` reuses those results, so an expensive filter does not run twice.

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
    let items = supercluster.tile_filtered(0, 0, 0, |station| station.available);

    for item in &items {
        println!("{:?}: {} station(s), zoom in to {:?}", item.coordinates, item.count(), item.expansion_zoom());
    }

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].count(), 1);

    Ok(())
}
```

For a cluster outside a tile request, such as a click, `cluster_expansion_zoom_filtered` finds the same zoom level and runs the filter again. See the [`filtered_tile`](https://github.com/chargetrip/supercluster-rs/blob/main/examples/filtered_tile.rs) example.

### Lowering the zoom level to cluster on

`with_cluster_max_zoom` returns a view of the supercluster that clusters up to a lower zoom level, for a single request. It can lower the `max_zoom` of the options but not raise it. Above that zoom level, the view returns the points instead of clusters. The view has the same queries: `clusters`, `tile`, `tile_filtered`, `cluster_expansion_zoom` and `cluster_expansion_zoom_filtered`.

## Upgrading from 3.x

Version 4 makes the supercluster generic over the type of the points, returns typed results instead of GeoJSON, and follows the naming of Rust APIs. The examples show version 3 code on `-` lines and version 4 code on `+` lines.

### Cargo features

The `geojson` feature holds all GeoJSON support and is not on by default anymore. The `cluster_metadata` feature is gone, because the GeoJSON features of clusters always have their `cluster`, `cluster_id`, `point_count` and `point_count_abbreviated` properties. The `log` feature is now the `tracing` feature, which still turns its events into `log` records for applications that only use the `log` crate.

```diff
-supercluster = { version = "3", features = ["log", "cluster_metadata"] }
+supercluster = { version = "4.0.0", features = ["geojson", "tracing"] }
```

### Options

Build the options with `SuperclusterOptions::builder()`, or take the defaults. `build()` returns a `Result`. It gives an error for a `max_zoom` above 30, a `min_zoom` above the `max_zoom`, a radius or extent that is not a finite number above 0, a `node_size` of 0 and a `DataRange` that cannot scale the data. Deserializing the options with the `serde` feature checks the same values, and a struct literal cannot build them anymore.

```diff
-let options = Supercluster::builder().radius(40.0).max_zoom(16).build();
+let options = SuperclusterOptions::builder().radius(40.0).max_zoom(16).build()?;
+let options = SuperclusterOptions::default();
```

A `DataRange` for the `Cartesian` coordinate system has no `offset` and `scale` fields anymore.

```diff
-let range = DataRange { min_x: 0.0, min_y: 0.0, max_x: 100.0, max_y: 100.0, offset: None, scale: None };
+let range = DataRange::new(0.0, 0.0, 100.0, 100.0);
```

### Loading

A supercluster is built in one step and does not change once it is loaded, so load a new one to cluster other points. `load_features` loads GeoJSON features, and `load` loads any type with a closure that tells where a point is. `FeatureBuilder` is gone, because `geojson` builds a feature from coordinates.

```diff
-let mut supercluster = Supercluster::new(options);
-let features = Supercluster::feature_builder().add_point(vec![4.9, 52.37]).build();
-supercluster.load(features)?;
+let features = vec![Feature::from(Geometry::new_point([4.9, 52.37]))];
+let supercluster = Supercluster::load_features(options, features)?;
+
+// Or your own type, kept as it is loaded
+let supercluster = Supercluster::load(options, stations, |station| Some([station.lng, station.lat]))?;
```

Points without coordinates, such as GeoJSON features that are not points, and points whose coordinates are not finite numbers are left out instead of causing a panic. The fields of `Supercluster` are private, so read them with `options()` and `points()`.

### Queries

The queries lose their `get_` prefix, as the Rust API guidelines ask, and return typed items instead of GeoJSON features.

```diff
-let features = supercluster.get_clusters(bbox, zoom)?;
-let tile = supercluster.get_tile(z, x as f64, y as f64)?;
-let children = supercluster.get_children(cluster_id)?;
-let leaves = supercluster.get_leaves(cluster_id, limit, offset);
-let zoom: usize = supercluster.get_cluster_expansion_zoom(cluster_id);
+let items = supercluster.clusters(bbox, zoom);
+let tile = supercluster.tile(z, x, y);
+let children = supercluster.children(cluster_id)?;
+let leaves: Vec<_> = supercluster.leaves(cluster_id)?.skip(offset).take(limit).collect();
+let zoom: u8 = supercluster.cluster_expansion_zoom(cluster_id)?;
```

- `clusters` and `tile` return no `Result`. A tile with nothing in it has no items, where version 3 gave the `TileNotFound` error.
- `tile` takes whole numbers (`u32`) for the tile coordinates instead of `f64`.
- `leaves` returns an iterator, so `skip` and `take` replace the `offset` and `limit`.
- `cluster_expansion_zoom` returns a `u8`, like the other zoom levels, and an error for an unknown cluster.
- `children`, `leaves` and `cluster_expansion_zoom` give `ClusterNotFound` for an unknown cluster. `TreeNotFound` and `TileNotFound` are gone.
- `tile` does not show items from the opposite edge of the data on the first and last tile for the `Cartesian` coordinate system, because the two ends of its range are not next to each other. `LatLng` still does.

### Typed results and GeoJSON

The queries return `Item`s and a `Tile`. Match on them, or convert them into GeoJSON with the `geojson` feature. The geometry of a tile feature holds the position in the tile, as in version 3.

```diff
-let tile = supercluster.get_tile(0, 0.0, 0.0)?;
+let tile = supercluster.tile(0, 0, 0).to_feature_collection();
+
+for tile_item in &supercluster.tile(0, 0, 0).items {
+    match tile_item.item {
+        Item::Cluster(cluster) => println!("{} points", cluster.point_count),
+        Item::Point { index, .. } => println!("point {index}"),
+    }
+}
```

You cannot build `Cluster`, `Tile` or `TileItem` yourself, so we can add fields to them without breaking your code.

### Cluster IDs

A cluster ID is a `ClusterId` instead of a `usize`, so a point index cannot be passed by mistake. Turn the number a client sends back into an ID, and an ID into the number to send.

```diff
-let children = supercluster.get_children(number)?;
+let children = supercluster.children(ClusterId::new(number))?;
+let number = cluster.id.get();
```

`ClusterId::from` and `usize::from` convert the same way.

### Imports

The modules are not public anymore, so import everything from the crate root. `KDBush` is not public anymore either.

```diff
-use supercluster::supercluster::Supercluster;
+use supercluster::Supercluster;
```

### Logging

The `tracing` feature reports one summary per load, with fields such as the number of points, and never reports queries, where version 3 logged every query.

### New in version 4

`tile_filtered` and `cluster_expansion_zoom_filtered` keep only the points that pass a filter, and `with_cluster_max_zoom` lowers the zoom level to cluster on for a single request. See [Filtering tiles](#filtering-tiles) and [Lowering the zoom level to cluster on](#lowering-the-zoom-level-to-cluster-on).

## Benchmarks

We use the `criterion` crate to benchmark the performance of the `supercluster` crate.

Benchmarks help us understand the performance characteristics of supercluster and identify areas for optimization.

We have several benchmark scenarios to test different aspects of supercluster:

- **Getting a Tile**: Tests the performance of retrieving a tile from the `Supercluster`.
- **Getting Clusters**: Tests the performance of retrieving clusters for a given bounding box and zoom level.
- **Loading Points**: Tests the performance of loading points into the `Supercluster`.
- **One Million Points**: Tests loading a million points and getting a tile from them.
- **Comparison with version 3**: Compares loading and the queries with version 3.0.8, on the same points.

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
