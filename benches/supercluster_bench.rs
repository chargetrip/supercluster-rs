use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use geojson::{Feature, Geometry};
use supercluster::{
    CoordinateSystem, Item, Supercluster, SuperclusterBuilder, SuperclusterOptions,
};

/// Create the options the benchmarks use.
fn create_options() -> SuperclusterOptions {
    SuperclusterBuilder::new()
        .radius(40.0)
        .extent(512.0)
        .min_points(2)
        .max_zoom(16)
        .coordinate_system(CoordinateSystem::LatLng)
        .build()
}

/// Create points that are spread over the world, without a random number generator so every run
/// uses the same points.
fn create_points(count: usize) -> Vec<[f64; 2]> {
    (0..count)
        .map(|i| {
            let lng = (i as f64 * 0.618_033_988_749_895) % 1.0;
            let lat = (i as f64 * 0.754_877_666_246_693) % 1.0;

            [-180.0 + 360.0 * lng, -80.0 + 160.0 * lat]
        })
        .collect()
}

fn bench_supercluster(c: &mut Criterion) {
    let features: Vec<Feature> = [[102.0, 0.5], [103.0, 1.0], [104.0, 0.0]]
        .into_iter()
        .map(|point| Feature::from(Geometry::new_point(point)))
        .collect();

    let mut supercluster = Supercluster::new(create_options());

    c.bench_function("load features", |b| {
        b.iter(|| {
            let _ = supercluster.load_features(black_box(features.clone()));
        })
    });

    c.bench_function("get tile", |b| {
        b.iter(|| {
            let _ = supercluster.get_tile(black_box(0), black_box(0.0), black_box(0.0));
        })
    });

    c.bench_function("get clusters", |b| {
        b.iter(|| {
            let _ = supercluster.get_clusters(black_box([101.0, 0.0, 105.0, 2.0]), black_box(2));
        })
    });
}

fn bench_many_points(c: &mut Criterion) {
    let points = create_points(10_000);
    let mut supercluster = Supercluster::new(create_options());

    c.bench_function("load 10000 points", |b| {
        b.iter(|| {
            let _ = supercluster.load_with(black_box(points.clone()), |point| Some(*point));
        })
    });

    c.bench_function("get tile of 10000 points", |b| {
        b.iter(|| {
            let _ = supercluster.get_tile(black_box(3), black_box(4.0), black_box(3.0));
        })
    });

    let cluster_id = supercluster
        .get_clusters([-180.0, -90.0, 180.0, 90.0], 0)
        .iter()
        .find_map(|item| match item {
            Item::Cluster(cluster) => Some(cluster.id),
            Item::Point { .. } => None,
        })
        .expect("the points form a cluster on zoom level 0");

    c.bench_function("get leaves of a cluster of 10000 points", |b| {
        b.iter(|| {
            let _ = supercluster.get_leaves(black_box(cluster_id), black_box(10), black_box(0));
        })
    });

    c.bench_function("get cluster expansion zoom of 10000 points", |b| {
        b.iter(|| {
            let _ = supercluster.get_cluster_expansion_zoom(black_box(cluster_id));
        })
    });
}

criterion_group!(benches, bench_supercluster, bench_many_points);
criterion_main!(benches);
