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
- [Cluster your points](#cluster-your-points)
- [Filter what a tile shows](#filter-what-a-tile-shows)
  - [Choose how to filter](#choose-how-to-filter)
  - [Filter on each request](#filter-on-each-request)
  - [Filter with cached filters](#filter-with-cached-filters)
- [More ways to use it](#more-ways-to-use-it)
  - [Map and reduce the points](#map-and-reduce-the-points)
  - [Load and return GeoJSON](#load-and-return-geojson)
  - [Cluster data that is not on a map](#cluster-data-that-is-not-on-a-map)
  - [Lower the maximum zoom level for one request](#lower-the-maximum-zoom-level-for-one-request)
- [API quick reference](#api-quick-reference)
  - [Load points](#load-points)
  - [Query the clusters](#query-the-clusters)
  - [Filter tiles](#filter-tiles)
  - [Read the results](#read-the-results)
  - [Convert to GeoJSON](#convert-to-geojson)
  - [Errors and limits](#errors-and-limits)
- [Upgrading from 3.x](#upgrading-from-3x)
- [Benchmarks](#benchmarks)
- [Safety](#safety)
- [Contributing](#contributing)
- [Sponsors](#sponsors)

## Overview

- **Load once.** `Supercluster::load` clusters your points on every zoom level and keeps the result as an index. A supercluster does not change once it is loaded. To cluster other points, load a new one.
- **Query many times.** Tiles, bounding boxes and clusters are answered from that index, without running your code again.
- **Keep your own type.** A closure tells where each point is, so you do not convert or wrap your points, and the supercluster never clones them.
- **Filter tiles.** Keep only the points that pass a filter, with a closure that runs on each request, or with cached filters that you prepare in advance and combine per request.
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
| `serde` | Saving and reading back the options and a loaded supercluster, in any serde format. Reading rejects another format version and data that does not hold together, so queries cannot panic on it. With `serde_json`, enable its `float_roundtrip` feature, so the numbers read back exactly. |
| `tracing` | Events through the [`tracing` crate](https://docs.rs/tracing), or as `log` records: a summary of each load at the `debug` level, one event per zoom level at the `trace` level, and a warning for points left out. Queries report nothing. |

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
        match tile_item.item() {
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

- **Points without coordinates are left out.** That is a point the closure returns `None` for, or whose coordinates are not finite numbers.
- **Points move into cluster order.** `points()` keeps the points of every cluster next to each other, not in the order you loaded them. `Item::Point` has the `index` of a point in the list you loaded.
- **Send where each cluster splits.** `expansion_zoom()` gives the zoom level where a cluster splits, so the client can zoom the map in to it when the user clicks the cluster.
- **Cluster IDs go to clients as numbers.** Send `cluster.id.get()`, and turn the number a client sends back into an ID with `ClusterId::new`. With it, `cluster_expansion_zoom` finds where the cluster splits, `children` what it splits into, and `leaves` its points. The numbers change from one load to the next, so an old one can give `ClusterNotFound` or name another cluster.

### Set the options

`SuperclusterOptions::default()` gives these values, and `SuperclusterOptions::builder()` changes the ones you set. `build()` checks every value and returns an error for one it cannot use. Read an option back with the method of the same name, such as `options.extent()`.

| Option | Default | What it sets |
| --- | --- | --- |
| `radius` | `40.0` | The cluster radius, in pixels. |
| `extent` | `512` | The size of a tile, in pixels, which the radius is relative to. A whole number from 1 to `i32::MAX`, and the extent plus the radius cannot be above `i32::MAX`. |
| `min_points` | `2` | The fewest points that form a cluster. |
| `min_zoom` | `0` | The lowest zoom level to cluster on. |
| `max_zoom` | `16` | The highest zoom level to cluster on, up to `30`. Above it, a tile shows the points instead of clusters. |
| `node_size` | `64` | The number of points in a leaf of the KD-tree, which affects speed. |
| `coordinate_system` | `LngLat` | `LngLat` for longitudes and latitudes, or `Cartesian` for other 2D data. See [Cluster data that is not on a map](#cluster-data-that-is-not-on-a-map). |

```rust
use supercluster::{SuperclusterError, SuperclusterOptions};

fn main() -> Result<(), SuperclusterError> {
    let options = SuperclusterOptions::builder()
        .radius(60.0)
        .max_zoom(14)
        .build()?;

    // The values that are not set keep their default
    assert_eq!(options.extent(), 512);

    Ok(())
}
```

## Filter what a tile shows

A filtered tile keeps only the points that pass a filter, for example the ones that match a search. A cluster without any point left is left out, a cluster with one point left becomes that point, and a cluster with more points left moves to the exact center of the points left.

Pass the filter to `with_filter`, which returns a `FilteredView`. Its `tile` and `cluster_expansion_zoom` work like the ones of the supercluster, on the points that pass. `with_filter` checks the filter once, so the queries of the view cannot fail on it.

Each item's `count()`, its points from `leaves()`, `expansion_zoom()` and `coordinates()` take only the points that pass into account, as does the `point_count` of its GeoJSON. The cluster from `item()` still describes all of its points, in its `total_count` and `coordinates()`.

### Choose how to filter

`with_filter` takes three kinds of filter, and a combined filter can also be cached. They differ in what runs on each request, so the right one depends on what you can prepare before the requests come in.

| Way | Use it when your filter | What runs on each request | When to refresh it |
| --- | --- | --- | --- |
| A `Filter`, made with `filter` | Changes with every request, such as a text search | Your closure, on every point under the tile, and the center of each cluster with points that fail. As fast as the others on zoomed-in tiles, but slow on zoomed-out ones. | Never, because nothing is cached. |
| A `CachedFilter`, made with `cache_filter` | Is one condition that you can prepare | A count of the points of each cluster that pass, and their center, with a few lookups, however many points the tile covers. The fastest way on zoomed-out tiles. | When the data your closure reads changes. |
| A `CombinedFilter` of cached filters, such as `a.or(&b).and(&c)` | Combines cached conditions differently on each request | `and`, `or` and `and_not` on the bits of the cached filters, for the points under the tile only, and the center of each cluster with points that fail. Far faster than a `Filter` on zoomed-out tiles, and at most about 2 µs slower on zoomed-in ones. | Never, because nothing is cached. Each request uses the cached filters as they are when it comes in. |
| A combined filter cached with `cache` | Combines cached conditions the same way on many requests, above all zoomed-out ones | A few lookups per cluster, like a `CachedFilter`. | After you refresh one of the cached filters it combines. |

#### What a request costs

One request on 10,000,000 points around 3,000 places, on a Mac with Apple silicon. It filters with four cached filters, `a.or(&b).and(&c).and(&d)`, reads every item, and uses the tile of the busiest place at each zoom level. In brackets is how many times faster than the `Filter` each way is. Run it with `cargo bench --bench request_cost_bench`.

| Zoom | Points under the tile | Filter | Combined filter | Cached filter |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 10,000,000 | 44.4 ms | 3.09 ms (14x) | 1.99 µs (22,248x) |
| 4 | 341,224 | 1.41 ms | 76.5 µs (18x) | 2.85 µs (495x) |
| 8 | 183,438 | 671 µs | 32.2 µs (21x) | 1.11 µs (607x) |
| 12 | 39,806 | 189 µs | 22.6 µs (8.3x) | 8.83 µs (21x) |
| 14 | 3,259 | 13.4 µs | 13.8 µs (1.0x) | 8.01 µs (1.7x) |
| 16 | 184 | 2.07 µs | 3.66 µs (0.6x) | 1.81 µs (1.1x) |
| 18 | 10 | 306 ns | 655 ns (0.5x) | 320 ns (1.0x) |

Your results depend on your points, filters and machine, so measure your own case.

#### What to keep in mind

- **A cached filter is a snapshot.** It stays valid while its closure only reads the points, which do not change after `load`. If the closure reads other data, call `cache_filter` again when that data changes.
- **A cached combined filter is a snapshot too.** Cache it again after you refresh a cached filter it combines.
- **Cache a combined filter only to reuse it.** Caching it for every request costs more than letting the query combine it.
- **A cached filter belongs to the supercluster that made it.** Replace both together when you load new points, because another supercluster rejects it with `ForeignCachedFilter`, even one with the same points. A `Filter` works with any supercluster. The [`shared_state` example](https://github.com/chargetrip/supercluster-rs/blob/main/examples/shared_state.rs) keeps them together and refreshes a cached filter when its data changes.

### Filter on each request

Make a `Filter` with `filter`, and pass it to `with_filter`. A query of the view runs the closure once per point, and the items reuse those results, so an expensive closure never runs twice.

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

    // The cluster has one available station left, so the tile holds that station
    let available = supercluster.filter(|station| station.available);
    let items = supercluster.with_filter(&available)?.tile(0, 0, 0).items;

    for item in &items {
        println!("{:?}: {} station(s), zoom in to {:?}", item.coordinates(), item.count(), item.expansion_zoom());
    }

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].count(), 1);

    Ok(())
}
```

The [`filtered_tile` example](https://github.com/chargetrip/supercluster-rs/blob/main/examples/filtered_tile.rs) shows both ways side by side.

### Filter with cached filters

Make one `CachedFilter` per condition with `cache_filter`, and combine them per request with `and`, `or` and `and_not`. Pass the combined filter straight to `with_filter`, and `cache` it when many requests use the same one.

```rust
use supercluster::{Item, Supercluster, SuperclusterError, SuperclusterOptions};

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

This crate has no `map` and `reduce` options. Combine the values of the points when you need them, which also works on filtered tiles:

- Instead of `map`, read the fields of your points, which are your own type.
- Instead of `reduce` to count matching points, filter the tile with `with_filter`, and read each item's `count()`.
- Instead of `reduce` to combine values, like a sum, compute them from `tile_item.leaves()`, which in a filtered tile holds only the points that pass the filter.

```rust
use supercluster::{Item, Supercluster, SuperclusterError, SuperclusterOptions};

struct Station {
    lng: f64,
    lat: f64,
    capacity: u32,
    available: bool,
}

fn main() -> Result<(), SuperclusterError> {
    let supercluster = Supercluster::load(
        SuperclusterOptions::default(),
        vec![
            Station { lng: 4.9, lat: 52.37, capacity: 4, available: true },
            Station { lng: 4.91, lat: 52.37, capacity: 2, available: true },
            Station { lng: 4.92, lat: 52.37, capacity: 8, available: false },
            Station { lng: -74.0, lat: 40.71, capacity: 3, available: true },
        ],
        |station| Some([station.lng, station.lat]),
    )?;
    let mut capacities = vec![];

    // The capacity of the stations that are available, per item
    let available = supercluster.filter(|station| station.available);

    for tile_item in &supercluster.with_filter(&available)?.tile(0, 0, 0).items {
        let capacity: u32 = match tile_item.item() {
            // Instead of map: a point is your own type, so read its fields
            Item::Point { point, .. } => point.capacity,
            // Instead of reduce: combine the fields of the points of the cluster that pass
            Item::Cluster(_) => tile_item.leaves().map(|station| station.capacity).sum(),
        };

        capacities.push(capacity);
    }

    capacities.sort();
    assert_eq!(capacities, [3, 6]);

    Ok(())
}
```

### Load and return GeoJSON

With the `geojson` feature, a supercluster loads GeoJSON features and returns its results as GeoJSON:

1. Load the features with `load_features`. A feature without a point geometry is left out.
2. Convert what a query returns into GeoJSON, with the calls in the table below.

| To return | Call | The geometry holds |
| --- | --- | --- |
| A tile, to send to a map | `tile.as_feature_collection()`, which you serialize, or `tile.to_feature_collection()` | The position in the tile, in the units of the tile `extent`. |
| A filtered tile | The same calls, on the tile of a `FilteredView` | The position in the tile, at the center of the points that pass. |
| The items of a bounding box | `item.to_feature()`, on each item that `clusters(bbox, zoom)` returns | The coordinates, such as the longitude and latitude. |
| One cluster | `cluster.to_feature()` | The coordinates of its center. |

A cluster becomes a feature with the properties `cluster`, `cluster_id` and `point_count`, and a point keeps the ID and the properties it was loaded with. In a filtered tile, a cluster's `point_count` counts the points that pass.

```rust,ignore
use geojson::{Feature, Geometry};
use supercluster::{Supercluster, SuperclusterOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let features: Vec<Feature> = [[-77.032, 38.913], [-77.033, 38.913]]
        .into_iter()
        .map(|point| Feature::from(Geometry::new_point(point)))
        .collect();

    let supercluster = Supercluster::load_features(SuperclusterOptions::default(), features)?;
    let tile = supercluster.tile(0, 0, 0);

    // Write the tile as GeoJSON, for example into the body of a response
    let mut body = Vec::new();

    serde_json::to_writer(&mut body, &tile.as_feature_collection())?;

    Ok(())
}
```

To send a tile, serialize `as_feature_collection()`. It borrows the properties instead of copying them, and writes whole numbers, such as `150`, where `to_feature_collection()` writes `150.0`. Use `to_feature_collection()` to change the features before you send them.

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

### Lower the maximum zoom level for one request

`with_cluster_max_zoom` returns a view of the supercluster that clusters up to a lower zoom level, for a single request. It can lower the `max_zoom` of the options but not raise it. Above that zoom level, the view returns the points instead of clusters. The view has the same queries: `clusters`, `tile` and `cluster_expansion_zoom`. Its `with_filter` returns a filtered view that clusters up to the same zoom level, as in `supercluster.with_cluster_max_zoom(12).with_filter(&available)?.tile(z, x, y)`.

## API quick reference

The [API documentation on docs.rs](https://docs.rs/supercluster) describes every item in full. Import everything from the crate root, such as `use supercluster::Supercluster;`. In the tables, `_` in a `Result` stands for `SuperclusterError`.

### Load points

| Call | Returns | What it does |
| --- | --- | --- |
| `Supercluster::load(options, points, get_coordinates)` | `Result<Supercluster<P>, _>` | Clusters points of any type. `get_coordinates` returns the coordinates of a point, or `None` to leave it out. |
| `Supercluster::load_features(options, features)` | `Result<Supercluster<Feature>, _>` | Clusters GeoJSON features, with the `geojson` feature. |
| `SuperclusterOptions::default()` | `SuperclusterOptions` | The default options, listed in [Set the options](#set-the-options). |
| `SuperclusterOptions::builder()` | `SuperclusterOptionsBuilder` | Sets the options one by one. Its `build()` checks them and returns `Result<SuperclusterOptions, _>`. |
| `CoordinateSystem::LngLat` | `CoordinateSystem` | Longitudes and latitudes. This is the default. |
| `CoordinateSystem::Cartesian { range }` | `CoordinateSystem` | Other 2D data, within the bounds of a `DataRange`. |
| `DataRange::new(min_x, min_y, max_x, max_y)` | `DataRange` | The bounds of the data for the `Cartesian` coordinate system. |

### Query the clusters

These are methods of `Supercluster<P>`. The last column tells whether the `SuperclusterView` that `with_cluster_max_zoom` returns has the method too.

| Method | Returns | What it does | On a view |
| --- | --- | --- | --- |
| `tile(z, x, y)` | `Tile` | The clusters and points of a map tile, with their positions in the tile. | Yes |
| `clusters(bbox, zoom)` | `Vec<Item>` | The clusters and points in a bounding box at a zoom level. The box is `[min_lng, min_lat, max_lng, max_lat]`, or `[min_x, min_y, max_x, max_y]` for `Cartesian`. | Yes |
| `cluster_expansion_zoom(cluster_id)` | `Result<u8, _>` | The zoom level where a cluster splits. | Yes |
| `children(cluster_id)` | `Result<impl ExactSizeIterator<Item = Item>, _>` | The clusters and points that a cluster splits into, on the next zoom level. | No |
| `leaves(cluster_id)` | `Result<&[P], _>` | All the points of a cluster. | No |
| `points()` | `&[P]` | The points that were loaded, in the order of the clusters. | No |
| `options()` | `&SuperclusterOptions` | The options of the supercluster. | No |
| `with_cluster_max_zoom(zoom)` | `SuperclusterView` | A view that clusters up to a lower zoom level, for one request. | No |

### Filter tiles

These are methods of `Supercluster<P>`, with the same last column as the queries. A `filter` is a `Filter`, a `CachedFilter` by reference, also behind an `Arc` or a `Box`, or a `CombinedFilter`.

| Method | Returns | What it does | On a view |
| --- | --- | --- | --- |
| `filter(closure)` | `Filter` | A filter that runs the closure on the points of each query. It runs nothing when it is made. | No |
| `cache_filter(closure)` | `CachedFilter` | The points that pass the closure, found once for many queries. | No |
| `with_filter(filter)` | `Result<FilteredView, _>` | A view whose queries keep only the points that pass the filter. It gives `ForeignCachedFilter` for a cached filter of another supercluster. | Yes |

These are methods of the `FilteredView` that `with_filter` returns.

| Method | Returns | What it does |
| --- | --- | --- |
| `tile(z, x, y)` | `Tile` | The items of a tile that keep a point that passes the filter. |
| `cluster_expansion_zoom(cluster_id)` | `Result<u8, _>` | The zoom level where a cluster splits, counting only the points that pass the filter. |

Cached filters combine with the methods below, which chain, as in `a.or(&b).and(&c)`. `a` is a cached or combined filter, and `b` is too, by reference. A `Filter` does not combine.

| Method | Returns | What it does |
| --- | --- | --- |
| `a.and(b)` | `CombinedFilter` | The points in both `a` and `b`. |
| `a.or(b)` | `CombinedFilter` | The points in `a`, in `b`, or in both. |
| `a.and_not(b)` | `CombinedFilter` | The points in `a` that are not in `b`. |
| `combined_filter.cache()` | `CachedFilter` | The combined filter evaluated for every point, to keep. |
| `cached_filter.count()` | `u32` | The number of points of the cached filter. |

### Read the results

| Type | What it holds | Methods |
| --- | --- | --- |
| `Tile` | `items`, a list of `TileItem`. `tile` returns it, with or without a filter. | `is_empty()` |
| `TileItem` | `x` and `y`, the position in the tile as whole numbers (`i32`). | `item()` for the cluster or point, `coordinates()`, `count()` and `leaves()` for its points, or in a filtered tile the points that pass, and `expansion_zoom()` |
| `Item` | `Item::Cluster(Cluster)`, or `Item::Point { index, point }`, a point with its index in the list that was loaded. | None |
| `Cluster` | `id`, and `total_count`, all of its points, also in a filtered tile. | `coordinates()` for its center |
| `ClusterId` | The ID of a cluster, as a `u32`. `ClusterId::new(number)` makes one from a `u32`. | `get()` returns the `u32` |

### Convert to GeoJSON

These methods need the `geojson` feature.

| Method | Returns | What it does |
| --- | --- | --- |
| `tile.to_feature_collection()` | `FeatureCollection` | The items of the tile, with their positions in the tile as the geometry. |
| `tile.as_feature_collection()` | `FeatureCollectionView` | Serializes to the same GeoJSON as `to_feature_collection()`, without copying the properties, and with whole numbers for the positions in the tile. |
| `tile_item.to_feature()` | `Feature` | The item, with its position in the tile as the geometry. |
| `item.to_feature()` | `Feature` | A cluster at its center, or the point as it was loaded. |
| `cluster.to_feature()` | `Feature` | The cluster at its center, with the properties `cluster`, `cluster_id` and `point_count`. |

### Errors and limits

| Item | What it means |
| --- | --- |
| `SuperclusterError::ClusterNotFound` | No cluster of the supercluster has the ID. |
| `SuperclusterError::TooManyPoints` | `load` got more than `MAX_POINTS` points. |
| `SuperclusterError::ForeignCachedFilter` | `with_filter` got a cached filter of another supercluster. |
| `SuperclusterError::InvalidMaxZoom`, `InvalidMinZoom`, `InvalidRadius`, `InvalidExtent`, `InvalidDataRange` and `InvalidNodeSize` | `build()` rejected an option. |
| `MAX_POINTS` | The most points a supercluster holds: 2^31. |
| `MAX_ZOOM_LIMIT` | The highest `max_zoom`: 30. |

## Upgrading from 3.x

Version 4 makes the supercluster generic over the type of the points, returns typed results instead of GeoJSON, and follows the naming of Rust APIs. Each example shows the version 3 code under a `Version 3` comment, and the version 4 code that replaces it under a `Version 4` comment.

### Cargo features

The `geojson` feature holds all GeoJSON support and is not on by default anymore. The `cluster_metadata` feature is gone, because the GeoJSON features of clusters always have their `cluster`, `cluster_id` and `point_count` properties. The `log` feature is now the `tracing` feature, which still turns its events into `log` records for applications that only use the `log` crate.

```toml
# Version 3
supercluster = { version = "3", features = ["log", "cluster_metadata"] }

# Version 4
supercluster = { version = "4.0.0", features = ["geojson", "tracing"] }
```

The `geojson` feature takes `geojson` 1.0 instead of 0.24. If your code uses `geojson` too, upgrade it to 1.0, or its types will not match the ones of the supercluster, and port your code to the 1.0 API, such as `GeometryValue::Point { coordinates }`. Version 4 needs Rust 1.96 or later.

### Options

Build the options with `SuperclusterOptions::builder()`, or take the defaults. `build()` returns a `Result` and rejects values it cannot use, and so does deserializing them. Options that serde reads can leave out fields, which take their defaults, and a field it does not know is an error. The fields are private, so read them with the methods of the same names, such as `options.max_zoom()`. `min_points` is a `u32` instead of a `u8`, and `extent` a `u32` instead of an `f64`, so write `512` instead of `512.0` in options that serde reads.

```rust,ignore
// Version 3
let options = Supercluster::builder().radius(40.0).max_zoom(16).build();

// Version 4
let options = SuperclusterOptions::builder().radius(40.0).max_zoom(16).build()?;

// Or take the defaults
let options = SuperclusterOptions::default();
```

`CoordinateSystem::LatLng` is now `LngLat`, which names the coordinates in the order they come in, `[lng, lat]`, as in GeoJSON. Options saved with serde need `"LngLat"` too, because `"LatLng"` no longer reads.

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

The queries lose their `get_` prefix, as the Rust API guidelines ask, and return typed items instead of GeoJSON features.

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
- `leaves` returns a slice of the points, so a range or `skip` and `take` replace the `offset` and `limit`.
- `children` returns an iterator, which reads the children that loading recorded.
- `cluster_expansion_zoom` returns a `u8`, like the other zoom levels, and an error for an unknown cluster.
- `children`, `leaves` and `cluster_expansion_zoom` give `ClusterNotFound` for an unknown cluster. `TreeNotFound` and `TileNotFound` are gone.
- `tile` does not show items from the opposite edge of the data on the first and last tile for the `Cartesian` coordinate system, because the two ends of its range are not next to each other. `LngLat` still does.

### Typed results and GeoJSON

The queries return `Item`s and a `Tile`. Match on them, or convert them into GeoJSON with the `geojson` feature. The geometry of a tile feature holds the position in the tile, as in version 3. The `id` of a cluster feature is a number, the same as its `cluster_id` property, where version 3 wrote it as a string.

```rust,ignore
// Version 3
let tile = supercluster.get_tile(0, 0.0, 0.0)?;

// Version 4
let tile = supercluster.tile(0, 0, 0).to_feature_collection();

for tile_item in &supercluster.tile(0, 0, 0).items {
    match tile_item.item() {
        Item::Cluster(cluster) => println!("{} points at {:?}", cluster.total_count, cluster.coordinates()),
        Item::Point { index, .. } => println!("point {index}"),
    }
}
```

The GeoJSON features of clusters no longer have a `point_count_abbreviated` property. We leave it to the consumers of the library to format the count as they wish. If you need it, format `point_count` yourself and add the property to the features of `to_feature_collection()`:

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

The `x` and `y` of a `TileItem` are the position in the tile as whole numbers (`i32`) on the grid of the tile extent, as in the GeoJSON of version 3. A position exactly halfway between two numbers rounds up, where version 3 rounded it away from zero, so -2.5 is -2 instead of -3. An item of the buffer around the tile is below 0 or above the extent.

A cluster's number of points is `cluster.total_count`, a `u32`, and its center is `cluster.coordinates()`, computed only when you call it. You cannot build `Cluster`, `Tile` or `TileItem` yourself, so we can add fields to them without breaking your code.

### Cluster IDs

A cluster ID is a `ClusterId` that holds a `u32`, instead of a `usize`, so a point index cannot be passed by mistake. Turn the number a client sends back into an ID, and an ID into the number to send.

```rust,ignore
// Version 3
let children = supercluster.get_children(number)?;

// Version 4
let children = supercluster.children(ClusterId::new(number))?;
let number: u32 = cluster.id.get();
```

`ClusterId::from` and `u32::from` convert the same way. The numbers differ from the ones of version 3, and from one load to the next, so do not keep them across loads.

### Saved superclusters

With the `serde` feature, version 4 cannot read a supercluster that version 3 wrote, because the layout of the data changed. Load the points again, and write the new supercluster. Version 4 writes the version of its format first, so a later version that changes the layout gives an error instead of reading the data wrong.

### Imports

The modules are not public anymore, so import everything from the crate root. `KDBush` is not public anymore either.

```rust,ignore
// Version 3
use supercluster::supercluster::Supercluster;

// Version 4
use supercluster::Supercluster;
```

### Logging

The `tracing` feature reports one summary per load, with fields such as the number of points, and never reports queries, where version 3 logged every query.

### New in version 4

`with_filter` returns a view whose `tile` and `cluster_expansion_zoom` keep only the points that pass a filter. `filter` runs a closure on each query, `cache_filter` runs one once for many tiles, and cached filters combine with `and`, `or` and `and_not`. `with_cluster_max_zoom` lowers the zoom level to cluster on for a single request. See [Filter what a tile shows](#filter-what-a-tile-shows) and [Lower the maximum zoom level for one request](#lower-the-maximum-zoom-level-for-one-request).

## Benchmarks

We use the [`criterion` crate](https://docs.rs/criterion) to measure the speed of the crate. Run the benchmarks with:

```sh
cargo bench --features geojson
```

They measure:

- **Getting a tile:** retrieving a tile from a supercluster.
- **Getting clusters:** retrieving clusters for a given bounding box and zoom level.
- **Loading points:** loading points into a supercluster.
- **Filtering tiles:** filtering a tile with a `Filter`, with a `CachedFilter` that `cache_filter` made, and with a combined filter of cached filters.
- **One million points:** loading a million points, and getting and filtering tiles from them.
- **Comparison with version 3:** loading and the queries of version 3.0.8, on the same points. It runs only when you name it: `cargo bench --features geojson --bench v3_comparison_bench`.
- **What a request costs:** the table of [What a request costs](#what-a-request-costs), on every zoom level. It runs only when you name it: `cargo bench --bench request_cost_bench`.

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
