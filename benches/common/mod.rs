use supercluster::{CoordinateSystem, SuperclusterOptions};

/// Create the options the benchmarks use.
pub fn create_options() -> SuperclusterOptions {
    SuperclusterOptions::builder()
        .radius(40.0)
        .extent(512.0)
        .min_points(2)
        .max_zoom(16)
        .coordinate_system(CoordinateSystem::LatLng)
        .build()
        .unwrap()
}

/// Create points spread over the world with a fixed formula, so every run uses the same points.
pub fn create_points(count: usize) -> Vec<[f64; 2]> {
    (0..count)
        .map(|i| {
            let lng = (i as f64 * 0.618_033_988_749_895) % 1.0;
            let lat = (i as f64 * 0.754_877_666_246_693) % 1.0;

            [-180.0 + 360.0 * lng, -80.0 + 160.0 * lat]
        })
        .collect()
}

/// Check whether `cargo bench` runs the benchmarks. `cargo test` also runs each benchmark once, as a
/// check, which takes too long and too much memory for a million points.
pub fn runs_under_cargo_bench() -> bool {
    std::env::args().any(|argument| argument == "--bench")
}
