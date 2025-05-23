use geojson::{Feature, FeatureCollection, Geometry, Value};
use serde_json::json;
use supercluster::{CoordinateSystem, Supercluster, SuperclusterError};

fn main() -> Result<(), SuperclusterError> {
    // Create a few points as GeoJSON features
    let features = vec![
        Feature {
            geometry: Some(Geometry::new(Value::Point(vec![102.0, 0.5]))),
            properties: Some(json!({"name": "Point 1"}).as_object().unwrap().clone()),
            ..Default::default()
        },
        Feature {
            geometry: Some(Geometry::new(Value::Point(vec![103.0, 1.0]))),
            properties: Some(json!({"name": "Point 2"}).as_object().unwrap().clone()),
            ..Default::default()
        },
        Feature {
            geometry: Some(Geometry::new(Value::Point(vec![104.0, 0.0]))),
            properties: Some(json!({"name": "Point 3"}).as_object().unwrap().clone()),
            ..Default::default()
        },
    ];

    // Create a FeatureCollection
    let feature_collection = FeatureCollection {
        features,
        bbox: None,
        foreign_members: None,
    };

    // Set the configuration settings
    let options = Supercluster::builder()
        .radius(40.0)
        .extent(512.0)
        .min_points(2)
        .max_zoom(16)
        .coordinate_system(CoordinateSystem::LatLng)
        .build();

    // Create a new instance with the specified configuration settings
    let mut cluster = Supercluster::new(options);

    // Load the FeatureCollection into the Supercluster instance
    let index = cluster.load(feature_collection.features)?;

    // Get a tile from the Supercluster instance
    let tile = index.get_tile(0, 0.0, 0.0)?;

    println!("Tile: {:?}", tile);

    for feature in tile.features {
        assert!(feature.properties.is_some());
    }

    Ok(())
}
