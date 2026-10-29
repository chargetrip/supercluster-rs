#![cfg(feature = "geojson")]

mod common;

use common::{
    get_data_range, load_cartesian, load_places, load_tile_places, load_tile_places_with_min_5,
};
use geojson::{feature::Id, Feature, Geometry, JsonObject};
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

/// The bounding box of the whole world.
const WORLD: [f64; 4] = [-180.0, -90.0, 180.0, 90.0];

/// Load the places with the options.
fn load_supercluster(options: SuperclusterOptions) -> Supercluster<Feature> {
    Supercluster::load_features(options, load_places()).unwrap()
}

/// Get the number of points a child holds.
fn get_point_count(item: &Item<'_, Feature>) -> usize {
    match item {
        Item::Cluster(cluster) => cluster.point_count as usize,
        Item::Point { .. } => 1,
    }
}

/// Get the ID of the cluster at `index` in the clusters of the whole world at `zoom`. The tests
/// find a cluster where it is, because the fixtures come from the JavaScript supercluster, which
/// numbers its clusters another way.
fn get_cluster_id(supercluster: &Supercluster<Feature>, zoom: u8, index: usize) -> ClusterId {
    match supercluster.clusters(WORLD, zoom)[index] {
        Item::Cluster(cluster) => cluster.id,
        Item::Point { .. } => panic!("expected a cluster at {index}"),
    }
}

/// The clusters the tests check: their zoom level and index in the clusters of the whole world.
/// The JavaScript supercluster has the IDs 164, 196, 581, 1157 and 4134 for them.
const CLUSTERS: [(u8, usize); 5] = [(0, 0), (0, 1), (0, 8), (0, 19), (0, 31)];

/// Remove the IDs of the clusters, which this crate numbers another way than the JavaScript
/// supercluster the fixtures come from. Everything else must match.
fn without_cluster_ids(features: Vec<Feature>) -> Vec<Feature> {
    features
        .into_iter()
        .map(|mut feature| {
            if feature.contains_property("cluster") {
                feature.id = None;
                feature.remove_property("cluster_id");
            }

            feature
        })
        .collect()
}

/// Check that the ID of every cluster feature finds the cluster, with as many points as the
/// feature says.
fn assert_cluster_ids_find_their_clusters(
    supercluster: &Supercluster<Feature>,
    features: &[Feature],
) {
    for feature in features {
        let Some(id) = feature.property("cluster_id") else {
            continue;
        };
        let id = ClusterId::new(id.as_u64().unwrap() as u32);
        let point_count = feature.property("point_count").unwrap().as_u64().unwrap();

        assert_eq!(feature.id, Some(Id::String(id.get().to_string())));
        assert_eq!(supercluster.leaves(id).unwrap().len() as u64, point_count);
    }
}

#[test]
fn test_tile() {
    let places_tile = load_tile_places();
    let supercluster = load_supercluster(create_options());
    let tile = supercluster.tile(0, 0, 0).to_feature_collection();

    assert_eq!(tile.features.len(), places_tile.features.len());
    assert_cluster_ids_find_their_clusters(&supercluster, &tile.features);
    assert_eq!(
        without_cluster_ids(tile.features),
        without_cluster_ids(places_tile.features)
    );
}

#[test]
fn test_tile_as_feature_collection() {
    let supercluster = load_supercluster(create_options());

    // A tile of clusters, and one of points with all their properties
    for tile in [
        supercluster.tile(0, 0, 0),
        supercluster.with_cluster_max_zoom(0).tile(1, 0, 0),
    ] {
        assert_eq!(
            serde_json::to_string(&tile.as_feature_collection()).unwrap(),
            serde_json::to_string(&tile.to_feature_collection()).unwrap()
        );
    }
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
    assert_cluster_ids_find_their_clusters(&supercluster, &tile.features);
    assert_eq!(
        without_cluster_ids(tile.features),
        without_cluster_ids(places_tile.features)
    );
}

#[test]
fn test_children() {
    let supercluster = load_supercluster(create_options());

    let point_counts: Vec<usize> = supercluster
        .children(get_cluster_id(&supercluster, 0, 0))
        .unwrap()
        .map(|item| get_point_count(&item))
        .collect();

    assert_eq!(point_counts, vec![6, 7, 2, 1]);
}

#[test]
fn test_children_of_unknown_cluster() {
    let supercluster = load_supercluster(create_options());

    assert!(matches!(
        supercluster.children(ClusterId::new(100000)),
        Err(SuperclusterError::ClusterNotFound)
    ));
}

#[test]
fn test_cluster_expansion_zoom() {
    let supercluster = load_supercluster(create_options());

    let expansion_zooms: Vec<u8> = CLUSTERS
        .iter()
        .map(|&(zoom, index)| {
            supercluster
                .cluster_expansion_zoom(get_cluster_id(&supercluster, zoom, index))
                .unwrap()
        })
        .collect();

    assert_eq!(expansion_zooms, vec![1, 1, 2, 2, 3]);
}

#[test]
fn test_cluster_expansion_zoom_for_max_zoom() {
    let supercluster = load_supercluster(create_options_with_low_max_zoom());

    // The JavaScript supercluster has the ID 2504 for this cluster
    let cluster_id = get_cluster_id(&supercluster, 4, 56);

    assert_eq!(supercluster.cluster_expansion_zoom(cluster_id), Ok(5));
}

#[test]
fn test_cluster_expansion_zoom_filtered_accepting_all() {
    let supercluster = load_supercluster(create_options());

    for (zoom, index) in CLUSTERS {
        let cluster_id = get_cluster_id(&supercluster, zoom, index);

        assert_eq!(
            supercluster.cluster_expansion_zoom_filtered(cluster_id, |_| true),
            supercluster.cluster_expansion_zoom(cluster_id)
        );
    }
}

#[test]
fn test_cluster_expansion_zoom_with_cluster_max_zoom() {
    let supercluster = load_supercluster(create_options_with_low_max_zoom());
    let cluster_id = get_cluster_id(&supercluster, 4, 56);

    // Below the maximum zoom of 4, the given zoom level caps where the cluster expands.
    assert_eq!(
        supercluster
            .with_cluster_max_zoom(2)
            .cluster_expansion_zoom(cluster_id),
        Ok(3)
    );

    // Above the maximum zoom, the given zoom level is ignored.
    assert_eq!(
        supercluster
            .with_cluster_max_zoom(8)
            .cluster_expansion_zoom(cluster_id),
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

    for (zoom, index) in CLUSTERS {
        let cluster_id = get_cluster_id(&supercluster, zoom, index);
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
        .leaves(get_cluster_id(&supercluster, 0, 0))
        .unwrap()
        .iter()
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
                let leaves: Vec<&Feature> =
                    supercluster.leaves(cluster.id).unwrap().iter().collect();

                assert_eq!(leaves.len(), cluster.point_count as usize);

                // The leaves are the points of the children, in the order of the children
                let children_leaves: Vec<&Feature> = supercluster
                    .children(cluster.id)
                    .unwrap()
                    .flat_map(|child| match child {
                        Item::Cluster(child) => {
                            supercluster.leaves(child.id).unwrap().iter().collect()
                        }
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
