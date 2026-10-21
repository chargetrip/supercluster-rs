use geojson::{Feature, Geometry};
use supercluster::{CoordinateSystem, Item, Supercluster, SuperclusterBuilder, SuperclusterError};

fn main() -> Result<(), SuperclusterError> {
    // Create a few points as GeoJSON features
    let features: Vec<Feature> = [[102.0, 0.5], [103.0, 1.0], [104.0, 0.0]]
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

    // Get clusters for a given bounding box and zoom level
    let clusters = supercluster.get_clusters([101.0, 0.0, 105.0, 2.0], 2);

    for item in &clusters {
        match item {
            Item::Cluster(cluster) => println!(
                "Cluster {} with {} points at {}, {}",
                cluster.id, cluster.point_count, cluster.x, cluster.y
            ),
            Item::Point { index, .. } => println!("Point {}", index),
        }
    }

    // Convert them into GeoJSON features
    let features: Vec<_> = clusters.iter().map(Item::to_feature).collect();

    println!("Clusters: {:?}", features);

    Ok(())
}
