use std::error::Error;

use geojson::{Feature, Geometry};
use supercluster::Supercluster;

fn main() -> Result<(), Box<dyn Error>> {
    // Create a list of features
    let features: Vec<Feature> = [[-77.032, 38.913], [-77.033, 38.913], [-77.034, 38.913]]
        .into_iter()
        .map(|point| Feature::from(Geometry::new_point(point)))
        .collect();

    // Create supercluster options from a JSON string
    let options = serde_json::from_str(
        r#"{
            "radius": 40.0,
            "extent": 512.0,
            "min_points": 1,
            "node_size": 64,
            "min_zoom": 2,
            "max_zoom": 16,
            "coordinate_system": "LatLng"
        }"#,
    )?;

    // Load the features into a new supercluster
    let supercluster = Supercluster::load_features(options, features)?;

    // Get a tile
    let tile = supercluster.tile(0, 0, 0).to_feature_collection();

    println!("tile: {}", serde_json::to_string(&tile)?);

    Ok(())
}
