use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};
use geojson::{Feature, Geometry};
use supercluster::{Item, Supercluster};

mod common;

use common::{create_options, create_points, runs_under_cargo_bench};

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
            let _ = supercluster.tile(black_box(0), black_box(0), black_box(0));
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
            let _ = supercluster.tile(black_box(3), black_box(4), black_box(3));
        })
    });

    let cluster_id = supercluster
        .clusters([-180.0, -90.0, 180.0, 90.0], 0)
        .iter()
        .find_map(|item| match item {
            Item::Cluster(cluster) => Some(cluster.id),
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
                supercluster
                    .tile_filtered(black_box(3), black_box(4), black_box(3), filter)
                    .iter()
                    .filter_map(|tile_item| tile_item.expansion_zoom())
                    .map(u32::from)
                    .sum::<u32>()
            })
        },
    );

    let selection = supercluster.select(filter);

    c.bench_function("select 10000 points", |b| {
        b.iter(|| supercluster.select(filter))
    });

    c.bench_function(
        "get selected tile with expansion zooms of 10000 points",
        |b| {
            b.iter(|| {
                supercluster
                    .tile_selected(black_box(3), black_box(4), black_box(3), &selection)
                    .iter()
                    .filter_map(|tile_item| tile_item.expansion_zoom())
                    .map(u32::from)
                    .sum::<u32>()
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
            supercluster
                .tile_filtered(black_box(0), black_box(0), black_box(0), filter)
                .iter()
                .filter_map(|tile_item| tile_item.expansion_zoom())
                .map(u32::from)
                .sum::<u32>()
        })
    });

    group.bench_function("select", |b| b.iter(|| supercluster.select(filter)));

    let selection = supercluster.select(filter);

    group.bench_function("get selected tile with expansion zooms", |b| {
        b.iter(|| {
            supercluster
                .tile_selected(black_box(0), black_box(0), black_box(0), &selection)
                .iter()
                .filter_map(|tile_item| tile_item.expansion_zoom())
                .map(u32::from)
                .sum::<u32>()
        })
    });

    group.bench_function("get tile", |b| {
        b.iter(|| {
            let _ = supercluster.tile(black_box(6), black_box(32), black_box(21));
        })
    });

    // A click on the first cluster of the world, counting only the points that pass
    let cluster_id = supercluster
        .clusters([-180.0, -90.0, 180.0, 90.0], 0)
        .iter()
        .find_map(|item| match item {
            Item::Cluster(cluster) => Some(cluster.id),
            Item::Point { .. } => None,
        })
        .expect("the points form a cluster on zoom level 0");

    group.bench_function("get filtered cluster expansion zoom", |b| {
        b.iter(|| supercluster.cluster_expansion_zoom_filtered(black_box(cluster_id), filter))
    });

    group.bench_function("get selected cluster expansion zoom", |b| {
        b.iter(|| supercluster.cluster_expansion_zoom_selected(black_box(cluster_id), &selection))
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_supercluster,
    bench_many_points,
    bench_one_million_points
);
criterion_main!(benches);
