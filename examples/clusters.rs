use geojson::{Feature, Geometry};
use supercluster::{CoordinateSystem, Item, Supercluster, SuperclusterError, SuperclusterOptions};

fn main() -> Result<(), SuperclusterError> {
    // Create a few points as GeoJSON features
    let features: Vec<Feature> = [[102.0, 0.5], [103.0, 1.0], [104.0, 0.0]]
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

    // Get clusters for a given bounding box and zoom level
    let clusters = supercluster.clusters([101.0, 0.0, 105.0, 2.0], 2);

    for item in &clusters {
        match item {
            Item::Cluster(cluster) => println!(
                "Cluster {} with {} points at {}, {}",
                cluster.id.get(),
                cluster.point_count,
                cluster.x,
                cluster.y
            ),
            Item::Point { index, .. } => println!("Point {}", index),
        }
    }

    // Convert them into GeoJSON features
    let features: Vec<_> = clusters.iter().map(Item::to_feature).collect();

    println!("Clusters: {:?}", features);

    Ok(())
}
