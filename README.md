# Supercluster

[![Test status](https://github.com/chargetrip/supercluster-rs/actions/workflows/test.yml/badge.svg)](https://github.com/chargetrip/supercluster-rs/actions/workflows/test.yml)
[![Documentation on docs.rs](https://docs.rs/supercluster/badge.svg)](https://docs.rs/supercluster)
[![Latest version on crates.io](https://img.shields.io/crates/v/supercluster.svg)](https://crates.io/crates/supercluster)
![Downloads from crates.io](https://img.shields.io/crates/d/supercluster)
![License](https://img.shields.io/github/license/chargetrip/supercluster-rs)
[![Test coverage on Codecov](https://codecov.io/gh/chargetrip/supercluster-rs/graph/badge.svg?token=0S31CZY2ZJ)](https://codecov.io/gh/chargetrip/supercluster-rs)

A high-performance Rust crate for geospatial and non-geospatial point clustering. It groups nearby points into clusters on every zoom level, so a map can show millions of points as a few clusters.

This README is for Rust developers who show many points on a map or a chart. For every type and method, read the [API documentation on docs.rs](https://docs.rs/supercluster). If you encounter any issues or have questions that are not addressed in the documentation, feel free to [submit an issue on GitHub](https://github.com/chargetrip/supercluster-rs/issues).

<https://github.com/user-attachments/assets/d99a7e82-f981-4e31-a327-1ff87631971c>

<sub>Clusters of points on a map, changing as the map zooms. From [go.chargetrip.com](https://go.chargetrip.com).</sub>

## Contents

- [Overview](#overview)
- [Install](#install)
- [Cluster your points](#cluster-your-points)
- [Filter what a tile shows](#filter-what-a-tile-shows)
  - [Choose how to filter](#choose-how-to-filter)
  - [Filter with a closure](#filter-with-a-closure)
  - [Filter with selections](#filter-with-selections)
- [More ways to use it](#more-ways-to-use-it)
  - [Expand a cluster that a user clicks](#expand-a-cluster-that-a-user-clicks)
  - [Map and reduce the points](#map-and-reduce-the-points)
  - [Load and return GeoJSON](#load-and-return-geojson)
  - [Cluster data that is not on a map](#cluster-data-that-is-not-on-a-map)
  - [Lower the zoom level for one request](#lower-the-zoom-level-for-one-request)
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
- **Cluster IDs go to clients as numbers.** A cluster ID is a `ClusterId`. Send its number to a client with `get()` or `u32::from`, and turn the number the client sends back into an ID with `ClusterId::new` or `ClusterId::from`. The numbers change from one load to the next. See [Expand a cluster that a user clicks](#expand-a-cluster-that-a-user-clicks).

### Set the options

`SuperclusterOptions::default()` gives these values, and `SuperclusterOptions::builder()` changes the ones you set. `build()` checks every value and returns an error for one it cannot use. Read an option with the method of the same name, such as `options.extent()`. The options cannot change once they are built.

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
    assert_eq!(options.extent(), 512.0);

    Ok(())
}
```

## Filter what a tile shows

A filtered tile keeps only the points that pass a filter, for example the ones that match a search. A cluster without any point left is left out, a cluster with one point left becomes that point, and a cluster with more points left moves to the exact center of the points left.

Each item has `count()` and `leaves()` for the points that pass, `coordinates()` for its position in your coordinate system, and `expansion_zoom()` for the zoom level where a cluster splits, counting only the points that pass. Map the items to the type of your response with an iterator.

### Choose how to filter

There are three ways to filter a tile, and a combination can also be computed and cached. They differ in what they cache and in what runs on each request, so the right one depends on what you can prepare before the requests come in, and on which tiles get the most requests.

| Way | What is cached | What runs on each request | When to refresh it |
| --- | --- | --- | --- |
| A closure, with `tile_filtered` | Nothing. | Your filter, on every point under the tile. For a cluster with points that fail, it also reads the positions of the points that pass, or of the ones that fail when they are fewer, to find their center. | Never, because nothing is cached. |
| A selection, made with `select` | The result of your filter for every point, with running sums of their positions. | A count of the points of each cluster that pass, and their center, with a few lookups, however many points the tile covers. | When the data your filter reads changes. |
| A combination of selections, such as `a.or(&b).and(&c)` | Nothing of its own. It reads the selections it combines, which are cached. | The combination, on the points under the tile only. Like a closure, it reads positions to find the center of a cluster with points that fail. | Never. Refresh the selections it combines instead. |
| A computed combination, made with `compute` | The result of the combination for every point, as a selection. | A count of the points of each cluster that pass, and their center, with a few lookups, like a selection. | After you refresh one of the selections it combines. |

Within one request, the items keep what the query found, so their `leaves()` and `expansion_zoom()` do not run the filter or combine the selections again.

| If your filter | Use | Because |
| --- | --- | --- |
| Changes with every request, such as a text search | A closure | Nothing can be prepared. It is as fast as the others on zoomed-in tiles, but on a zoomed-out tile it runs on every point the tile covers. |
| Is one condition that you can prepare | A selection | It is the fastest way on zoomed-out tiles, and about as fast as a closure on zoomed-in ones. |
| Mixes conditions that you can prepare, differently per request | A combination, made per request | On zoomed-out tiles it is far faster than a closure, and on zoomed-in ones it takes at most about 3 µs more. It needs no refresh of its own. |
| Mixes them the same way on most zoomed-out requests | That combination, computed once with `compute` and cached | A zoomed-out tile covers the most points, so combining costs the most there, and a cached selection counts them in constant time. |

#### What a request costs

This benchmark times one request on 10,000,000 points spread around 3,000 places, on a Mac with Apple silicon. The request filters with a combination of four selections, `a.or(&b).and(&c).and(&d)`, and asks every item for its `expansion_zoom()`. At each zoom level, it uses the tile that holds the busiest place. In brackets is how many times faster than the closure each way is, where below 1 means slower. Run it with `cargo bench --bench request_cost_bench`.

| Zoom | Points under the tile | Closure | Combination | Cached selection |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 10,000,000 | 40,147 µs | 2,919 µs (14x) | 1.73 µs (23,274x) |
| 4 | 341,224 | 1,278 µs | 73.4 µs (17x) | 2.20 µs (581x) |
| 8 | 183,438 | 688 µs | 36.9 µs (19x) | 1.07 µs (643x) |
| 12 | 39,806 | 145 µs | 19.2 µs (7.6x) | 6.76 µs (22x) |
| 14 | 3,259 | 11.6 µs | 12.9 µs (0.9x) | 6.87 µs (1.7x) |
| 16 | 184 | 1.61 µs | 3.51 µs (0.5x) | 1.15 µs (1.4x) |
| 18 | 10 | 0.28 µs | 0.69 µs (0.4x) | 0.29 µs (1.0x) |

- **Zoomed out, a closure is slow.** It runs on every point the tile covers, which takes 40.1 ms for the tile of the whole world.
- **Zoomed out, a combination takes milliseconds too.** It reads the positions of the points that pass, or of the ones that fail when they are fewer, to find the exact centers, which takes 2.9 ms for the tile of the whole world and 864 µs on zoom level 1. A cached selection takes 1.73 µs there, so cache the combinations that zoomed-out tiles use.
- **Zoomed in, every way is fast.** From zoom level 14, every way takes 12.9 µs or less.
- **A cached selection is the fastest way up to zoom level 16.** Above it, a cached selection and a closure take about as long.
- **Computing a combination for every request does not pay.** It took about 3 ms at every zoom level. That is about what the combination takes on the tile of the whole world, and more than it takes on any other tile.

<details>
<summary>The whole benchmark, with zoom levels 0 to 30</summary>

The "Computed per query" column computes the combination with `compute` on every request. From zoom level 20, the tile that holds the busiest place has no point in it, so those rows time an empty tile.

| Zoom | Points under the tile | Closure | Computed per query | Lazy combination | Cached selection |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 10,000,000 | 40,147 µs | 2,991 µs (13x) | 2,919 µs (14x) | 1.73 µs (23,274x) |
| 1 | 3,149,699 | 12,403 µs | 3,017 µs (4.1x) | 864 µs (14x) | 1.60 µs (7,748x) |
| 2 | 1,189,182 | 4,617 µs | 2,987 µs (1.5x) | 272 µs (17x) | 2.15 µs (2,147x) |
| 3 | 760,516 | 2,910 µs | 2,985 µs (1.0x) | 168 µs (17x) | 4.29 µs (678x) |
| 4 | 341,224 | 1,278 µs | 2,950 µs (0.4x) | 73.4 µs (17x) | 2.20 µs (581x) |
| 5 | 213,250 | 791 µs | 2,959 µs (0.3x) | 43.2 µs (18x) | 1.17 µs (674x) |
| 6 | 185,389 | 684 µs | 3,000 µs (0.2x) | 36.4 µs (19x) | 0.40 µs (1,700x) |
| 7 | 183,448 | 679 µs | 2,954 µs (0.2x) | 35.5 µs (19x) | 0.55 µs (1,231x) |
| 8 | 183,438 | 688 µs | 2,993 µs (0.2x) | 36.9 µs (19x) | 1.07 µs (643x) |
| 9 | 183,438 | 695 µs | 3,010 µs (0.2x) | 40.6 µs (17x) | 3.05 µs (228x) |
| 10 | 122,108 | 461 µs | 3,027 µs (0.2x) | 32.8 µs (14x) | 5.08 µs (91x) |
| 11 | 78,559 | 294 µs | 3,053 µs (0.1x) | 25.9 µs (11x) | 6.79 µs (43x) |
| 12 | 39,806 | 145 µs | 3,041 µs (0.0x) | 19.2 µs (7.6x) | 6.76 µs (22x) |
| 13 | 11,866 | 34.8 µs | 3,062 µs (0.0x) | 14.3 µs (2.4x) | 6.59 µs (5.3x) |
| 14 | 3,259 | 11.6 µs | 3,051 µs (0.0x) | 12.9 µs (0.9x) | 6.87 µs (1.7x) |
| 15 | 776 | 4.42 µs | 3,036 µs (0.0x) | 6.95 µs (0.6x) | 3.06 µs (1.4x) |
| 16 | 184 | 1.61 µs | 3,005 µs (0.0x) | 3.51 µs (0.5x) | 1.15 µs (1.4x) |
| 17 | 42 | 0.50 µs | 3,028 µs (0.0x) | 1.81 µs (0.3x) | 0.56 µs (0.9x) |
| 18 | 10 | 0.28 µs | 2,992 µs (0.0x) | 0.69 µs (0.4x) | 0.29 µs (1.0x) |
| 19 | 2 | 0.18 µs | 2,990 µs (0.0x) | 0.41 µs (0.4x) | 0.19 µs (1.0x) |
| 20 | 0 | 0.14 µs | 3,015 µs (0.0x) | 0.28 µs (0.5x) | 0.14 µs (1.0x) |
| 21 | 0 | 0.14 µs | 2,956 µs (0.0x) | 0.28 µs (0.5x) | 0.14 µs (1.0x) |
| 22 | 0 | 0.14 µs | 2,949 µs (0.0x) | 0.28 µs (0.5x) | 0.14 µs (1.0x) |
| 23 | 0 | 0.14 µs | 2,987 µs (0.0x) | 0.28 µs (0.5x) | 0.14 µs (1.0x) |
| 24 | 0 | 0.14 µs | 2,959 µs (0.0x) | 0.28 µs (0.5x) | 0.14 µs (1.0x) |
| 25 | 0 | 0.14 µs | 2,967 µs (0.0x) | 0.28 µs (0.5x) | 0.14 µs (1.0x) |
| 26 | 0 | 0.14 µs | 2,994 µs (0.0x) | 0.30 µs (0.4x) | 0.14 µs (1.0x) |
| 27 | 0 | 0.14 µs | 2,986 µs (0.0x) | 0.29 µs (0.5x) | 0.14 µs (1.0x) |
| 28 | 0 | 0.14 µs | 2,959 µs (0.0x) | 0.28 µs (0.5x) | 0.14 µs (1.0x) |
| 29 | 0 | 0.14 µs | 2,965 µs (0.0x) | 0.28 µs (0.5x) | 0.14 µs (1.0x) |
| 30 | 0 | 0.14 µs | 2,970 µs (0.0x) | 0.28 µs (0.5x) | 0.14 µs (1.0x) |

</details>

Your results depend on your points, your filters and your machine, so measure your own case before you choose.

#### What to keep in mind

- **A selection is a snapshot (cached) of what the filter returned.** If the filter only reads the points, it stays valid, because points do not change after `load`. If the filter reads other data that changes, run `select` again when it changes, for example every 30 seconds for data that changes often.
- **A computed combination is a snapshot too.** It does not follow the selections it was computed from, so compute it again after you refresh one of them.
- **Compute a combination only to cache it.** Computing it for every request costs more than letting the query combine it, except on the tile of the whole world, where both take about 3 ms, as the benchmark shows.
- **A selection belongs to the supercluster that made it.** Keep them together, for example in the same `Arc`, and replace both when you load new points. With a selection of another supercluster, a query does not panic, but its results mean nothing. The [`shared_state` example](https://github.com/chargetrip/supercluster-rs/blob/main/examples/shared_state.rs) shows a server that keeps them together, combines them per request and refreshes a selection when its data changes.

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

    // The cluster has one available station left, so the tile holds that station
    let items = supercluster.tile_filtered(0, 0, 0, |station| station.available).items;

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

Make one selection per condition with `select`, and combine them per request with `and`, `or` and `and_not`. A combination borrows the selections and computes nothing, and the query evaluates it on the points of its tile only. The items borrow the selections, not the combination, so you can pass the combination straight to the query. `compute` turns a combination into a selection of its own, which you can cache.

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

    // One selection per condition, made once and reused by every request
    let fast = supercluster.select(|station| station.power_kw >= 50);
    let available = supercluster.select(|station| station.available);

    // Per request: combine what the request asks for, which the query evaluates on its tile only
    let items = supercluster.tile_selected(0, 0, 0, fast.and(&available)).items;

    // One station is fast and available, so the cluster becomes that station
    assert!(matches!(items[0].item, Item::Point { index: 0, .. }));
    assert_eq!(items[0].count(), 1);

    Ok(())
}
```

## More ways to use it

### Expand a cluster that a user clicks

A client gets the number of every cluster ID with the tile, and sends it back when the user clicks the cluster. Turn the number back into a `ClusterId`, then zoom the map in to where the cluster splits, or list what it holds.

```rust
use supercluster::{ClusterId, Item, Supercluster, SuperclusterError, SuperclusterOptions};

struct Station {
    lng: f64,
    lat: f64,
}

fn main() -> Result<(), SuperclusterError> {
    let supercluster = Supercluster::load(
        SuperclusterOptions::default(),
        vec![Station { lng: 4.9, lat: 52.37 }, Station { lng: 4.91, lat: 52.37 }],
        |station| Some([station.lng, station.lat]),
    )?;

    for tile_item in &supercluster.tile(0, 0, 0).items {
        if let Item::Cluster(cluster) = tile_item.item {
            // The number the client sends back after a click
            let number = cluster.id.get();
            let cluster_id = ClusterId::new(number);

            // The zoom level to zoom the map in to, where the cluster splits
            let zoom = supercluster.cluster_expansion_zoom(cluster_id)?;

            // Or the clusters and points it splits into, and all of its points
            let children = supercluster.children(cluster_id)?.len();
            let points = supercluster.leaves(cluster_id)?.len();

            println!("Zoom in to {zoom}: {children} children, {points} points");
        }
    }

    Ok(())
}
```

The numbers change from one load to the next. After you load new points, a number from before can name no cluster, which gives `ClusterNotFound`, or name another cluster. On a filtered map, `cluster_expansion_zoom_filtered` and `cluster_expansion_zoom_selected` find the zoom level for the points that pass.

### Map and reduce the points

Mapbox’s supercluster combines the values of points into clusters while it loads, with its `map` and `reduce` options. This crate has no such options. Here you combine the values when you need them, which also works on filtered tiles:

- Instead of `map`, read the fields of your points directly, they're your own type.
- Instead of `reduce` to count matching points, filter the tile with `tile_filtered` or `tile_selected`, and read each item's `count()`.
- Instead of `reduce` to combine values, like a sum, compute them from `supercluster.leaves(cluster.id)`, or from `filtered_tile_item.leaves()` for only the points that pass the filter.

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
    for filtered_tile_item in &supercluster.tile_filtered(0, 0, 0, |station| station.available).items {
        let capacity: u32 = match filtered_tile_item.item {
            // Instead of map: a point is your own type, so read its fields
            Item::Point { point, .. } => point.capacity,
            // Instead of reduce: combine the fields of the points of the cluster that pass
            Item::Cluster(_) => filtered_tile_item.leaves().map(|station| station.capacity).sum(),
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
| A filtered tile | The same calls, on what `tile_filtered` or `tile_selected` returns | The position in the tile, at the center of the points that pass. |
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

To send a tile, serialize `as_feature_collection()`, as the example does with `serde_json`, which you add to your own dependencies. It writes the same JSON as `to_feature_collection()`, but borrows the properties of the features instead of copying them. Use `to_feature_collection()` when you change the features before you send them.

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

## API quick reference

The [API documentation on docs.rs](https://docs.rs/supercluster) describes every item in full. Import everything from the crate root, such as `use supercluster::Supercluster;`. In the tables, `_` in a `Result` stands for `SuperclusterError`.

### Load points

| Call | Returns | What it does |
| --- | --- | --- |
| `Supercluster::load(options, points, get_coordinates)` | `Result<Supercluster<P>, _>` | Clusters points of any type. `get_coordinates` returns the coordinates of a point, or `None` to leave it out. |
| `Supercluster::load_features(options, features)` | `Result<Supercluster<Feature>, _>` | Clusters GeoJSON features, with the `geojson` feature. |
| `SuperclusterOptions::default()` | `SuperclusterOptions` | The default options, listed in [Set the options](#set-the-options). |
| `SuperclusterOptions::builder()` | `SuperclusterOptionsBuilder` | Sets the options one by one. Its `build()` checks them and returns `Result<SuperclusterOptions, _>`. |
| `CoordinateSystem::LatLng` | `CoordinateSystem` | Longitudes and latitudes. This is the default. |
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

These are methods of `Supercluster<P>`, with the same last column as the queries. The `selection` of a query is a reference to a `Selection`, also behind an `Arc` or a `Box`, or a `Combination`, by value or by reference.

| Method | Returns | What it does | On a view |
| --- | --- | --- | --- |
| `tile_filtered(z, x, y, filter)` | `FilteredTile` | The items of a tile that keep a point for which the closure returns `true`. | Yes |
| `cluster_expansion_zoom_filtered(cluster_id, filter)` | `Result<u8, _>` | The zoom level where a cluster splits, counting only the points that pass the closure. | Yes |
| `select(filter)` | `Selection` | The points that pass the closure, found once for many queries. | No |
| `tile_selected(z, x, y, selection)` | `FilteredTile` | The items of a tile that keep a point of the selection. | Yes |
| `cluster_expansion_zoom_selected(cluster_id, selection)` | `Result<u8, _>` | The zoom level where a cluster splits, counting only the points of the selection. | Yes |

Selections combine with the methods below, and `count()` gives the number of points of a selection. `a` is a `Selection` or a `Combination`, and `b` is a reference to a `Selection`, also behind an `Arc` or a `Box`, or a `Combination`, by value or by reference. The calls chain, as in `a.or(&b).and(&c)`. A combination computes nothing until a query or `compute` evaluates it.

| Method | Returns | What it does |
| --- | --- | --- |
| `a.and(b)` | `Combination` | The points in both `a` and `b`. |
| `a.or(b)` | `Combination` | The points in `a`, in `b`, or in both. |
| `a.and_not(b)` | `Combination` | The points in `a` that are not in `b`. |
| `combination.compute()` | `Selection` | The combination computed for every point, to cache. |
| `selection.count()` | `u32` | The number of points of the selection. |

### Read the results

| Type | What it holds | Methods |
| --- | --- | --- |
| `Tile` | `items`, a list of `TileItem`. | `is_empty()` |
| `TileItem` | `x` and `y`, the position in the tile, and `item`. | `count()` for its points, and `expansion_zoom()` |
| `Item` | `Item::Cluster(Cluster)`, or `Item::Point { index, point }`, a point with its index in the list that was loaded. | None |
| `Cluster` | `id` and `point_count`. | `coordinates()` for its center |
| `ClusterId` | The ID of a cluster, as a `u32`. `ClusterId::new(number)` makes one from a `u32`. | `get()` returns the `u32` |
| `FilteredTile` | `items`, a list of `FilteredTileItem`. | `is_empty()` |
| `FilteredTileItem` | `x`, `y` and `item`, like a `TileItem`. | `coordinates()`, `count()` and `leaves()` for the points that pass, and `expansion_zoom()` |

### Convert to GeoJSON

These methods need the `geojson` feature.

| Method | Returns | What it does |
| --- | --- | --- |
| `tile.to_feature_collection()` | `FeatureCollection` | The items of the tile, with their positions in the tile as the geometry. |
| `tile.as_feature_collection()` | `FeatureCollectionView` | Serializes to the same JSON as `to_feature_collection()`, without copying the properties. |
| `tile_item.to_feature()` | `Feature` | The item, with its position in the tile as the geometry. |
| `filtered_tile.to_feature_collection()`, `as_feature_collection()` and `filtered_tile_item.to_feature()` | Like the ones of a tile | The items of a filtered tile, at their positions in the tile. |
| `item.to_feature()` | `Feature` | A cluster at its center, or the point as it was loaded. |
| `cluster.to_feature()` | `Feature` | The cluster at its center, with the properties `cluster`, `cluster_id` and `point_count`. |

### Errors and limits

| Item | What it means |
| --- | --- |
| `SuperclusterError::ClusterNotFound` | No cluster of the supercluster has the ID. |
| `SuperclusterError::TooManyPoints` | `load` got more than `MAX_POINTS` points. |
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

### Options

Build the options with `SuperclusterOptions::builder()`, or take the defaults. `build()` returns a `Result`. It gives an error for a `max_zoom` above 30, a `min_zoom` above the `max_zoom`, a radius or extent that is not a finite number above 0, a `node_size` of 0 and a `DataRange` that cannot scale the data. Deserializing the options with the `serde` feature checks the same values. A struct literal cannot build them anymore, and their fields are private, so they cannot change once they are checked. Read them with the methods of the same names, such as `options.max_zoom()`. `min_points` is a `u32` instead of a `u8`.

```rust,ignore
// Version 3
let options = Supercluster::builder().radius(40.0).max_zoom(16).build();

// Version 4
let options = SuperclusterOptions::builder().radius(40.0).max_zoom(16).build()?;

// Or take the defaults
let options = SuperclusterOptions::default();
```

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
- `tile` does not show items from the opposite edge of the data on the first and last tile for the `Cartesian` coordinate system, because the two ends of its range are not next to each other. `LatLng` still does.

### Typed results and GeoJSON

The queries return `Item`s and a `Tile`. Match on them, or convert them into GeoJSON with the `geojson` feature. The geometry of a tile feature holds the position in the tile, as in version 3.

```rust,ignore
// Version 3
let tile = supercluster.get_tile(0, 0.0, 0.0)?;

// Version 4
let tile = supercluster.tile(0, 0, 0).to_feature_collection();

for tile_item in &supercluster.tile(0, 0, 0).items {
    match tile_item.item {
        Item::Cluster(cluster) => println!("{} points at {:?}", cluster.point_count, cluster.coordinates()),
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

You cannot build `Cluster`, `Tile` or `TileItem` yourself, so we can add fields to them without breaking your code. The `point_count` of a `Cluster` is a `u32`, and `coordinates()` computes its center when you ask for it, so a tile that only needs positions in the tile does not pay for it.

### Cluster IDs

A cluster ID is a `ClusterId` that holds a `u32`, instead of a `usize`, so a point index cannot be passed by mistake. Turn the number a client sends back into an ID, and an ID into the number to send.

```rust,ignore
// Version 3
let children = supercluster.get_children(number)?;

// Version 4
let children = supercluster.children(ClusterId::new(number))?;
let number: u32 = cluster.id.get();
```

`ClusterId::from` and `u32::from` convert the same way. The numbers differ from the ones of version 3 and of the JavaScript supercluster, and from one load to the next, so do not keep them across loads.

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
- **What a request costs:** the tables of [What a request costs](#what-a-request-costs), which time one request on every zoom level. It runs only when you name it: `cargo bench --bench request_cost_bench`.

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
