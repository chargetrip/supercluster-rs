# Supercluster

[![Test status](https://github.com/chargetrip/supercluster-rs/actions/workflows/test.yml/badge.svg)](https://github.com/chargetrip/supercluster-rs/actions/workflows/test.yml)
[![Documentation on docs.rs](https://docs.rs/supercluster/badge.svg)](https://docs.rs/supercluster)
[![Latest version on crates.io](https://img.shields.io/crates/v/supercluster.svg)](https://crates.io/crates/supercluster)
![Downloads from crates.io](https://img.shields.io/crates/d/supercluster)
![License](https://img.shields.io/github/license/chargetrip/supercluster-rs)
[![Test coverage on Codecov](https://codecov.io/gh/chargetrip/supercluster-rs/graph/badge.svg?token=0S31CZY2ZJ)](https://codecov.io/gh/chargetrip/supercluster-rs)

A high-performance Rust crate for geospatial and non-geospatial point clustering. It groups nearby points into clusters on every zoom level, so a map can show millions of points as a few clusters.

For every type and method, see the [API documentation on docs.rs](https://docs.rs/supercluster). For questions and bugs, [open an issue on GitHub](https://github.com/chargetrip/supercluster-rs/issues).

<https://github.com/user-attachments/assets/d99a7e82-f981-4e31-a327-1ff87631971c>

<sub>Clusters of points on a map, changing as the map zooms. From [go.chargetrip.com](https://go.chargetrip.com).</sub>

## Contents

- [Overview](#overview)
- [Install](#install)
- [API reference](https://docs.rs/supercluster)
- [Cluster your points](#cluster-your-points)
  - [Set the options](#set-the-options)
- [Filter what a tile shows](#filter-what-a-tile-shows)
  - [Choose how to filter](#choose-how-to-filter)
  - [Filter on each request](#filter-on-each-request)
  - [Filter with cached filters](#filter-with-cached-filters)
- [More ways to use it](#more-ways-to-use-it)
  - [Map and reduce the points](#map-and-reduce-the-points)
  - [Load and return GeoJSON](#load-and-return-geojson)
  - [Cluster data that is not on a map](#cluster-data-that-is-not-on-a-map)
  - [Lower the maximum zoom level for one request](#lower-the-maximum-zoom-level-for-one-request)
- [Upgrading from 3.x](#upgrading-from-3x)
- [Safety](#safety)
- [Contributing](#contributing)
- [Sponsors](#sponsors)

## Overview

- **Load once.** `Supercluster::load` clusters your points on every zoom level. To cluster other points, load a new supercluster.
- **Query many times.** Tiles, bounding boxes and clusters come from the index that `load` builds, without loading again.
- **Keep your own type.** A closure tells where each point is, so you never convert your points, and the supercluster never clones them.
- **Filter tiles.** Keep only the points that pass a filter, per request or with cached filters.
- **Use GeoJSON when you need it.** The `geojson` feature loads GeoJSON features and returns GeoJSON.

This crate started as a Rust rewrite of Mapbox's [supercluster](https://github.com/mapbox/supercluster) and of [kdbush](https://github.com/mourner/kdbush).

## Install

It needs Rust 1.96 or later.

```sh
cargo add supercluster@4.0.0-alpha.7
```

No feature is on by default. Turn on the ones you use:

```sh
cargo add supercluster@4.0.0-alpha.7 --features geojson
```

| Feature | What it adds |
| --- | --- |
| `geojson` | Load [GeoJSON features](https://docs.rs/geojson) with `load_features`, and turn tiles, clusters and points back into GeoJSON. |
| `serde` | Save and read back the options and a loaded supercluster. Reading checks the data, so queries cannot panic on it. With `serde_json`, enable its `float_roundtrip` feature. |
| `tracing` | Events of each load through the [`tracing` crate](https://docs.rs/tracing), or as `log` records: a summary at the `debug` level, events for each zoom level and KD-tree at the `trace` level, and a warning for coordinates that are not finite numbers. Queries report nothing. |

## Cluster your points

Load your points with a closure that returns the longitude and latitude of each one, then ask for the tiles of your map.

```rust
use supercluster::{Item, Options, Supercluster};

struct Station {
    name: String,
    lng: f64,
    lat: f64,
}

fn main() -> Result<(), supercluster::Error> {
    let supercluster = Supercluster::load(
        Options::default(),
        vec![
            Station { name: "A".to_string(), lng: 4.9, lat: 52.37 },
            Station { name: "B".to_string(), lng: 4.91, lat: 52.37 },
            Station { name: "C".to_string(), lng: -77.03, lat: 38.91 },
        ],
        // Return None to leave a point out. Coordinates that are not finite numbers are left out too
        |station| Some([station.lng, station.lat]),
    )?;

    // The tile of the whole world: a cluster of the two close stations, and the one far away.
    // The items come in no set order, so sort them if you need one
    for tile_item in &supercluster.tile(0, 0, 0).items {
        match tile_item.item() {
            Item::Cluster(cluster) => {
                // The number of the ID, which ClusterId::new(id) turns back into an ID. It can
                // change when the points, their order or the options change
                let id = cluster.id().get();

                // The zoom level where the cluster splits into smaller clusters or points
                if let Some(zoom) = tile_item.expansion_zoom() {
                    println!("Cluster {id} splits on zoom level {zoom}");
                }

                // The stations of the cluster, ten at most
                for station in tile_item.leaves().take(10) {
                    println!("{} is in cluster {id}", station.name);
                }
            }
            // index is the position in the list you loaded, not in points(), which keeps the
            // points of every cluster together
            Item::Point { index, point, .. } => {
                println!("{} at {}, {} in the tile, loaded at {index}", point.name, tile_item.x(), tile_item.y());
            }
        }
    }

    Ok(())
}
```

### Set the options

`Options::default()` works for most maps. Change the values you need with the builder, whose `build()` returns an error for a value it cannot use. See `OptionsBuilder` for every option, its default and its limits.

```rust
use supercluster::Options;

fn main() -> Result<(), supercluster::Error> {
    let options = Options::builder()
        .radius(60.0)
        .max_zoom(14)
        .build()?;

    // The values that are not set keep their default
    assert_eq!(options.extent(), 512);

    Ok(())
}
```

## Filter what a tile shows

A filtered tile keeps only the points that pass a filter, such as the ones that match a search:

- A cluster with no point left is left out.
- A cluster with one point left becomes that point.
- A cluster with more points left moves to the center of those points.

Pass the filter to `with_filter`. It returns a `FilteredView`, whose `tile` and `cluster_expansion_zoom` work on the points that pass. In a filtered tile, `count()`, `leaves()`, `center()` and `expansion_zoom()` cover only the points that pass. `item()` describes the whole cluster, or the point when one is left.

### Choose how to filter

The right filter depends on what you can prepare before the requests come in.

| Way | Use it when your filter | Speed | When to refresh it |
| --- | --- | --- | --- |
| A `Filter`, made with `filter` | Changes with every request, such as a text search | Runs your closure on every point under the tile, so it is slow on zoomed-out tiles. | Never |
| A `CachedFilter`, made with `cache_filter` | Is one condition you can prepare | The fastest on zoomed-out tiles, however many points they cover. | When the data your closure reads changes |
| A `CombinedFilter`, such as `a.or(&b).and(&c)` | Combines cached filters differently on each request | Far faster than a `Filter` on zoomed-out tiles, slower on zoomed-in ones. | Never |
| A combined filter cached with `cache` | Combines them the same way on many requests | As fast as a `CachedFilter`. | When a cached filter it combines changes |

The [`filtered_tile` example](https://github.com/chargetrip/supercluster-rs/blob/main/examples/filtered_tile.rs) shows a `Filter` and cached filters side by side.

#### What a request costs

Each time is one filtered tile on 10,000,000 points: `with_filter(filter).tile(z, x, y)`, then reading every item, for the tile of the busiest place at each zoom level, on a Mac with Apple silicon. Each column passes a different filter. The combined filter is `a.or(&b).and(&c).and(&d)`. The `cache()` in advance column uses a filter cached before the request, so building the cache is not counted. Brackets show the speedup over the `Filter`.

| Zoom | Points under the tile | `Filter` | `CombinedFilter` | `CombinedFilter` with `cache()` in advance |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 10,000,000 | 44.7 ms | 3.11 ms (14x) | 2.07 µs (21,589x) |
| 4 | 341,224 | 1.40 ms | 77.6 µs (18x) | 3.17 µs (442x) |
| 8 | 183,438 | 659 µs | 33.6 µs (20x) | 1.43 µs (460x) |
| 12 | 39,806 | 176 µs | 23.1 µs (7.6x) | 8.76 µs (20x) |
| 14 | 3,259 | 13.6 µs | 14.2 µs (1.0x) | 8.27 µs (1.6x) |
| 16 | 184 | 2.21 µs | 4.04 µs (0.5x) | 2.06 µs (1.1x) |
| 18 | 10 | 269 ns | 662 ns (0.4x) | 283 ns (1.0x) |

Your results depend on your points, filters and machine, so measure your own case.

#### What to keep in mind

- **A cached filter is a snapshot.** It keeps the results from when it was made, so refresh it as [Choose how to filter](#choose-how-to-filter) says.
- **Cache a combined filter only to reuse it.** Caching it for every request costs as much as letting the query combine it on the most zoomed-out tile, and more on every other tile.
- **A cached filter belongs to the supercluster that made it.** Another supercluster rejects it with `ForeignCachedFilter`, so replace both together when you load new points. The [`shared_state` example](https://github.com/chargetrip/supercluster-rs/blob/main/examples/shared_state.rs) shows how.

### Filter on each request

Make a `Filter` with `filter`, and pass it to `with_filter`. A query runs the closure on every point of the items it reads, including the buffer around the tile.

```rust
use supercluster::{Options, Supercluster};

struct Station {
    lng: f64,
    lat: f64,
    available: bool,
}

fn main() -> Result<(), supercluster::Error> {
    let supercluster = Supercluster::load(
        Options::default(),
        vec![
            Station { lng: 4.9, lat: 52.37, available: true },
            Station { lng: 4.91, lat: 52.37, available: false },
        ],
        |station| Some([station.lng, station.lat]),
    )?;

    // The cluster has one available station left, so the tile holds that station
    let available = supercluster.filter(|station| station.available);
    let items = supercluster.with_filter(&available)?.tile(0, 0, 0).items;

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].count(), 1);

    Ok(())
}
```

### Filter with cached filters

Make one `CachedFilter` per condition with `cache_filter`. On each request, combine them with `and`, `or` and `and_not`, and pass the result to `with_filter`.

```rust
use supercluster::{Item, Options, Supercluster};

struct Station {
    lng: f64,
    lat: f64,
    power_kw: u32,
    available: bool,
}

fn main() -> Result<(), supercluster::Error> {
    let supercluster = Supercluster::load(
        Options::default(),
        vec![
            Station { lng: 4.9, lat: 52.37, power_kw: 150, available: true },
            Station { lng: 4.91, lat: 52.37, power_kw: 150, available: false },
            Station { lng: 4.92, lat: 52.37, power_kw: 11, available: true },
        ],
        |station| Some([station.lng, station.lat]),
    )?;

    // One cached filter per condition, made once and reused by every request
    let fast = supercluster.cache_filter(|station| station.power_kw >= 50);
    let available = supercluster.cache_filter(|station| station.available);

    // Per request: combine what the request asks for, which the query evaluates on its tile only
    let items = supercluster.with_filter(fast.and(&available))?.tile(0, 0, 0).items;

    // One station is fast and available, so the cluster becomes that station
    assert!(matches!(items[0].item(), Item::Point { index: 0, .. }));
    assert_eq!(items[0].count(), 1);

    Ok(())
}
```

## More ways to use it

### Map and reduce the points

This crate has no `map` and `reduce` options, but you can achieve the same by reading your points from the items of a tile, filtered or not.

```rust
use supercluster::{Item, Options, Supercluster};

struct Station {
    lng: f64,
    lat: f64,
    capacity: u32,
    available: bool,
}

fn main() -> Result<(), supercluster::Error> {
    let supercluster = Supercluster::load(
        Options::default(),
        vec![
            Station { lng: 4.9, lat: 52.37, capacity: 4, available: true },
            Station { lng: 4.91, lat: 52.37, capacity: 2, available: true },
            Station { lng: 4.92, lat: 52.37, capacity: 8, available: false },
            Station { lng: -74.0, lat: 40.71, capacity: 3, available: true },
        ],
        |station| Some([station.lng, station.lat]),
    )?;

    // The capacity of the stations that are available, per item. To count the stations that pass,
    // read tile_item.count() instead
    let available = supercluster.filter(|station| station.available);
    let mut capacities: Vec<u32> = supercluster
        .with_filter(&available)?
        .tile(0, 0, 0)
        .into_iter()
        .map(|tile_item| match tile_item.item() {
            // Instead of map: a point is your own type, so read its fields
            Item::Point { point, .. } => point.capacity,
            // Instead of reduce: combine the fields of the points of the cluster that pass, which
            // leaves() holds
            Item::Cluster(_) => tile_item.leaves().map(|station| station.capacity).sum(),
        })
        .collect();

    capacities.sort();
    assert_eq!(capacities, [3, 6]);

    Ok(())
}
```

### Load and return GeoJSON

With the `geojson` feature, load GeoJSON features with `load_features`, and convert the results back into GeoJSON. The features of a tile hold the position in the tile, and `item.to_feature()` and `cluster.to_feature()` hold the coordinates. In a filtered tile, the `point_count` of a cluster counts only the points that pass.

```rust,ignore
use geojson::{Feature, Geometry};
use supercluster::{Options, Supercluster};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let features: Vec<Feature> = [[-77.032, 38.913], [-77.033, 38.913]]
        .into_iter()
        .map(|point| Feature::from(Geometry::new_point(point)))
        .collect();

    // A feature without a point geometry is left out. Shared features, such as Arc<Feature>,
    // load too, so the supercluster holds no copy
    let supercluster = Supercluster::load_features(Options::default(), features)?;
    let tile = supercluster.tile(0, 0, 0);

    // Write the tile as GeoJSON, for example into the body of a response. A cluster gets the
    // properties cluster, cluster_id and point_count. A point keeps the ID and properties it was
    // loaded with
    let mut body = Vec::new();

    serde_json::to_writer(&mut body, &tile.as_feature_collection())?;

    Ok(())
}
```

### Cluster data that is not on a map

For data in its own units, such as the pixels of an image, use the `Cartesian` coordinate system with a `DataRange` that holds the bounds of the data. The data keeps its proportions, and unlike a map, its tiles do not wrap around at the edges.

```rust
use supercluster::{CoordinateSystem, DataRange, Options, Supercluster};

fn main() -> Result<(), supercluster::Error> {
    // A microscope image of 2048 by 1024 pixels
    let range = DataRange::new(0.0, 0.0, 2048.0, 1024.0);
    let options = Options::builder()
        .coordinate_system(CoordinateSystem::Cartesian { range })
        .build()?;

    let cells = vec![[100.0, 200.0], [101.0, 200.0], [1900.0, 900.0]];
    let supercluster = Supercluster::load(options, cells, |cell| Some(*cell))?;

    // The two close cells form a cluster in the tile of the whole image
    assert_eq!(supercluster.tile(0, 0, 0).items.len(), 2);

    Ok(())
}
```

### Lower the maximum zoom level for one request

`with_cluster_max_zoom` returns a `View` that clusters up to a lower zoom level than `max_zoom`.

```rust
use supercluster::{Item, Options, Supercluster};

fn main() -> Result<(), supercluster::Error> {
    let points = vec![[4.9, 52.37], [4.9001, 52.37]];
    let supercluster =
        Supercluster::load(Options::default(), points, |point| Some(*point))?;
    let world = [-180.0, -90.0, 180.0, 90.0];

    // The two close points form a cluster on zoom level 13
    assert!(matches!(supercluster.clusters(world, 13)[0], Item::Cluster(_)));

    // Cluster up to zoom level 12 instead of 16. The value is kept between min_zoom and max_zoom
    let view = supercluster.with_cluster_max_zoom(12);

    // Above zoom level 12, the view returns the points. It also has tile, cluster_expansion_zoom
    // and with_filter
    assert!(view.clusters(world, 13).iter().all(|item| matches!(item, Item::Point { .. })));

    Ok(())
}
```

## Upgrading from 3.x

Version 4 makes the supercluster generic over the type of the points, returns typed results instead of GeoJSON, and follows the naming of Rust APIs. Each example shows the version 3 code under a `Version 3` comment, and the version 4 code that replaces it under a `Version 4` comment.

### Cargo features

The `geojson` feature holds all GeoJSON support and is not on by default anymore. The `cluster_metadata` feature is gone, because the GeoJSON features of clusters always have their `cluster`, `cluster_id` and `point_count` properties. The `log` feature is now the `tracing` feature, which still turns its events into `log` records for applications that only use the `log` crate.

```toml
# Version 3
supercluster = { version = "3", features = ["log", "cluster_metadata"] }

# Version 4
supercluster = { version = "4.0.0-alpha.7", features = ["geojson", "tracing"] }
```

The `geojson` feature takes `geojson` 1.0 instead of 0.24. If your code uses `geojson` too, upgrade it to 1.0, or its types will not match the ones of the supercluster, and port your code to the 1.0 API, such as `GeometryValue::Point { coordinates }`. Version 4 needs Rust 1.96 or later.

### Options

`SuperclusterOptions` is now `Options`, and `SuperclusterError` is now `Error`. Build the options with `Options::builder()`, or take the defaults. `build()` returns a `Result` and rejects values it cannot use, and so does deserializing them. Options that serde reads can leave out fields, which take their defaults, and a field it does not know is an error. The fields are private, so read them with the methods of the same names, such as `options.max_zoom()`. `min_points` is a `u32` instead of a `u8`, and `extent` a `u32` instead of an `f64`, so write `512` instead of `512.0` in options that serde reads.

```rust,ignore
// Version 3
let options = Supercluster::builder().radius(40.0).max_zoom(16).build();

// Version 4
let options = Options::builder().radius(40.0).max_zoom(16).build()?;

// Or take the defaults
let options = Options::default();
```

`CoordinateSystem::LatLng` is now `LngLat`. The order of the coordinates stays `[lng, lat]`. Options saved with serde need `"LngLat"` too, because `"LatLng"` no longer reads.

A `DataRange` for the `Cartesian` coordinate system has no `offset` and `scale` fields, and no public `normalize` and `denormalize` anymore. A `match` on `CoordinateSystem` needs a `_` arm.

```rust,ignore
// Version 3
let range = DataRange { min_x: 0.0, min_y: 0.0, max_x: 100.0, max_y: 100.0, offset: None, scale: None };

// Version 4
let range = DataRange::new(0.0, 0.0, 100.0, 100.0);
```

### Loading

A supercluster is built in one step and does not change once it is loaded, so load a new one to cluster other points. `load_features` loads GeoJSON features, and `load` loads any type with a closure that tells where a point is. `FeatureBuilder` is gone, because `geojson` builds a feature from coordinates.

```rust,ignore
// Version 3
let mut supercluster = Supercluster::new(options);
let features = Supercluster::feature_builder().add_point(vec![4.9, 52.37]).build();
supercluster.load(features)?;

// Version 4
let features = vec![Feature::from(Geometry::new_point([4.9, 52.37]))];
let supercluster = Supercluster::load_features(options, features)?;

// Or your own type, without converting it
let supercluster = Supercluster::load(options, stations, |station| Some([station.lng, station.lat]))?;
```

Points without coordinates, such as GeoJSON features that are not points, and points whose coordinates are not finite numbers are left out instead of causing a panic. The fields of `Supercluster` are private, so read them with `options()` and `points()`.

`points()` returns the points in the order of the clusters, where the points of every cluster are next to each other, and the points that were left out come last. `Item::Point` has the `index` of a point in the list that was loaded. A supercluster holds up to `MAX_POINTS` points, 2^31, instead of `u32::MAX`.

### Queries

The queries lose their `get_` prefix, and return typed items instead of GeoJSON features.

```rust,ignore
// Version 3
let features = supercluster.get_clusters(bbox, zoom)?;
let tile = supercluster.get_tile(z, x as f64, y as f64)?;
let children = supercluster.get_children(cluster_id)?;
let leaves = supercluster.get_leaves(cluster_id, limit, offset);
let zoom: usize = supercluster.get_cluster_expansion_zoom(cluster_id);

// Version 4
let items = supercluster.clusters(bbox, zoom);
let tile = supercluster.tile(z, x, y);
let children: Vec<_> = supercluster.children(cluster_id)?.collect();
let leaves: Vec<_> = supercluster.leaves(cluster_id)?.iter().skip(offset).take(limit).collect();
let zoom: u8 = supercluster.cluster_expansion_zoom(cluster_id)?;
```

- `clusters` and `tile` return no `Result`. A tile with nothing in it has no items, where version 3 gave the `TileNotFound` error.
- `tile` takes whole numbers (`u32`) for the tile coordinates instead of `f64`.
- `tile` and `clusters` return their items in another order than version 3. Neither version sets the order, so sort the items if you need one.
- `leaves` returns a slice of the points, so a range or `skip` and `take` replace the `offset` and `limit`.
- `children` returns an iterator, which reads the children that loading recorded.
- `cluster_expansion_zoom` returns a `u8`, like the other zoom levels, and an error for an unknown cluster.
- `children`, `leaves` and `cluster_expansion_zoom` give `ClusterNotFound` for an unknown cluster. `TreeNotFound` and `TileNotFound` are gone.
- `tile` does not show items from the opposite edge of the data on the first and last tile for the `Cartesian` coordinate system. `LngLat` still does.

### Typed results and GeoJSON

The queries return `Item`s and a `Tile`. Match on them, or convert them into GeoJSON with the `geojson` feature. The geometry of a tile feature holds the position in the tile, as in version 3. The `id` of a cluster feature is a number, the same as its `cluster_id` property, where version 3 wrote it as a string.

```rust,ignore
// Version 3
let tile = supercluster.get_tile(0, 0.0, 0.0)?;

// Version 4
let tile = supercluster.tile(0, 0, 0).to_feature_collection();

for tile_item in &supercluster.tile(0, 0, 0).items {
    // The position in the tile, as whole numbers (i32) on the grid of the extent, as in the
    // GeoJSON of version 3. Halfway rounds up, so -2.5 is -2 where version 3 gave -3. Items of the
    // buffer around the tile are below 0 or above the extent
    let (x, y) = (tile_item.x(), tile_item.y());

    // The center of a cluster, which in a filtered tile is the center of the points that pass.
    // None for a point, whose coordinates are in the point of item()
    let center = tile_item.center();

    match tile_item.item() {
        // A cluster is read through methods. Its center() holds the center of all of its points,
        // and is computed on each call
        Item::Cluster(cluster) => println!("{} points at {:?}", cluster.total_count(), cluster.center()),
        // A pattern on Item::Point needs ..
        Item::Point { index, .. } => println!("point {index}"),
    }
}
```

The GeoJSON features of clusters no longer have a `point_count_abbreviated` property. If you need it, format `point_count` yourself and add the property to the features of `to_feature_collection()`:

```rust,ignore
// Version 3, with the cluster_metadata feature
let tile = supercluster.get_tile(0, 0.0, 0.0)?;

// Version 4
let mut tile = supercluster.tile(0, 0, 0).to_feature_collection();

for feature in &mut tile.features {
    if let Some(count) = feature.property("point_count").and_then(|count| count.as_u64()) {
        let abbreviated = match count {
            10000.. => format!("{}k", (count as f64 / 1000.0).round()),
            1000.. => format!("{}k", (count as f64 / 100.0).round() / 10.0),
            _ => count.to_string(),
        };

        feature.set_property("point_count_abbreviated", abbreviated);
    }
}

// Write the tile as GeoJSON, for example into the body of a response
let mut body = Vec::new();

serde_json::to_writer(&mut body, &tile)?;
```

### Cluster IDs

Version 3 took a cluster ID as a plain `usize`. The numbers differ from the ones of version 3, and can change when the points, their order or the options change.

```rust,ignore
// Version 3
let children = supercluster.get_children(number)?;

// Version 4
let children = supercluster.children(ClusterId::new(number))?;

// Its number is a u32. ClusterId::from and u32::from convert the same way
let number: u32 = cluster.id().get();
```

### Saved superclusters

With the `serde` feature, version 4 cannot read a supercluster that version 3 wrote. Load the points again, and write the new supercluster. Version 4 writes the version of its format first, so a later version that changes the format gives an error instead of reading the data wrong.

### Imports

The modules are not public anymore, so import everything from the crate root. `KDBush` is not public anymore either.

```rust,ignore
// Version 3
use supercluster::supercluster::Supercluster;

// Version 4
use supercluster::Supercluster;
```

### Logging

The `tracing` feature reports a summary per load at the `debug` level, with fields such as the number of points, events for each zoom level and KD-tree at the `trace` level, and a warning for coordinates that are not finite numbers. It never reports queries, where version 3 logged every query.

### New in version 4

`with_filter` returns a view whose `tile` and `cluster_expansion_zoom` keep only the points that pass a filter. `filter` runs a closure on each query, `cache_filter` runs one once for many tiles, and cached filters combine with `and`, `or` and `and_not`. `with_cluster_max_zoom` lowers the zoom level to cluster on for a single request, and `cluster` finds a cluster by its ID. See [Filter what a tile shows](#filter-what-a-tile-shows) and [Lower the maximum zoom level for one request](#lower-the-maximum-zoom-level-for-one-request).

## Safety

This crate uses `#![forbid(unsafe_code)]` to ensure everything is implemented in 100% safe Rust.

## Contributing

🎈 Thanks for your help improving the project! We are so happy to have you!

We have a [contributing guide](https://github.com/chargetrip/supercluster-rs/blob/main/CONTRIBUTING.md) to help you get involved in the project.

## Sponsors

<a href="https://www.chargetrip.com" target="_blank">
    <img src="https://dka575ofm4ao0.cloudfront.net/pages-transactional_logos/retina/149188/Chargetrip_Combined_-_Black.png" width="240" alt="Chargetrip">
</a>
