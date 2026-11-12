use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};
use geojson::{Feature, Geometry};
use supercluster::{Item, Supercluster};

mod common;

use common::{create_options, create_points, read_items, runs_under_cargo_bench};

fn bench_supercluster(c: &mut Criterion) {
    let features: Vec<Feature> = [[102.0, 0.5], [103.0, 1.0], [104.0, 0.0]]
        .into_iter()
        .map(|point| Feature::from(Geometry::new_point(point)))
        .collect();

    let options = create_options();

    c.bench_function("load features", |b| {
        b.iter(|| Supercluster::load_features(options.clone(), black_box(features.clone())))
    });

    let supercluster = Supercluster::load_features(options, features).unwrap();

    c.bench_function("get tile", |b| {
        b.iter(|| {
            read_items(
                &supercluster
                    .tile(black_box(0), black_box(0), black_box(0))
                    .items,
                false,
            )
        })
    });

    c.bench_function("get tile as geojson", |b| {
        b.iter(|| {
            let tile = supercluster.tile(black_box(0), black_box(0), black_box(0));

            serde_json::to_vec(&tile.as_feature_collection()).unwrap()
        })
    });

    c.bench_function("get clusters", |b| {
        b.iter(|| {
            let _ = supercluster.clusters(black_box([101.0, 0.0, 105.0, 2.0]), black_box(2));
        })
    });
}

fn bench_many_points(c: &mut Criterion) {
    let points = create_points(10_000);
    let options = create_options();

    c.bench_function("load 10000 points", |b| {
        b.iter(|| {
            Supercluster::load(options.clone(), black_box(points.clone()), |point| {
                Some(*point)
            })
        })
    });

    let supercluster = Supercluster::load(options, points, |point| Some(*point)).unwrap();

    c.bench_function("get tile of 10000 points", |b| {
        b.iter(|| {
            read_items(
                &supercluster
                    .tile(black_box(3), black_box(4), black_box(3))
                    .items,
                false,
            )
        })
    });

    c.bench_function("get tile with expansion zooms of 10000 points", |b| {
        b.iter(|| {
            read_items(
                &supercluster
                    .tile(black_box(3), black_box(4), black_box(3))
                    .items,
                true,
            )
        })
    });

    let cluster_id = supercluster
        .clusters([-180.0, -90.0, 180.0, 90.0], 0)
        .iter()
        .find_map(|item| match item {
            Item::Cluster(cluster) => Some(cluster.id()),
            Item::Point { .. } => None,
        })
        .expect("the points form a cluster on zoom level 0");

    c.bench_function("get leaves of a cluster of 10000 points", |b| {
        b.iter(|| {
            supercluster
                .leaves(black_box(cluster_id))
                .map(|leaves| leaves.iter().take(black_box(10)).collect::<Vec<_>>())
        })
    });

    // About one point in three passes, so most clusters lose some of their points
    let filter = |point: &[f64; 2]| (point[0] * 1e6) as i64 % 3 == 0;

    c.bench_function(
        "get filtered tile with expansion zooms of 10000 points",
        |b| {
            b.iter(|| {
                let tile = supercluster
                    .with_filter(supercluster.filter(filter))
                    .unwrap()
                    .tile(black_box(3), black_box(4), black_box(3));

                read_items(&tile.items, true)
            })
        },
    );

    c.bench_function("get filtered tile with coordinates of 10000 points", |b| {
        b.iter(|| {
            let tile = supercluster
                .with_filter(supercluster.filter(filter))
                .unwrap()
                .tile(black_box(3), black_box(4), black_box(3));

            tile.items
                .iter()
                .map(|tile_item| tile_item.coordinates()[0])
                .sum::<f64>()
        })
    });

    let cached = supercluster.cache_filter(filter);

    c.bench_function("cache filter of 10000 points", |b| {
        b.iter(|| supercluster.cache_filter(filter))
    });

    c.bench_function(
        "get cached filter tile with expansion zooms of 10000 points",
        |b| {
            b.iter(|| {
                read_items(
                    &supercluster
                        .with_filter(&cached)
                        .unwrap()
                        .tile(black_box(3), black_box(4), black_box(3))
                        .items,
                    true,
                )
            })
        },
    );

    c.bench_function("get cluster expansion zoom of 10000 points", |b| {
        b.iter(|| {
            let _ = supercluster.cluster_expansion_zoom(black_box(cluster_id));
        })
    });
}

fn bench_one_million_points(c: &mut Criterion) {
    if !runs_under_cargo_bench() {
        return;
    }

    let points = create_points(1_000_000);
    let options = create_options();
    let mut group = c.benchmark_group("one million points");

    // Each load is slow, so the group uses the smallest sample size criterion allows, with a
    // longer measurement time so it can still fit that many samples.
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));

    group.bench_function("load", |b| {
        b.iter(|| {
            Supercluster::load(options.clone(), black_box(points.clone()), |point| {
                Some(*point)
            })
        })
    });

    let supercluster = Supercluster::load(options, points, |point| Some(*point)).unwrap();

    // About one point in three passes, so most clusters lose some of their points
    let filter = |point: &[f64; 2]| (point[0] * 1e6) as i64 % 3 == 0;

    // The world tile holds the largest clusters, where filtering and expanding cost the most
    group.bench_function("get filtered tile with expansion zooms", |b| {
        b.iter(|| {
            let tile = supercluster
                .with_filter(supercluster.filter(filter))
                .unwrap()
                .tile(black_box(0), black_box(0), black_box(0));

            read_items(&tile.items, true)
        })
    });

    // A filtered cluster moves to the center of the points that pass, which the largest clusters of
    // the world tile take the longest to find
    group.bench_function("get filtered tile with coordinates", |b| {
        b.iter(|| {
            let tile = supercluster
                .with_filter(supercluster.filter(filter))
                .unwrap()
                .tile(black_box(0), black_box(0), black_box(0));

            tile.items
                .iter()
                .map(|tile_item| tile_item.coordinates()[0])
                .sum::<f64>()
        })
    });

    group.bench_function("cache filter", |b| {
        b.iter(|| supercluster.cache_filter(filter))
    });

    let cached = supercluster.cache_filter(filter);

    group.bench_function("get cached filter tile with expansion zooms", |b| {
        b.iter(|| {
            read_items(
                &supercluster
                    .with_filter(&cached)
                    .unwrap()
                    .tile(black_box(0), black_box(0), black_box(0))
                    .items,
                true,
            )
        })
    });

    group.bench_function("get tile", |b| {
        b.iter(|| {
            read_items(
                &supercluster
                    .tile(black_box(6), black_box(32), black_box(21))
                    .items,
                false,
            )
        })
    });

    // A click on the first cluster of the world, counting only the points that pass
    let cluster_id = supercluster
        .clusters([-180.0, -90.0, 180.0, 90.0], 0)
        .iter()
        .find_map(|item| match item {
            Item::Cluster(cluster) => Some(cluster.id()),
            Item::Point { .. } => None,
        })
        .expect("the points form a cluster on zoom level 0");

    group.bench_function("get filtered cluster expansion zoom", |b| {
        b.iter(|| {
            supercluster
                .with_filter(supercluster.filter(filter))
                .unwrap()
                .cluster_expansion_zoom(black_box(cluster_id))
        })
    });

    group.bench_function("get cached filter cluster expansion zoom", |b| {
        b.iter(|| {
            supercluster
                .with_filter(&cached)
                .unwrap()
                .cluster_expansion_zoom(black_box(cluster_id))
        })
    });

    // Cached filters made once per attribute, combined for every request
    let north = supercluster.cache_filter(|point| point[1] > 0.0);

    group.bench_function("cache the combined filter of two cached filters", |b| {
        b.iter(|| cached.and(black_box(&north)).cache())
    });

    // The query evaluates the combined filter on the points of the tile, all of them for the world
    let combined = cached.and(&north);

    group.bench_function(
        "get tile filtered by a combined filter with expansion zooms",
        |b| {
            b.iter(|| {
                read_items(
                    &supercluster
                        .with_filter(&combined)
                        .unwrap()
                        .tile(black_box(0), black_box(0), black_box(0))
                        .items,
                    true,
                )
            })
        },
    );

    group.finish();
}

criterion_group!(
    benches,
    bench_supercluster,
    bench_many_points,
    bench_one_million_points
);
criterion_main!(benches);
