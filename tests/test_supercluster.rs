#![cfg(feature = "geojson")]

mod common;

use common::{
    get_data_range, load_cartesian, load_places, load_tile_places, load_tile_places_with_min_5,
};
use geojson::{Feature, Geometry, JsonObject};
use supercluster::{
    ClusterId, CoordinateSystem, Item, Supercluster, SuperclusterError, SuperclusterOptions,
};

/// Create the options most tests use: a radius of 40, an extent of 512 and a maximum zoom of 16.
fn create_options() -> SuperclusterOptions {
    SuperclusterOptions::builder()
        .radius(40.0)
        .extent(512.0)
        .min_points(2)
        .max_zoom(16)
        .coordinate_system(CoordinateSystem::LatLng)
        .build()
        .unwrap()
}

/// Create the options of the tests with a low maximum zoom.
fn create_options_with_low_max_zoom() -> SuperclusterOptions {
    SuperclusterOptions::builder()
        .radius(60.0)
        .extent(256.0)
        .min_points(2)
        .max_zoom(4)
        .coordinate_system(CoordinateSystem::LatLng)
        .build()
        .unwrap()
}

/// Load the places with the options.
fn load_supercluster(options: SuperclusterOptions) -> Supercluster<Feature> {
    Supercluster::load_features(options, load_places()).unwrap()
}

/// Get the number of points a child holds.
fn get_point_count(item: &Item<'_, Feature>) -> usize {
    match item {
        Item::Cluster(cluster) => cluster.point_count,
        Item::Point { .. } => 1,
    }
}

#[test]
fn test_tile() {
    let places_tile = load_tile_places();
    let supercluster = load_supercluster(create_options());
    let tile = supercluster.tile(0, 0, 0).to_feature_collection();

    assert_eq!(tile.features.len(), places_tile.features.len());
    assert_eq!(tile.features, places_tile.features);
}

#[test]
fn test_tile_not_found() {
    let supercluster = load_supercluster(create_options());

    assert!(supercluster.tile(10, 10, 10).is_empty());
}

#[test]
fn test_tile_with_cluster_max_zoom() {
    let supercluster = load_supercluster(create_options());
    let mut clustered = 0;
    let mut unclustered = 0;

    for x in 0..4 {
        for y in 0..4 {
            let tile = supercluster.tile(2, x, y);

            clustered += tile.items.len();

            // Above the maximum zoom level to cluster on, all that is left are the points
            let tile = supercluster.with_cluster_max_zoom(1).tile(2, x, y);

            assert!(tile
                .items
                .iter()
                .all(|tile_item| matches!(tile_item.item, Item::Point { .. })));

            unclustered += tile.items.len();
        }
    }

    assert!(clustered > 0);
    assert!(unclustered > clustered);
}

#[test]
fn test_generate_clusters_with_min_points() {
    let places_tile = load_tile_places_with_min_5();

    let options = SuperclusterOptions::builder()
        .radius(40.0)
        .extent(512.0)
        .min_points(5)
        .max_zoom(16)
        .coordinate_system(CoordinateSystem::LatLng)
        .build()
        .unwrap();
    let supercluster = load_supercluster(options);
    let tile = supercluster.tile(0, 0, 0).to_feature_collection();

    assert_eq!(tile.features.len(), places_tile.features.len());
    assert_eq!(tile.features, places_tile.features);
}

#[test]
fn test_children() {
    let supercluster = load_supercluster(create_options());

    let point_counts: Vec<usize> = supercluster
        .children(ClusterId::new(164))
        .unwrap()
        .iter()
        .map(get_point_count)
        .collect();

    assert_eq!(point_counts, vec![6, 7, 2, 1]);
}

#[test]
fn test_children_of_unknown_cluster() {
    let supercluster = load_supercluster(create_options());

    assert_eq!(
        supercluster.children(ClusterId::new(100000)).err(),
        Some(SuperclusterError::ClusterNotFound)
    );
}

#[test]
fn test_cluster_expansion_zoom() {
    let supercluster = load_supercluster(create_options());

    assert_eq!(
        supercluster.cluster_expansion_zoom(ClusterId::new(164)),
        Ok(1)
    );
    assert_eq!(
        supercluster.cluster_expansion_zoom(ClusterId::new(196)),
        Ok(1)
    );
    assert_eq!(
        supercluster.cluster_expansion_zoom(ClusterId::new(581)),
        Ok(2)
    );
    assert_eq!(
        supercluster.cluster_expansion_zoom(ClusterId::new(1157)),
        Ok(2)
    );
    assert_eq!(
        supercluster.cluster_expansion_zoom(ClusterId::new(4134)),
        Ok(3)
    );
}

#[test]
fn test_cluster_expansion_zoom_for_max_zoom() {
    let supercluster = load_supercluster(create_options_with_low_max_zoom());

    assert_eq!(
        supercluster.cluster_expansion_zoom(ClusterId::new(2504)),
        Ok(5)
    );
}

#[test]
fn test_cluster_expansion_zoom_filtered_accepting_all() {
    let supercluster = load_supercluster(create_options());

    for cluster_id in [164, 196, 581, 1157, 4134].map(ClusterId::new) {
        assert_eq!(
            supercluster.cluster_expansion_zoom_filtered(cluster_id, |_| true),
            supercluster.cluster_expansion_zoom(cluster_id)
        );
    }
}

#[test]
fn test_cluster_expansion_zoom_with_cluster_max_zoom() {
    let supercluster = load_supercluster(create_options_with_low_max_zoom());

    // Below the maximum zoom of 4, the given zoom level caps where the cluster expands.
    assert_eq!(
        supercluster
            .with_cluster_max_zoom(2)
            .cluster_expansion_zoom(ClusterId::new(2504)),
        Ok(3)
    );

    // Above the maximum zoom, the given zoom level is ignored.
    assert_eq!(
        supercluster
            .with_cluster_max_zoom(8)
            .cluster_expansion_zoom(ClusterId::new(2504)),
        Ok(5)
    );
}

#[test]
fn test_cluster_expansion_zoom_filtered() {
    let supercluster = load_supercluster(create_options());

    // Only the points with a name that starts with a letter after N are left, so some of the
    // children of the cluster are left out, and it expands on or after the zoom level it did.
    let filter = |place: &Feature| {
        place
            .property("name")
            .and_then(|name| name.as_str())
            .is_some_and(|name| name > "N")
    };

    for cluster_id in [164, 196, 581, 1157, 4134].map(ClusterId::new) {
        let expansion_zoom = supercluster.cluster_expansion_zoom(cluster_id).unwrap();
        let filtered_expansion_zoom = supercluster
            .cluster_expansion_zoom_filtered(cluster_id, filter)
            .unwrap();

        assert!(filtered_expansion_zoom >= expansion_zoom);
        assert!(filtered_expansion_zoom <= 17);
    }
}

#[test]
fn test_leaves() {
    let expected_names = vec![
        "Niagara Falls",
        "Cape San Blas",
        "Cape Sable",
        "Cape Canaveral",
        "San  Salvador",
        "Cabo Gracias a Dios",
        "I. de Cozumel",
        "Grand Cayman",
        "Miquelon",
        "Cape Bauld",
    ];

    let supercluster = load_supercluster(create_options());

    let leaf_names: Vec<String> = supercluster
        .leaves(ClusterId::new(164))
        .unwrap()
        .skip(5)
        .take(10)
        .map(|leaf| leaf.property("name").unwrap().as_str().unwrap().to_string())
        .collect();

    assert_eq!(leaf_names.len(), expected_names.len());
    assert_eq!(leaf_names, expected_names);
}

#[test]
fn test_leaves_of_unknown_cluster() {
    let supercluster = load_supercluster(create_options());

    assert_eq!(
        supercluster.leaves(ClusterId::new(100000)).err(),
        Some(SuperclusterError::ClusterNotFound)
    );
}

#[test]
fn test_leaves_matches_point_count_and_children() {
    let supercluster = load_supercluster(create_options());

    let mut clusters = 0;

    // Every zoom level that clusters, so every cluster is checked
    for zoom in 0..=16 {
        for item in supercluster.clusters([-180.0, -90.0, 180.0, 90.0], zoom) {
            if let Item::Cluster(cluster) = item {
                let leaves: Vec<&Feature> = supercluster.leaves(cluster.id).unwrap().collect();

                assert_eq!(leaves.len(), cluster.point_count);

                // The leaves are the points of the children, in the order of the children.
                // Loading indexes the leaves from the children it records, and `children` searches
                // the KD-tree, so this also checks that both find the children in the same order.
                let children_leaves: Vec<&Feature> = supercluster
                    .children(cluster.id)
                    .unwrap()
                    .into_iter()
                    .flat_map(|child| match child {
                        Item::Cluster(child) => supercluster.leaves(child.id).unwrap().collect(),
                        Item::Point { point, .. } => vec![point],
                    })
                    .collect();

                assert_eq!(leaves, children_leaves);

                clusters += 1;
            }
        }
    }

    assert!(clusters > 0);
}

#[test]
fn test_clusters_when_query_crosses_international_dateline() {
    let supercluster = Supercluster::load_features(
        create_options(),
        [-178.989, -178.99, -178.991, -178.992]
            .iter()
            .map(|lng| Feature {
                id: None,
                bbox: None,
                foreign_members: None,
                geometry: Some(Geometry::new_point([*lng, 0.0])),
                properties: Some(JsonObject::new()),
            })
            .collect(),
    )
    .unwrap();

    let non_crossing = supercluster.clusters([-179.0, -10.0, -177.0, 10.0], 1);
    let crossing = supercluster.clusters([179.0, -10.0, -177.0, 10.0], 1);

    assert!(!crossing.is_empty());
    assert!(!non_crossing.is_empty());
    assert_eq!(non_crossing.len(), crossing.len());
}

#[test]
fn test_does_not_crash_on_weird_bbox_values() {
    let supercluster = load_supercluster(create_options());

    assert_eq!(
        supercluster
            .clusters([129.42639, -103.720017, -445.930843, 114.518236], 1)
            .len(),
        26
    );
    assert_eq!(
        supercluster
            .clusters([112.207836, -84.578666, -463.149397, 120.169159], 1)
            .len(),
        27
    );
    assert_eq!(
        supercluster
            .clusters([129.886277, -82.33268, -445.470956, 120.39093], 1)
            .len(),
        26
    );
    assert_eq!(
        supercluster
            .clusters([458.220043, -84.239039, -117.13719, 120.206585], 1)
            .len(),
        25
    );
    assert_eq!(
        supercluster
            .clusters([456.713058, -80.354196, -118.644175, 120.539148], 1)
            .len(),
        25
    );
    assert_eq!(
        supercluster
            .clusters([453.105328, -75.857422, -122.251904, 120.73276], 1)
            .len(),
        25
    );
    assert_eq!(
        supercluster.clusters([-180.0, -90.0, 180.0, 90.0], 1).len(),
        61
    );
}

#[test]
fn test_cartesian_coordinates() {
    let data = load_cartesian();
    let range = get_data_range(&data).unwrap();

    let options = SuperclusterOptions::builder()
        .radius(20.0)
        .extent(512.0)
        .min_points(2)
        .max_zoom(16)
        .coordinate_system(CoordinateSystem::Cartesian { range })
        .build()
        .unwrap();
    let supercluster = Supercluster::load_features(options, data).unwrap();

    let clusters = supercluster.clusters([0.0, 0.0, 1000.0, 1000.0], 0);
    let point_counts: Vec<usize> = clusters.iter().map(get_point_count).collect();

    assert_eq!(point_counts, vec![3, 1, 1, 3]);

    let first = clusters[0].to_feature();

    assert_eq!(first.property("point_count").unwrap(), 3);
    assert_eq!(clusters[1].to_feature().property("point_count"), None);
}
