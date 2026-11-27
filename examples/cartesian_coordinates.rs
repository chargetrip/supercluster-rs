use geojson::{Feature, Geometry};
use supercluster::{CoordinateSystem, DataRange, Options, Supercluster};

fn main() -> Result<(), supercluster::Error> {
    // Create a few points as GeoJSON features
    let features: Vec<Feature> = [[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]]
        .into_iter()
        .map(|point| Feature::from(Geometry::new_point(point)))
        .collect();

    // Set the configuration settings for Cartesian coordinates
    let options = Options::builder()
        .radius(40.0)
        .extent(512)
        .min_points(2)
        .max_zoom(16)
        .coordinate_system(CoordinateSystem::Cartesian {
            range: DataRange::new(0.0, 0.0, 2.0, 2.0),
        })
        .build()?;

    // Load the features into a new supercluster
    let supercluster = Supercluster::load_features(options, features)?;

    // Get a tile
    let tile = supercluster.tile(0, 0, 0);

    println!("Tile: {:?}", tile.to_feature_collection());

    Ok(())
}
