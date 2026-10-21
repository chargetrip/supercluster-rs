use geojson::{Feature, Geometry};
use supercluster::{CoordinateSystem, Supercluster, SuperclusterError, SuperclusterOptions};

fn main() -> Result<(), SuperclusterError> {
    // Create a few points as GeoJSON features
    let features: Vec<Feature> = [[-77.032, 38.913], [-77.033, 38.913], [-77.034, 38.913]]
        .into_iter()
        .map(|point| Feature::from(Geometry::new_point(point)))
        .collect();

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
    let tile = supercluster.tile(0, 0, 0);

    println!("Tile: {:?}", tile.to_feature_collection());

    Ok(())
}
