use geojson::{Feature, Geometry};
use supercluster::{CoordinateSystem, Supercluster, SuperclusterBuilder, SuperclusterError};

fn main() -> Result<(), SuperclusterError> {
    // Create a few points as GeoJSON features
    let features: Vec<Feature> = [[-77.032, 38.913], [-77.033, 38.913], [-77.034, 38.913]]
        .into_iter()
        .map(|point| Feature::from(Geometry::new_point(point)))
        .collect();

    // Set the configuration settings
    let options = SuperclusterBuilder::new()
        .radius(40.0)
        .extent(512.0)
        .min_points(2)
        .max_zoom(16)
        .coordinate_system(CoordinateSystem::LatLng)
        .build()?;

    // Create a new instance with the specified configuration settings
    let mut supercluster = Supercluster::new(options);

    // Load the features into the Supercluster instance
    supercluster.load_features(features)?;

    // Get a tile from the Supercluster instance
    let tile = supercluster.get_tile(0, 0, 0);

    println!("Tile: {:?}", tile.to_feature_collection());

    Ok(())
}
