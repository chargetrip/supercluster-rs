//! Run it with `cargo run --example tracing --features geojson,tracing` to see what a load reports.
//! An application that only uses the `log` crate sees the same as log records.

use geojson::{Feature, Geometry};
use supercluster::{CoordinateSystem, Supercluster, SuperclusterError, SuperclusterOptions};

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
    let options = SuperclusterOptions::builder()
        .radius(40.0)
        .extent(512.0)
        .min_points(2)
        .max_zoom(16)
        .coordinate_system(CoordinateSystem::LatLng)
        .build()?;

    // Load the features into a new supercluster
    let supercluster = Supercluster::load_features(options, features)?;

    // Querying does not log anything
    supercluster.tile(0, 0, 0);

    Ok(())
}
