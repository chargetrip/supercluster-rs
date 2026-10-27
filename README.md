# Supercluster

[![Test status](https://github.com/chargetrip/supercluster-rs/actions/workflows/test.yml/badge.svg)](https://github.com/chargetrip/supercluster-rs/actions/workflows/test.yml)
[![Documentation on docs.rs](https://docs.rs/supercluster/badge.svg)](https://docs.rs/supercluster)
[![Latest version on crates.io](https://img.shields.io/crates/v/supercluster.svg)](https://crates.io/crates/supercluster)
![Downloads from crates.io](https://img.shields.io/crates/d/supercluster)
![License](https://img.shields.io/github/license/chargetrip/supercluster-rs)
[![Test coverage on Codecov](https://codecov.io/gh/chargetrip/supercluster-rs/graph/badge.svg?token=0S31CZY2ZJ)](https://codecov.io/gh/chargetrip/supercluster-rs)

A high-performance Rust crate for geospatial and non-geospatial point clustering. It groups nearby points into clusters on every zoom level, so a map can show millions of points as a few clusters.

This README is for Rust developers who show many points on a map or a chart. For every type and method, read the [API documentation on docs.rs](https://docs.rs/supercluster). If you encounter any issues or have questions that are not addressed in the documentation, feel free to [submit an issue on GitHub](https://github.com/chargetrip/supercluster-rs/issues).

![Clusters of points on a map, changing as the map zooms](https://cloud.githubusercontent.com/assets/25395/11857351/43407b46-a40c-11e5-8662-e99ab1cd2cb7.gif)

## Contents

- [Overview](#overview)
- [Install](#install)
- [Cluster your points](#cluster-your-points)
- [Filter what a tile shows](#filter-what-a-tile-shows)
- [More ways to use it](#more-ways-to-use-it)
- [API overview](#api-overview)
- [Upgrading from 3.x](#upgrading-from-3x)
- [Benchmarks](#benchmarks)
- [Safety](#safety)
- [Contributing](#contributing)
- [Sponsors](#sponsors)

## Overview

- **Load once.** `Supercluster::load` clusters your points on every zoom level and keeps the result as an index. A supercluster does not change once it is loaded. To cluster other points, load a new one.
- **Query many times.** Tiles, bounding boxes and clusters are answered from that index, without running your code again.
- **Keep your own type.** A closure tells where each point is, so you do not convert or wrap your points, and the supercluster never clones them.
- **Filter tiles.** Keep only the points that pass a filter, with a closure, or with selections that you prepare in advance and combine per request.
- **Use GeoJSON when you need it.** The `geojson` feature loads GeoJSON features, and turns what the supercluster returns back into GeoJSON.

This crate was initially inspired by Mapbox's supercluster [blog post about clustering millions of points](https://blog.mapbox.com/clustering-millions-of-points-on-a-map-with-supercluster-272046ec5c97).

## Install

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
supercluster = "4.0.0"
```

No feature is on by default, so the crate only has what clustering needs. Turn on the ones you use:

```toml
[dependencies]
supercluster = { version = "4.0.0", features = ["geojson"] }
```

| Feature | What it adds |
| --- | --- |
| `geojson` | Support for [GeoJSON features](https://docs.rs/geojson): loading them with `load_features`, and turning tiles, clusters and points back into GeoJSON. |
| `serde` | Serialization of the options and of a loaded supercluster. To read a supercluster back with `serde_json`, enable its `float_roundtrip` feature, or some numbers come back with a tiny difference. The data is not checked when it is read, so only read what a supercluster wrote. |
| `tracing` | Reports through the [`tracing` crate](https://docs.rs/tracing). A load reports a summary at the `debug` level, one event per zoom level at the `trace` level, and a warning for points left out because their coordinates are missing or not finite. The events have fields, such as the number of points. Queries report nothing. Applications that only use the `log` crate get the same events as `log` records. |

## Cluster your points

Load your points with a closure that returns the longitude and latitude of each one, then ask for the tiles of your map.

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
            Station { name: "C".to_string(), lng: -77.03, lat: 38.91 },
        ],
        |station| Some([station.lng, station.lat]),
    )?;

    // The tile of the whole world: a cluster of the two close stations, and the one far away
    for tile_item in &supercluster.tile(0, 0, 0).items {
        match &tile_item.item {
            Item::Cluster(cluster) => {
                // The stations of the cluster, ten at most
                for station in supercluster.leaves(cluster.id)?.iter().take(10) {
                    println!("{} is in cluster {}", station.name, cluster.id.get());
                }
            }
            Item::Point { index, point } => {
                println!("{} at {}, {} in the tile, loaded at {index}", point.name, tile_item.x, tile_item.y);
            }
        }
    }

    Ok(())
}
```

- **Points without coordinates are left out.** The closure returns `None` for a point that has no coordinates, which leaves it out. A point whose coordinates are not finite numbers is left out too.
- **Points move into cluster order.** `points()` returns the points in the order of the clusters, not the order you loaded them in, because loading keeps the points of every cluster next to each other. `Item::Point` has the `index` of a point in the list you loaded.
- **Cluster IDs go to clients as numbers.** A cluster ID is a `ClusterId`. Send its number to a client with `get()` or `u32::from`, and turn the number the client sends back into an ID with `ClusterId::new` or `ClusterId::from`. The numbers change from one load to the next.

### Set the options

`SuperclusterOptions::default()` gives these values, and `SuperclusterOptions::builder()` changes the ones you set. `build()` checks every value and returns an error for one it cannot use.

| Option | Default | What it sets |
| --- | --- | --- |
| `radius` | `40.0` | The cluster radius, in pixels. |
| `extent` | `512.0` | The size of a tile, in pixels, which the radius is relative to. |
| `min_points` | `2` | The fewest points that form a cluster. |
| `min_zoom` | `0` | The lowest zoom level to cluster on. |
| `max_zoom` | `16` | The highest zoom level to cluster on, up to `30`. Above it, a tile shows the points instead of clusters. |
| `node_size` | `64` | The number of points in a leaf of the KD-tree, which affects speed. |
| `coordinate_system` | `LatLng` | `LatLng` for longitudes and latitudes, or `Cartesian` for other 2D data. See [Cluster data that is not on a map](#cluster-data-that-is-not-on-a-map). |

```rust
use supercluster::{SuperclusterError, SuperclusterOptions};

fn main() -> Result<(), SuperclusterError> {
    let options = SuperclusterOptions::builder()
        .radius(60.0)
        .max_zoom(14)
        .build()?;

    // The values that are not set keep their default
    assert_eq!(options.extent, 512.0);

    Ok(())
}
```

## Filter what a tile shows

A filtered tile keeps only the points that pass a filter, for example the ones that match a search. A cluster without any point left is left out, and a cluster with a few points left moves to their center.

Each item has `count()` and `points()` for the points that pass, `coordinates()` for its position in your coordinate system, and `expansion_zoom()` for the zoom level where a cluster splits, counting only the points that pass. Map the items to the type of your response with an iterator.

### Choose a closure or selections

`tile_filtered` runs the filter on every point under the tile. That is cheap for a zoomed-in tile, but the tile of the whole world runs it on every point. A `Selection` runs a filter once, with `select`, and `tile_selected` and `cluster_expansion_zoom_selected` reuse it, so no filter runs per request.

| Use | What every query does | When to use it | What to keep in mind |
| --- | --- | --- | --- |
| `tile_filtered` with a closure | Runs the filter on every point under the tile. Nothing is cached, so the next query runs it again. | The filter cannot be prepared in advance, for example because it depends on the user, or you serve zoomed-in tiles only, where it costs about as much as a selection. It needs no preparation, so start with this one. | Nothing to prepare or refresh. |
| `tile_selected` with a combination of selections, such as `a.or(&b).and(&c)` | Evaluates the combination on the bits of the points under the tile, without running a filter. Nothing is cached, so it always reads the current selections. | The filter is made of several conditions that you can prepare in advance with `select`, one selection per condition. Its cost grows with the points of the tile, not with all the points. | Nothing to refresh: it reads the selections it combines when the query runs. The items borrow the combination, so to keep the items after the statement that makes them, keep the combination in a variable. |
| `tile_selected` with a selection, from `select` or from `compute` | Counts the points of every cluster in constant time, whatever the tile. | The filter is a single condition, or a combination that zoomed-out tiles use heavily. A combination costs the most there, because those tiles cover the most points, so compute it once with `compute` and cache it. | A selection is a snapshot (cached) of what the filter returned. If the filter only reads the points, it stays valid, because points do not change after `load`. If the filter reads other data that changes, run `select` or `compute` again when it changes. |

A selection belongs to the supercluster that made it. Keep them together, for example in the same `Arc`, and replace both when you load new points. With a selection of another supercluster, a query does not panic, but its results mean nothing.

Your results depend on your points, your filters and your machine, so measure your own case before you choose.

### Filter with a closure

The filter runs once for every point of every item, and `expansion_zoom()` reuses those results, so an expensive filter does not run twice. For a cluster outside a tile request, such as a click, `cluster_expansion_zoom_filtered` finds the same zoom level and runs the filter again.

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
        println!("{:?}: {} station(s), zoom in to {:?}", item.coordinates(), item.count(), item.expansion_zoom());
    }

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].count(), 1);

    Ok(())
}
```

The [`filtered_tile` example](https://github.com/chargetrip/supercluster-rs/blob/main/examples/filtered_tile.rs) shows both ways side by side.

### Filter with selections

Make one selection per condition with `select`, and combine them per request with `and`, `or` and `and_not`. A combination borrows the selections and computes nothing, and the query evaluates it on the points of its tile only. `compute` turns a combination into a selection of its own, which you can cache.

```rust
use supercluster::{Supercluster, SuperclusterError, SuperclusterOptions};

struct Station {
    lng: f64,
    lat: f64,
    power_kw: u32,
    available: bool,
}

fn main() -> Result<(), SuperclusterError> {
    let supercluster = Supercluster::load(
        SuperclusterOptions::default(),
        vec![
            Station { lng: 4.9, lat: 52.37, power_kw: 150, available: true },
            Station { lng: 4.91, lat: 52.37, power_kw: 150, available: false },
            Station { lng: 4.92, lat: 52.37, power_kw: 11, available: true },
        ],
        |station| Some([station.lng, station.lat]),
    )?;

    // One selection per condition, made once and reused by every request
    let fast = supercluster.select(|station| station.power_kw >= 50);
    let available = supercluster.select(|station| station.available);

    // Per request: combine what the request asks for, which the query evaluates on its tile only
    let fast_and_available = fast.and(&available);
    let items = supercluster.tile_selected(0, 0, 0, &fast_and_available);

    // The cluster holds the one fast charger that is available
    assert_eq!(items[0].count(), 1);

    Ok(())
}
```

## More ways to use it

### Send tiles as GeoJSON

With the `geojson` feature, a list of GeoJSON features is loaded as it is, with `load_features`. A feature without a point geometry is left out. What the supercluster returns converts back into GeoJSON.

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

To send a tile as GeoJSON, serialize `tile.as_feature_collection()`, for example with `serde_json::to_writer`. It writes the same JSON as `to_feature_collection()` but borrows the properties of the features instead of copying them.

### Cluster data that is not on a map

For data in its own units, such as the pixels of an image, use the `Cartesian` coordinate system with a `DataRange` that holds the bounds of the data. Both axes are scaled by the same factor, so the data keeps its proportions. A `Cartesian` tile does not wrap around at its edges, the way a map wraps at the antimeridian.

```rust
use supercluster::{CoordinateSystem, DataRange, Supercluster, SuperclusterError, SuperclusterOptions};

fn main() -> Result<(), SuperclusterError> {
    // A microscope image of 2048 by 1024 pixels
    let range = DataRange::new(0.0, 0.0, 2048.0, 1024.0);
    let options = SuperclusterOptions::builder()
        .coordinate_system(CoordinateSystem::Cartesian { range })
        .build()?;

    let cells = vec![[100.0, 200.0], [101.0, 200.0], [1900.0, 900.0]];
    let supercluster = Supercluster::load(options, cells, |cell| Some(*cell))?;

    // The two close cells form a cluster in the tile of the whole image
    assert_eq!(supercluster.tile(0, 0, 0).items.len(), 2);

    Ok(())
}
```

### Lower the zoom level for one request

`with_cluster_max_zoom` returns a view of the supercluster that clusters up to a lower zoom level, for a single request. It can lower the `max_zoom` of the options but not raise it. Above that zoom level, the view returns the points instead of clusters. The view has the same queries: `clusters`, `tile`, `tile_filtered`, `tile_selected`, `cluster_expansion_zoom`, `cluster_expansion_zoom_filtered` and `cluster_expansion_zoom_selected`.

## API overview

The [API documentation on docs.rs](https://docs.rs/supercluster) describes every item in full.

| Load | Returns | What it does |
| --- | --- | --- |
| `Supercluster::load(options, points, get_coordinates)` | `Result<Supercluster<P>, _>` | Clusters points of any type, with a closure that returns their coordinates. |
| `Supercluster::load_features(options, features)` | `Result<Supercluster<Feature>, _>` | Clusters GeoJSON features, with the `geojson` feature. |
| `SuperclusterOptions::default()` | `SuperclusterOptions` | The default options. |
| `SuperclusterOptions::builder()` | `SuperclusterOptionsBuilder` | Options that `build()` checks. |
| `DataRange::new(min_x, min_y, max_x, max_y)` | `DataRange` | The bounds of data in the `Cartesian` coordinate system. |

| Query | Returns | What it does |
| --- | --- | --- |
| `tile(z, x, y)` | `Tile` | The clusters and points of a map tile, with their positions in the tile. |
| `clusters(bbox, zoom)` | `Vec<Item>` | The clusters and points in a bounding box, at a zoom level. |
| `children(cluster_id)` | `Result<impl Iterator<Item = Item>, _>` | The clusters and points that a cluster splits into, on the next zoom level. |
| `leaves(cluster_id)` | `Result<&[P], _>` | All the points of a cluster. |
| `cluster_expansion_zoom(cluster_id)` | `Result<u8, _>` | The zoom level where a cluster splits. |
| `points()` | `&[P]` | The points that were loaded, in the order of the clusters. |
| `options()` | `&SuperclusterOptions` | The options of the supercluster. |
| `with_cluster_max_zoom(zoom)` | `SuperclusterView` | A view that clusters up to a lower zoom level, with the same tile, cluster and expansion zoom queries. |

| Filter | Returns | What it does |
| --- | --- | --- |
| `tile_filtered(z, x, y, filter)` | `Vec<FilteredTileItem>` | The items of a tile that keep a point for which the closure returns `true`. |
| `cluster_expansion_zoom_filtered(cluster_id, filter)` | `Result<u8, _>` | The zoom level where a cluster splits, counting only the points that pass the closure. |
| `select(filter)` | `Selection` | The points that pass the closure, found once for many queries. |
| `selection.and(&other)`, `or`, `and_not` | `Combination` | Selections combined without computing anything, for a query to evaluate. |
| `combination.compute()` | `Selection` | The combination computed for every point, to cache. |
| `tile_selected(z, x, y, &selection)` | `Vec<FilteredTileItem>` | The items of a tile that keep a point of a selection or a combination. |
| `cluster_expansion_zoom_selected(cluster_id, &selection)` | `Result<u8, _>` | The zoom level where a cluster splits, counting only the points of a selection or a combination. |

| Result | What it holds |
| --- | --- |
| `Tile` | `items`, a list of `TileItem`, and `is_empty()`. |
| `TileItem` | `x` and `y`, the position in the tile, and `item`. |
| `Item` | `Item::Cluster(Cluster)`, or `Item::Point { index, point }`, a point with its index in the list that was loaded. |
| `Cluster` | `id`, `point_count`, `coordinates()` for its center, and `point_count_abbreviated()`, such as `1.5k`. |
| `ClusterId` | The ID of a cluster: `ClusterId::new(number)` and `get()` turn it from and into a `u32`. |
| `FilteredTileItem` | `x`, `y` and `item`, with `coordinates()`, `count()` and `points()` for the points that pass, and `expansion_zoom()`. |
| `SuperclusterError` | The error of a query or a load: an unknown cluster, options that `build()` rejects, or too many points. |

With the `geojson` feature, `Tile::to_feature_collection()`, `Item::to_feature()`, `TileItem::to_feature()` and `Cluster::to_feature()` return GeoJSON, and `Tile::as_feature_collection()` serializes a tile without copying its features.

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
+// Or your own type, without converting it
+let supercluster = Supercluster::load(options, stations, |station| Some([station.lng, station.lat]))?;
```

Points without coordinates, such as GeoJSON features that are not points, and points whose coordinates are not finite numbers are left out instead of causing a panic. The fields of `Supercluster` are private, so read them with `options()` and `points()`.

`points()` returns the points in the order of the clusters, where the points of every cluster are next to each other, and the points that were left out come last. `Item::Point` has the `index` of a point in the list that was loaded. A supercluster holds up to `MAX_POINTS` points, 2^31, instead of `u32::MAX`.

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
+let children: Vec<_> = supercluster.children(cluster_id)?.collect();
+let leaves: Vec<_> = supercluster.leaves(cluster_id)?.iter().skip(offset).take(limit).collect();
+let zoom: u8 = supercluster.cluster_expansion_zoom(cluster_id)?;
```

- `clusters` and `tile` return no `Result`. A tile with nothing in it has no items, where version 3 gave the `TileNotFound` error.
- `tile` takes whole numbers (`u32`) for the tile coordinates instead of `f64`.
- `leaves` returns a slice of the points, so a range or `skip` and `take` replace the `offset` and `limit`.
- `children` returns an iterator, which reads the children that loading recorded.
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
+        Item::Cluster(cluster) => println!("{} points at {:?}", cluster.point_count, cluster.coordinates()),
+        Item::Point { index, .. } => println!("point {index}"),
+    }
+}
```

You cannot build `Cluster`, `Tile` or `TileItem` yourself, so we can add fields to them without breaking your code. The `point_count` of a `Cluster` is a `u32`, and `coordinates()` computes its center when you ask for it, so a tile that only needs positions in the tile does not pay for it.

### Cluster IDs

A cluster ID is a `ClusterId` that holds a `u32`, instead of a `usize`, so a point index cannot be passed by mistake. Turn the number a client sends back into an ID, and an ID into the number to send.

```diff
-let children = supercluster.get_children(number)?;
+let children = supercluster.children(ClusterId::new(number))?;
+let number: u32 = cluster.id.get();
```

`ClusterId::from` and `u32::from` convert the same way. The numbers differ from the ones of version 3 and of the JavaScript supercluster, and from one load to the next, so do not keep them across loads.

### Imports

The modules are not public anymore, so import everything from the crate root. `KDBush` is not public anymore either.

```diff
-use supercluster::supercluster::Supercluster;
+use supercluster::Supercluster;
```

### Logging

The `tracing` feature reports one summary per load, with fields such as the number of points, and never reports queries, where version 3 logged every query.

### New in version 4

`tile_filtered` and `cluster_expansion_zoom_filtered` keep only the points that pass a filter, `select` runs a filter once for many tiles and its selections combine with `and`, `or` and `and_not`, and `with_cluster_max_zoom` lowers the zoom level to cluster on for a single request. See [Filter what a tile shows](#filter-what-a-tile-shows) and [Lower the zoom level for one request](#lower-the-zoom-level-for-one-request).

## Benchmarks

We use the [`criterion` crate](https://docs.rs/criterion) to measure the speed of the crate. Run the benchmarks with:

```sh
cargo bench --features geojson
```

They measure:

- **Getting a tile:** retrieving a tile from a supercluster.
- **Getting clusters:** retrieving clusters for a given bounding box and zoom level.
- **Loading points:** loading points into a supercluster.
- **Filtering tiles:** filtering a tile with a closure, with a selection that `select` made, and with a combination of selections.
- **One million points:** loading a million points, and getting and filtering tiles from them.
- **Comparison with version 3:** loading and the queries of version 3.0.8, on the same points. It runs only when you name it: `cargo bench --features geojson --bench v3_comparison_bench`.

For more detailed benchmark scenarios, please refer to the [`benches` directory](https://github.com/chargetrip/supercluster-rs/tree/main/benches) in the repository.

## Safety

This crate uses `#![forbid(unsafe_code)]` to ensure everything is implemented in 100% safe Rust.

## Contributing

🎈 Thanks for your help improving the project! We are so happy to have you!

We have a [contributing guide](https://github.com/chargetrip/supercluster-rs/blob/main/CONTRIBUTING.md) to help you get involved in the project.

## Sponsors

<a href="https://www.chargetrip.com" target="_blank">
    <img src="https://dka575ofm4ao0.cloudfront.net/pages-transactional_logos/retina/149188/Chargetrip_Combined_-_Black.png" width="240" alt="Chargetrip">
</a>
