use geojson::{Feature, Geometry, JsonObject, JsonValue};
use supercluster::{CoordinateSystem, Supercluster, SuperclusterError, SuperclusterOptions};

/// Create a point feature with a name.
fn create_point(lng: f64, lat: f64, name: &str) -> Feature {
    let mut properties = JsonObject::new();

    properties.insert("name".to_string(), JsonValue::from(name));

    Feature {
        geometry: Some(Geometry::new_point([lng, lat])),
        properties: Some(properties),
        ..Default::default()
    }
}

fn main() -> Result<(), SuperclusterError> {
    // Create a few points as GeoJSON features
    let features = vec![
        create_point(102.0, 0.5, "Point 1"),
        create_point(103.0, 1.0, "Point 2"),
        create_point(104.0, 0.0, "Point 3"),
    ];

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

    // Get a tile
    let tile = supercluster.tile(0, 0, 0).to_feature_collection();

    println!("Tile: {:?}", tile);

    for feature in tile.features {
        assert!(feature.properties.is_some());
    }

    Ok(())
}
