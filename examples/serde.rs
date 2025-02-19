use supercluster::{FeatureBuilder, Supercluster, SuperclusterError};

fn main() -> Result<(), SuperclusterError> {
    // Create a list of features
    let features = FeatureBuilder::new()
        .add_point(vec![-77.032, 38.913])
        .add_point(vec![-77.033, 38.913])
        .add_point(vec![-77.034, 38.913])
        .build();

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
    )
    .unwrap();

    // Create a new instance with the specified configuration settings
    let mut cluster = Supercluster::new(options);

    // Load features into the Supercluster instance
    let index = cluster.load(features)?;

    // Get a tile from the Supercluster instance
    let tile = index.get_tile(0, 0.0, 0.0)?;

    let json_string = serde_json::to_string(&tile).unwrap();
    println!("tile: {}", json_string);

    Ok(())
}
