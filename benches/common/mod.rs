use supercluster::{CoordinateSystem, Item, Options, TileItem};

/// Create the options the benchmarks use.
pub fn create_options() -> Options {
    Options::builder()
        .radius(40.0)
        .extent(512)
        .min_points(2)
        .max_zoom(16)
        .coordinate_system(CoordinateSystem::LngLat)
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

/// Read what a server reads from the items of a tile: the cluster or point of every item, and with
/// `expansion_zooms`, the zoom level where every cluster splits. Every benchmark of a tile reads its
/// items with it, so it measures what a request pays for, not only building the tile.
pub fn read_items<P>(items: &[TileItem<'_, P>], expansion_zooms: bool) -> u32 {
    items
        .iter()
        .map(|tile_item| {
            let read = match tile_item.item() {
                Item::Cluster(cluster) => cluster.total_count(),
                Item::Point { index, .. } => index as u32,
            };
            let expansion_zoom = match expansion_zooms {
                true => tile_item.expansion_zoom().map_or(0, u32::from),
                false => 0,
            };

            read + expansion_zoom
        })
        .sum()
}
