//! Compare version 4 with version 3.0.8 on the same million points, options and queries.
//!
//! Version 3 takes GeoJSON features and returns them, so "v4" does the same, through the `geojson`
//! feature. "v4 points" clusters the points as they are and returns its own items, which only
//! version 4 can do. The versions use different versions of geojson, so each gets features of its
//! own, made from the same points in the same order.

use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use geojson::{Feature, Geometry};
use geojson_v3::{Feature as FeatureV3, Geometry as GeometryV3, Value as ValueV3};
use supercluster::{ClusterId, Item, Supercluster};
use supercluster_v3::{
    CoordinateSystem as CoordinateSystemV3, Supercluster as SuperclusterV3,
    SuperclusterOptions as SuperclusterOptionsV3,
};

mod common;

use common::{create_options, create_points, runs_under_cargo_bench};

/// The bounding box of the whole world.
const WORLD: [f64; 4] = [-180.0, -90.0, 180.0, 90.0];

/// Create the options of version 3, with the values of `create_options`.
fn create_options_v3() -> SuperclusterOptionsV3 {
    SuperclusterV3::builder()
        .radius(40.0)
        .extent(512.0)
        .min_points(2)
        .max_zoom(16)
        .coordinate_system(CoordinateSystemV3::LatLng)
        .build()
}

/// Turn points into GeoJSON features with a point geometry, the only input version 3 takes.
fn create_features(points: &[[f64; 2]]) -> Vec<Feature> {
    points
        .iter()
        .map(|&point| Feature::from(Geometry::new_point(point)))
        .collect()
}

/// Turn points into GeoJSON features of the version of geojson that version 3 uses.
fn create_features_v3(points: &[[f64; 2]]) -> Vec<FeatureV3> {
    points
        .iter()
        .map(|point| FeatureV3::from(GeometryV3::new(ValueV3::Point(point.to_vec()))))
        .collect()
}

/// Get the ID and the number of points of the first cluster of the whole world on zoom level 0,
/// from version 3.
fn find_cluster_v3(supercluster: &SuperclusterV3) -> (usize, u64) {
    supercluster
        .get_clusters(WORLD, 0)
        .unwrap()
        .iter()
        .find_map(|feature| {
            let id = feature.property("cluster_id")?.as_u64()?;
            let point_count = feature.property("point_count")?.as_u64()?;

            Some((id as usize, point_count))
        })
        .expect("the points form a cluster on zoom level 0")
}

/// Get the ID and the number of points of the first cluster of the whole world on zoom level 0,
/// from version 4.
fn find_cluster<P>(supercluster: &Supercluster<P>) -> (ClusterId, u64) {
    supercluster
        .clusters(WORLD, 0)
        .iter()
        .find_map(|item| match item {
            Item::Cluster(cluster) => Some((cluster.id, u64::from(cluster.point_count))),
            Item::Point { .. } => None,
        })
        .expect("the points form a cluster on zoom level 0")
}

/// Compare the versions on a million points: loading them, then the queries.
fn compare_one_million_points(c: &mut Criterion) {
    if !runs_under_cargo_bench() {
        return;
    }

    let points = create_points(1_000_000);
    let features = create_features(&points);
    let features_v3 = create_features_v3(&points);
    let options = create_options();

    let mut group = c.benchmark_group("load 1000000 points");

    // Each load takes seconds, so the group uses the smallest sample size criterion allows, with a
    // longer measurement time so it can still fit that many samples
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));

    // The input is copied outside the measurement, and the supercluster is dropped outside it
    group.bench_function("v3", |b| {
        b.iter_batched(
            || features_v3.clone(),
            |features| {
                let mut supercluster = SuperclusterV3::new(create_options_v3());

                supercluster.load(features).unwrap();
                supercluster
            },
            BatchSize::PerIteration,
        )
    });

    group.bench_function("v4", |b| {
        b.iter_batched(
            || features.clone(),
            |features| Supercluster::load_features(options.clone(), features).unwrap(),
            BatchSize::PerIteration,
        )
    });

    group.bench_function("v4 points", |b| {
        b.iter_batched(
            || points.clone(),
            |points| Supercluster::load(options.clone(), points, |point| Some(*point)).unwrap(),
            BatchSize::PerIteration,
        )
    });

    group.finish();

    let mut v3 = SuperclusterV3::new(create_options_v3());

    v3.load(features_v3).unwrap();

    let v4 = Supercluster::load_features(options.clone(), features).unwrap();
    let v4_points = Supercluster::load(options, points, |point| Some(*point)).unwrap();

    // The versions cluster the same way, so they find the same cluster, which version 4 numbers
    // another way
    let (cluster_v3, point_count) = find_cluster_v3(&v3);
    let (cluster, _) = find_cluster(&v4);

    assert_eq!(find_cluster(&v4), (cluster, point_count));
    assert_eq!(find_cluster(&v4_points), (cluster, point_count));

    // The tile the main benchmark uses for a million points
    let mut group = c.benchmark_group("get tile of 1000000 points");

    group.bench_function("v3", |b| {
        b.iter(|| v3.get_tile(black_box(6), black_box(32.0), black_box(21.0)))
    });

    group.bench_function("v4", |b| {
        b.iter(|| {
            v4.tile(black_box(6), black_box(32), black_box(21))
                .to_feature_collection()
        })
    });

    group.bench_function("v4 points", |b| {
        b.iter(|| v4_points.tile(black_box(6), black_box(32), black_box(21)))
    });

    group.finish();

    let mut group = c.benchmark_group("get clusters of 1000000 points");

    group.bench_function("v3", |b| {
        b.iter(|| v3.get_clusters(black_box(WORLD), black_box(3)))
    });

    group.bench_function("v4", |b| {
        b.iter(|| {
            v4.clusters(black_box(WORLD), black_box(3))
                .iter()
                .map(Item::to_feature)
                .collect::<Vec<_>>()
        })
    });

    group.bench_function("v4 points", |b| {
        b.iter(|| v4_points.clusters(black_box(WORLD), black_box(3)))
    });

    group.finish();

    let mut group = c.benchmark_group("get leaves of a cluster of 1000000 points");

    group.bench_function("v3", |b| {
        b.iter(|| v3.get_leaves(black_box(cluster_v3), black_box(10), 0))
    });

    group.bench_function("v4", |b| {
        b.iter(|| {
            v4.leaves(black_box(cluster)).map(|leaves| {
                leaves
                    .iter()
                    .take(black_box(10))
                    .cloned()
                    .collect::<Vec<_>>()
            })
        })
    });

    group.bench_function("v4 points", |b| {
        b.iter(|| {
            v4_points
                .leaves(black_box(cluster))
                .map(|leaves| leaves.iter().take(black_box(10)).collect::<Vec<_>>())
        })
    });

    group.finish();

    // The cluster has thousands of points, so this shows what it costs to get all of them
    let mut group = c.benchmark_group("get all leaves of a cluster of 1000000 points");

    group.bench_function("v3", |b| {
        b.iter(|| v3.get_leaves(black_box(cluster_v3), usize::MAX, 0))
    });

    group.bench_function("v4", |b| {
        b.iter(|| v4.leaves(black_box(cluster)).map(<[Feature]>::to_vec))
    });

    group.bench_function("v4 points", |b| {
        b.iter(|| {
            v4_points
                .leaves(black_box(cluster))
                .map(|leaves| leaves.iter().collect::<Vec<_>>())
        })
    });

    group.finish();

    let mut group = c.benchmark_group("get children of a cluster of 1000000 points");

    group.bench_function("v3", |b| b.iter(|| v3.get_children(black_box(cluster_v3))));

    group.bench_function("v4", |b| {
        b.iter(|| {
            v4.children(black_box(cluster))
                .map(|children| children.map(|child| child.to_feature()).collect::<Vec<_>>())
        })
    });

    group.bench_function("v4 points", |b| {
        b.iter(|| {
            v4_points
                .children(black_box(cluster))
                .map(|children| children.collect::<Vec<_>>())
        })
    });

    group.finish();

    // The expansion zoom does not depend on the type of the points, so "v4 points" is left out
    let mut group = c.benchmark_group("get cluster expansion zoom of 1000000 points");

    group.bench_function("v3", |b| {
        b.iter(|| v3.get_cluster_expansion_zoom(black_box(cluster_v3)))
    });

    group.bench_function("v4", |b| {
        b.iter(|| v4.cluster_expansion_zoom(black_box(cluster)))
    });

    group.finish();

    // Sending a tile as GeoJSON, in the fastest way each version has. Only GeoJSON is serialized,
    // so "v4 points" is left out
    let mut group = c.benchmark_group("serialize the tile of 1000000 points as GeoJSON");

    group.bench_function("v3", |b| {
        b.iter(|| {
            v3.get_tile(black_box(6), black_box(32.0), black_box(21.0))
                .map(|tile| serde_json::to_string(&tile))
        })
    });

    group.bench_function("v4", |b| {
        b.iter(|| {
            serde_json::to_string(
                &v4.tile(black_box(6), black_box(32), black_box(21))
                    .as_feature_collection(),
            )
        })
    });

    group.finish();
}

criterion_group!(benches, compare_one_million_points);
criterion_main!(benches);
