mod common;

use common::{
    get_options, load_non_geospatial, load_places, load_tile_places, load_tile_places_with_min_5,
};
use geojson::{Feature, Geometry, JsonObject, Value::Point};
use serde_json::json;
use supercluster::Supercluster;

#[test]
fn test_generate_clusters() {
    let places_tile = load_tile_places();

    let mut cluster = Supercluster::new(get_options(40.0, 512.0, 2, 16));
    let index = cluster.load(load_places());

    let tile = index.get_tile(0, 0.0, 0.0).expect("cannot get a tile");

    assert_eq!(tile.features.len(), places_tile.features.len());
    assert_eq!(tile.features, places_tile.features);
}

#[test]
fn test_generate_clusters_with_min_points() {
    let places_tile = load_tile_places_with_min_5();

    let mut cluster = Supercluster::new(get_options(40.0, 512.0, 5, 16));
    let index = cluster.load(load_places());

    let tile = index.get_tile(0, 0.0, 0.0).expect("cannot get a tile");

    assert_eq!(tile.features.len(), places_tile.features.len());
    assert_eq!(tile.features, places_tile.features);
}

#[test]
fn test_get_cluster() {
    let mut cluster = Supercluster::new(get_options(40.0, 512.0, 2, 16));
    let index = cluster.load(load_places());

    let cluster_counts: Vec<usize> = index
        .get_children(164)
        .unwrap()
        .iter()
        .map(|cluster| {
            cluster
                .property("point_count")
                .unwrap_or(&json!(1))
                .as_u64()
                .unwrap() as usize
        })
        .collect();

    // Define the expected cluster counts.
    let expected_counts: Vec<usize> = vec![6, 7, 2, 1];

    // Assert that the child counts match the expected counts.
    assert_eq!(cluster_counts, expected_counts);
}

#[test]
fn test_cluster_expansion_zoom() {
    let mut cluster = Supercluster::new(get_options(40.0, 512.0, 2, 16));
    let index = cluster.load(load_places());

    assert_eq!(index.get_cluster_expansion_zoom(164), 1);
    assert_eq!(index.get_cluster_expansion_zoom(196), 1);
    assert_eq!(index.get_cluster_expansion_zoom(581), 2);
    assert_eq!(index.get_cluster_expansion_zoom(1157), 2);
    assert_eq!(index.get_cluster_expansion_zoom(4134), 3);
}

#[test]
fn test_cluster_expansion_zoom_for_max_zoom() {
    let mut cluster = Supercluster::new(get_options(60.0, 256.0, 2, 4));
    let index = cluster.load(load_places());

    assert_eq!(index.get_cluster_expansion_zoom(2504), 5);
}

#[test]
fn test_get_cluster_leaves() {
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

    let mut cluster = Supercluster::new(get_options(40.0, 512.0, 2, 16));
    let index = cluster.load(load_places());

    let leaf_names: Vec<String> = index
        .get_leaves(164, 10, 5)
        .iter()
        .map(|leaf| leaf.property("name").unwrap().as_str().unwrap().to_string())
        .collect();

    assert_eq!(leaf_names.len(), expected_names.len());
    assert_eq!(leaf_names, expected_names);
}

#[test]
fn test_clusters_when_query_crosses_international_dateline() {
    let mut cluster = Supercluster::new(get_options(40.0, 512.0, 2, 16));
    let index = cluster.load(vec![
        Feature {
            id: None,
            bbox: None,
            foreign_members: None,
            geometry: Some(Geometry::new(Point(vec![-178.989, 0.0]))),
            properties: Some(JsonObject::new()),
        },
        Feature {
            id: None,
            bbox: None,
            foreign_members: None,
            geometry: Some(Geometry::new(Point(vec![-178.99, 0.0]))),
            properties: Some(JsonObject::new()),
        },
        Feature {
            id: None,
            bbox: None,
            foreign_members: None,
            geometry: Some(Geometry::new(Point(vec![-178.991, 0.0]))),
            properties: Some(JsonObject::new()),
        },
        Feature {
            id: None,
            bbox: None,
            foreign_members: None,
            geometry: Some(Geometry::new(Point(vec![-178.992, 0.0]))),
            properties: Some(JsonObject::new()),
        },
    ]);

    let non_crossing = index.get_clusters([-179.0, -10.0, -177.0, 10.0], 1);
    let crossing = index.get_clusters([179.0, -10.0, -177.0, 10.0], 1);

    assert!(!crossing.is_empty());
    assert!(!non_crossing.is_empty());
    assert_eq!(non_crossing.len(), crossing.len());
}

#[test]
fn test_does_not_crash_on_weird_bbox_values() {
    let mut cluster = Supercluster::new(get_options(40.0, 512.0, 2, 16));
    let index = cluster.load(load_places());

    assert_eq!(
        index
            .get_clusters([129.42639, -103.720017, -445.930843, 114.518236], 1)
            .len(),
        26
    );
    assert_eq!(
        index
            .get_clusters([112.207836, -84.578666, -463.149397, 120.169159], 1)
            .len(),
        27
    );
    assert_eq!(
        index
            .get_clusters([129.886277, -82.33268, -445.470956, 120.39093], 1)
            .len(),
        26
    );
    assert_eq!(
        index
            .get_clusters([458.220043, -84.239039, -117.13719, 120.206585], 1)
            .len(),
        25
    );
    assert_eq!(
        index
            .get_clusters([456.713058, -80.354196, -118.644175, 120.539148], 1)
            .len(),
        25
    );
    assert_eq!(
        index
            .get_clusters([453.105328, -75.857422, -122.251904, 120.73276], 1)
            .len(),
        25
    );
    assert_eq!(
        index.get_clusters([-180.0, -90.0, 180.0, 90.0], 1).len(),
        61
    );
}

#[test]
fn test_non_geospatial() {
    let mut cluster = Supercluster::new_non_geospatial(get_options(500.0, 32.0, 2, 16));
    let index = cluster.load(load_non_geospatial());

    let clusters = index.get_clusters([0.0, 0.0, 1000.0, 1000.0], 0);

    assert_eq!(clusters.len(), 4);
    assert_eq!(clusters[0].property("point_count").unwrap(), 3);
    assert_eq!(clusters[1].property("point_count"), None);
    assert_eq!(clusters[2].property("point_count"), None);
    assert_eq!(clusters[3].property("point_count").unwrap(), 3);
}
