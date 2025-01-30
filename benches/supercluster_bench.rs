use criterion::{black_box, criterion_group, criterion_main, Criterion};
use geojson::Feature;
use supercluster::{CoordinateSystem, Supercluster};

fn create_feature_collection() -> Vec<Feature> {
    Supercluster::feature_builder()
        .add_point(vec![102.0, 0.5])
        .add_point(vec![103.0, 1.0])
        .add_point(vec![104.0, 0.0])
        .build()
}

fn bench_supercluster(c: &mut Criterion) {
    let feature_collection = create_feature_collection();

    let options = Supercluster::builder()
        .radius(40.0)
        .extent(512.0)
        .min_points(2)
        .max_zoom(16)
        .coordinate_system(CoordinateSystem::LatLng)
        .build();

    let mut cluster = Supercluster::new(options);

    c.bench_function("load feature collection", |b| {
        b.iter(|| {
            let _ = cluster.load(black_box(feature_collection.clone()));
        })
    });

    let index = cluster.load(feature_collection).unwrap();

    c.bench_function("get tile", |b| {
        b.iter(|| {
            let _ = index.get_tile(black_box(0), black_box(0.0), black_box(0.0));
        })
    });

    c.bench_function("get clusters", |b| {
        b.iter(|| {
            let _ = index.get_clusters(black_box([101.0, 0.0, 105.0, 2.0]), black_box(2));
        })
    });
}

criterion_group!(benches, bench_supercluster);
criterion_main!(benches);
