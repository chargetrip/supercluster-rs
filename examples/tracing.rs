//! Run it with `cargo run --example tracing --features geojson,tracing` to see what a load reports.
//! An application that only uses the `log` crate sees the same as log records.

use geojson::{Feature, Geometry};
use supercluster::{CoordinateSystem, Supercluster, SuperclusterBuilder, SuperclusterError};

fn main() -> Result<(), SuperclusterError> {
    // Print every event of the supercluster, down to the trace level
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .init();

    // Create a list of features
    let mut features: Vec<Feature> = [[-77.032, 38.913], [-77.033, 38.913], [-77.034, 38.913]]
        .into_iter()
        .map(|point| Feature::from(Geometry::new_point(point)))
        .collect();

    // A feature without a point has no coordinates, so it is left out and a warning tells it
    features.push(Feature::default());

    // Set the configuration settings
    let options = SuperclusterBuilder::new()
        .radius(40.0)
        .extent(512.0)
        .min_points(2)
        .max_zoom(16)
        .coordinate_system(CoordinateSystem::LatLng)
        .build();

    // Create a new instance with the specified configuration settings
    let mut supercluster = Supercluster::new(options);

    // Load features into the Supercluster instance
    supercluster.load_features(features)?;

    // Querying does not log anything
    supercluster.get_tile(0, 0, 0);

    Ok(())
}
