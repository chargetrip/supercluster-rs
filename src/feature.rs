//! # Feature module
//!
//! Contains the GeoJSON support of the supercluster crate, which the `geojson` feature enables.
//!
//! A list of GeoJSON features is loaded into a supercluster as it is, with `load_features`. The
//! clusters and tiles it returns convert back into GeoJSON features, or serialize as GeoJSON
//! without copying the features. The points can be the features themselves or anything that
//! borrows one, such as an `Arc<Feature>`.

use std::{
    borrow::Borrow,
    fmt::{self, Debug, Formatter},
};

use geojson::{
    feature::Id, Feature, FeatureCollection, Geometry, GeometryValue, JsonObject, JsonValue,
};
use serde::{ser::SerializeMap, Serialize, Serializer};

use crate::{supercluster::Cluster, Error, Item, Options, Supercluster, Tile, TileItem};

impl<P: Borrow<Feature>> Supercluster<P> {
    /// Load GeoJSON features into a new supercluster, like `load`. A feature can be shared, such as
    /// an `Arc<Feature>`, so the supercluster does not hold a copy of it. When Rust cannot tell the
    /// type of the list, for example from `serde_json::from_str` or `collect`, name it, as in
    /// `Vec<Feature>`.
    /// A feature with a point geometry is clustered by its longitude and latitude, or by its X and
    /// Y values with the `Cartesian` coordinate system. A third value, the altitude, is ignored.
    /// Any other feature, and a point whose coordinates are not finite numbers, is left out of the
    /// clustering.
    ///
    /// # Errors
    ///
    /// Returns `Error::TooManyPoints` if there are more than `MAX_POINTS` features.
    ///
    /// # Examples
    ///
    /// ```
    /// use geojson::{Feature, Geometry};
    /// use supercluster::{Supercluster, Options};
    ///
    /// let features: Vec<Feature> = [[-77.032, 38.913], [-77.033, 38.913]]
    ///     .into_iter()
    ///     .map(|point| Feature::from(Geometry::new_point(point)))
    ///     .collect();
    ///
    /// let supercluster = Supercluster::load_features(Options::default(), features)?;
    ///
    /// let tile = supercluster.tile(0, 0, 0).to_feature_collection();
    ///
    /// assert_eq!(tile.features.len(), 1);
    /// # Ok::<(), supercluster::Error>(())
    /// ```
    pub fn load_features(options: Options, features: Vec<P>) -> Result<Self, Error> {
        Self::load(options, features, |feature| {
            get_coordinates(feature.borrow())
        })
    }
}

/// Get the first two values of a feature with a point geometry, or `None` for any other feature.
fn get_coordinates(feature: &Feature) -> Option<[f64; 2]> {
    match &feature.geometry.as_ref()?.value {
        GeometryValue::Point { coordinates } if coordinates.len() >= 2 => {
            Some([coordinates[0], coordinates[1]])
        }
        _ => None,
    }
}

impl Cluster<'_> {
    /// Get the GeoJSON properties of the cluster, with their keys in alphabetical order:
    /// `cluster`, `cluster_id` and `point_count`, where `point_count` is all the points of the
    /// cluster, or the ones that pass for a filtered tile. Both
    /// `get_properties` and `ClusterPropertiesView` build on them, so they always agree.
    fn properties(&self, point_count: u32) -> [(&'static str, ClusterProperty); 3] {
        [
            ("cluster", ClusterProperty::Flag(true)),
            ("cluster_id", ClusterProperty::Number(self.id().get())),
            ("point_count", ClusterProperty::Number(point_count)),
        ]
    }

    /// Get the GeoJSON properties of the cluster as a JSON object, with `point_count` points.
    fn get_properties(&self, point_count: u32) -> JsonObject {
        let mut properties = JsonObject::new();

        // Inserted one by one, because collecting into the map first collects into a vector
        // and sorts it
        for (key, value) in self.properties(point_count) {
            properties.insert(key.to_string(), JsonValue::from(value));
        }

        properties
    }

    /// Convert the cluster into a GeoJSON feature, with its center as the geometry.
    #[must_use]
    pub fn to_feature(&self) -> Feature {
        self.feature_at(Geometry::new_point(self.center()), self.total_count())
    }

    /// Convert the cluster into a GeoJSON feature with the geometry, so a tile does not compute
    /// a center it replaces with the position in the tile, and with `point_count` points.
    fn feature_at(&self, geometry: Geometry, point_count: u32) -> Feature {
        Feature {
            id: Some(Id::Number(self.id().get().into())),
            bbox: None,
            geometry: Some(geometry),
            properties: Some(self.get_properties(point_count)),
            foreign_members: None,
        }
    }
}

impl<P: Borrow<Feature>> Item<'_, P> {
    /// Convert the item into a GeoJSON feature: a cluster at its center, with the properties
    /// `cluster`, `cluster_id` and `point_count`, or the point as it was loaded.
    #[must_use]
    pub fn to_feature(&self) -> Feature {
        match self {
            Item::Cluster(cluster) => cluster.to_feature(),
            Item::Point { point, .. } => (*point).borrow().clone(),
        }
    }
}

impl<P: Borrow<Feature>> TileItem<'_, P> {
    /// Convert the item into a GeoJSON feature, with its position in the tile as the geometry: the
    /// point with its ID and properties, or the cluster with `count` as its `point_count`, which
    /// in a filtered tile counts the points that pass.
    #[must_use]
    pub fn to_feature(&self) -> Feature {
        let geometry = Geometry::new_point([f64::from(self.x()), f64::from(self.y())]);

        match self.item() {
            Item::Cluster(cluster) => cluster.feature_at(geometry, self.count()),
            Item::Point { point, .. } => Feature {
                id: point.borrow().id.clone(),
                bbox: None,
                geometry: Some(geometry),
                properties: point.borrow().properties.clone(),
                foreign_members: None,
            },
        }
    }
}

impl<'a, P: Borrow<Feature>> Tile<'a, P> {
    /// Convert the tile into a GeoJSON feature collection, with the position of each item in the
    /// tile as its geometry. In a filtered tile, a cluster is at the center of the points that
    /// pass, and its `point_count` counts the points that pass. The features it returns are owned,
    /// so they can be changed.
    ///
    /// A `geojson` geometry holds floats, so the positions serialize as `150.0`. To serialize them
    /// as whole numbers, such as `150`, serialize `as_feature_collection` instead.
    #[must_use]
    pub fn to_feature_collection(&self) -> FeatureCollection {
        FeatureCollection {
            bbox: None,
            features: self.items.iter().map(|item| item.to_feature()).collect(),
            foreign_members: None,
        }
    }

    /// Get a view of the tile as a GeoJSON feature collection, which serializes to the same GeoJSON
    /// as `to_feature_collection` but borrows the properties of the features instead of copying
    /// them. It writes the positions in the tile as whole numbers, such as `150`, where a
    /// `geojson` geometry holds floats and writes `150.0`. Serialize it to write a tile as
    /// GeoJSON, for example with `serde_json::to_writer`.
    ///
    /// # Examples
    ///
    /// ```
    /// use geojson::{Feature, FeatureCollection, Geometry};
    /// use supercluster::{Supercluster, Options};
    ///
    /// let features = vec![Feature::from(Geometry::new_point([-77.032, 38.913]))];
    /// let supercluster = Supercluster::load_features(Options::default(), features)?;
    /// let tile = supercluster.tile(0, 0, 0);
    ///
    /// let json = serde_json::to_string(&tile.as_feature_collection())?;
    ///
    /// assert_eq!(serde_json::from_str::<FeatureCollection>(&json)?, tile.to_feature_collection());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn as_feature_collection(&self) -> FeatureCollectionView<'_, 'a, P> {
        FeatureCollectionView { items: &self.items }
    }
}

/// A view of a tile as a GeoJSON feature collection, which `Tile::as_feature_collection` returns.
/// It serializes to the same GeoJSON as the feature collection that `to_feature_collection`
/// returns, without copying the features, and writes the positions in the tile as whole numbers.
pub struct FeatureCollectionView<'t, 'a, P> {
    /// The items of the tile to serialize.
    items: &'t [TileItem<'a, P>],
}

// Written by hand, because a derive would also require the point type to be `Clone` and `Copy`,
// and a view only holds a reference to the items.
impl<P> Clone for FeatureCollectionView<'_, '_, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Copy for FeatureCollectionView<'_, '_, P> {}

impl<P: Debug> Debug for FeatureCollectionView<'_, '_, P> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FeatureCollectionView")
            .field("items", &self.items)
            .finish()
    }
}

impl<P: Borrow<Feature>> Serialize for FeatureCollectionView<'_, '_, P> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;

        map.serialize_entry("type", "FeatureCollection")?;
        map.serialize_entry("features", &FeatureViews(self.items))?;
        map.end()
    }
}

/// The items of a tile, which serialize as a list of GeoJSON features.
struct FeatureViews<'t, 'a, P>(&'t [TileItem<'a, P>]);

impl<P: Borrow<Feature>> Serialize for FeatureViews<'_, '_, P> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(
            self.0
                .iter()
                .map(|item| FeatureView(item.x(), item.y(), item.item(), item.count())),
        )
    }
}

/// An item of a tile at its position in the tile, which serializes like `TileItem::to_feature`
/// returns it: the fields in the order of `geojson`, with the position in the tile as the geometry.
/// A cluster has the count of the item, which in a filtered tile counts the points that pass.
struct FeatureView<'a, P>(i32, i32, Item<'a, P>, u32);

impl<P: Borrow<Feature>> Serialize for FeatureView<'_, P> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let FeatureView(x, y, item, count) = self;

        // The type, the geometry, the properties, and the ID, which a point may not have
        let length = match item {
            Item::Cluster(_) => 4,
            Item::Point { point, .. } => 3 + usize::from((*point).borrow().id.is_some()),
        };
        let mut map = serializer.serialize_map(Some(length))?;

        map.serialize_entry("type", "Feature")?;
        map.serialize_entry("geometry", &PointView([*x, *y]))?;

        match item {
            Item::Cluster(cluster) => {
                map.serialize_entry("id", &cluster.id().get())?;
                map.serialize_entry("properties", &ClusterPropertiesView(cluster, *count))?;
            }
            Item::Point { point, .. } => {
                let point = (*point).borrow();

                if let Some(id) = &point.id {
                    map.serialize_entry("id", id)?;
                }

                map.serialize_entry("properties", &point.properties)?;
            }
        }

        map.end()
    }
}

/// A point geometry at a position in a tile, which serializes like `Geometry::new_point`, with
/// whole numbers.
struct PointView([i32; 2]);

impl Serialize for PointView {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;

        map.serialize_entry("type", "Point")?;
        map.serialize_entry("coordinates", &self.0)?;
        map.end()
    }
}

/// The GeoJSON properties of a cluster with a number of points, which serialize like
/// `Cluster::get_properties`.
struct ClusterPropertiesView<'c, 'a>(&'c Cluster<'a>, u32);

impl Serialize for ClusterPropertiesView<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let properties = self.0.properties(self.1);
        let mut map = serializer.serialize_map(Some(properties.len()))?;

        for (key, value) in &properties {
            map.serialize_entry(key, value)?;
        }

        map.end()
    }
}

/// A value of the GeoJSON properties of a cluster.
enum ClusterProperty {
    /// A property that is `true` or `false`.
    Flag(bool),

    /// A property that is a whole number.
    Number(u32),
}

impl From<ClusterProperty> for JsonValue {
    fn from(property: ClusterProperty) -> Self {
        match property {
            ClusterProperty::Flag(flag) => JsonValue::from(flag),
            ClusterProperty::Number(number) => JsonValue::from(number),
        }
    }
}

impl Serialize for ClusterProperty {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ClusterProperty::Flag(flag) => serializer.serialize_bool(*flag),
            ClusterProperty::Number(number) => serializer.serialize_u32(*number),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    /// Create a point feature with a name.
    fn create_feature(id: &str, lng: f64, lat: f64) -> Feature {
        let mut properties = JsonObject::new();

        properties.insert("name".to_string(), JsonValue::from(id));

        Feature {
            id: Some(Id::String(id.to_string())),
            bbox: None,
            geometry: Some(Geometry::new_point([lng, lat])),
            properties: Some(properties),
            foreign_members: None,
        }
    }

    /// Create a supercluster with two close features, which form a cluster on the first zoom levels.
    fn load_features() -> Supercluster<Feature> {
        Supercluster::load_features(
            Options::default(),
            vec![
                create_feature("a", 10.0, 50.0),
                create_feature("b", 10.0001, 50.0),
            ],
        )
        .unwrap()
    }

    /// Load the features and get the indexes of the ones that are not clustered at the highest zoom
    /// level, which are the features that were not left out.
    fn load_and_get_indexes(features: Vec<Feature>) -> Vec<usize> {
        let supercluster = Supercluster::load_features(Options::default(), features).unwrap();

        let mut indexes: Vec<usize> = supercluster
            .clusters([-180.0, -90.0, 180.0, 90.0], 17)
            .iter()
            .filter_map(|item| match item {
                Item::Point { index, .. } => Some(*index),
                Item::Cluster(_) => None,
            })
            .collect();

        indexes.sort();

        indexes
    }

    #[test]
    fn test_load_features() {
        let indexes = load_and_get_indexes(vec![
            create_feature("a", 10.0, 50.0),
            create_feature("b", 12.0, 50.0),
        ]);

        assert_eq!(indexes, vec![0, 1]);
    }

    #[test]
    fn test_load_features_of_shared_features() {
        let features = vec![
            create_feature("a", 10.0, 50.0),
            create_feature("b", 10.0001, 50.0),
            create_feature("c", 12.0, 50.0),
        ];
        let owned = Supercluster::load_features(Options::default(), features.clone()).unwrap();
        let shared = Supercluster::load_features(
            Options::default(),
            features.into_iter().map(Arc::new).collect(),
        )
        .unwrap();

        // Shared features convert and serialize like the features themselves, as clusters and as
        // points
        for (z, x, y) in [(0, 0, 0), (17, 69_176, 44_452)] {
            let (owned_tile, shared_tile) = (owned.tile(z, x, y), shared.tile(z, x, y));

            assert!(!owned_tile.is_empty());
            assert_eq!(
                shared_tile.to_feature_collection(),
                owned_tile.to_feature_collection()
            );
            assert_eq!(
                serde_json::to_string(&shared_tile.as_feature_collection()).unwrap(),
                serde_json::to_string(&owned_tile.as_feature_collection()).unwrap()
            );
        }

        for zoom in [0, 17] {
            let world = [-180.0, -90.0, 180.0, 90.0];
            let owned_features: Vec<Feature> = owned
                .clusters(world, zoom)
                .iter()
                .map(Item::to_feature)
                .collect();
            let shared_features: Vec<Feature> = shared
                .clusters(world, zoom)
                .iter()
                .map(Item::to_feature)
                .collect();

            assert_eq!(shared_features, owned_features);
        }
    }

    #[test]
    fn test_load_features_with_altitude() {
        let mut feature = create_feature("a", 10.0, 50.0);

        feature.geometry = Some(Geometry::new_point([10.0, 50.0, 3.0]));

        assert_eq!(load_and_get_indexes(vec![feature]), vec![0]);
    }

    #[test]
    fn test_load_features_without_geometry() {
        let mut feature = create_feature("a", 10.0, 50.0);

        feature.geometry = None;

        assert_eq!(load_and_get_indexes(vec![feature]), Vec::<usize>::new());
    }

    #[test]
    fn test_load_features_with_other_geometry() {
        let mut feature = create_feature("a", 10.0, 50.0);

        feature.geometry = Some(Geometry::new_multi_point([[10.0, 50.0]]));

        assert_eq!(load_and_get_indexes(vec![feature]), Vec::<usize>::new());
    }

    #[test]
    fn test_load_features_with_short_position() {
        let mut feature = create_feature("a", 10.0, 50.0);

        feature.geometry = Some(Geometry::new_point(vec![10.0]));

        assert_eq!(load_and_get_indexes(vec![feature]), Vec::<usize>::new());
    }

    #[test]
    fn test_load_features_keeps_the_index_of_features_that_are_left_out() {
        let indexes = load_and_get_indexes(vec![
            create_feature("a", 10.0, 50.0),
            Feature::default(),
            create_feature("b", 12.0, 50.0),
        ]);

        assert_eq!(indexes, vec![0, 2]);
    }

    #[test]
    fn test_cluster_to_feature() {
        // 1500 points close to each other form one cluster
        let features = (0..1500)
            .map(|i| create_feature("a", 10.0 + f64::from(i) * 1e-7, 50.0))
            .collect();
        let supercluster = Supercluster::load_features(Options::default(), features).unwrap();
        let items = supercluster.clusters([-180.0, -90.0, 180.0, 90.0], 0);
        let clusters: Vec<_> = items
            .iter()
            .filter_map(|item| match item {
                Item::Cluster(cluster) => Some(cluster),
                Item::Point { .. } => None,
            })
            .collect();

        assert_eq!((items.len(), clusters.len()), (1, 1));

        let cluster = clusters[0];
        let feature = cluster.to_feature();
        let id = cluster.id().get();

        assert_eq!(feature.id, Some(Id::Number(id.into())));
        assert_eq!(
            feature.geometry,
            Some(Geometry::new_point(cluster.center()))
        );
        assert_eq!(feature.property("cluster"), Some(&JsonValue::from(true)));
        assert_eq!(feature.property("cluster_id"), Some(&JsonValue::from(id)));
        assert_eq!(
            feature.property("point_count"),
            Some(&JsonValue::from(1500))
        );
        assert_eq!(feature.property("point_count_abbreviated"), None);
    }

    #[test]
    fn test_item_to_feature() {
        let supercluster = load_features();
        let items = supercluster.clusters([-180.0, -90.0, 180.0, 90.0], 0);

        assert_eq!(items.len(), 1);

        let feature = items[0].to_feature();

        assert!(feature.contains_property("cluster"));
        assert_eq!(feature.property("point_count"), Some(&JsonValue::from(2)));

        let center = get_coordinates(&feature).unwrap();

        assert!((center[0] - 10.00005).abs() < 1e-9);
        assert!((center[1] - 50.0).abs() < 1e-9);

        let items = supercluster.clusters([-180.0, -90.0, 180.0, 90.0], 17);

        assert_eq!(items.len(), 2);
        assert!(items
            .iter()
            .all(|item| !item.to_feature().contains_property("cluster")));
    }

    #[test]
    fn test_tile_to_feature_collection() {
        let supercluster = load_features();
        let collection = supercluster.tile(0, 0, 0).to_feature_collection();

        assert_eq!(collection.features.len(), 1);

        let feature = &collection.features[0];

        assert!(feature.contains_property("cluster"));
        assert_eq!(feature.property("point_count"), Some(&JsonValue::from(2)));

        // The geometry holds the position in the tile.
        let [x, y] = get_coordinates(feature).unwrap();

        assert!((0.0..=512.0).contains(&x));
        assert!((0.0..=512.0).contains(&y));
    }

    #[test]
    fn test_tile_to_feature_collection_with_points() {
        let supercluster = load_features();
        let collection = supercluster
            .with_cluster_max_zoom(0)
            .tile(1, 1, 0)
            .to_feature_collection();

        assert_eq!(collection.features.len(), 2);

        let mut names: Vec<&JsonValue> = collection
            .features
            .iter()
            .map(|feature| feature.property("name").unwrap())
            .collect();

        names.sort_by_key(|name| name.to_string());

        assert_eq!(names, vec![&JsonValue::from("a"), &JsonValue::from("b")]);
        assert!(collection
            .features
            .iter()
            .all(|feature| !feature.contains_property("cluster")));
    }

    #[test]
    fn test_point_item_to_feature() {
        let supercluster = load_features();
        let items = supercluster.clusters([-180.0, -90.0, 180.0, 90.0], 17);
        let item = items
            .iter()
            .find(|item| matches!(item, Item::Point { index: 0, .. }))
            .unwrap();

        assert_eq!(item.to_feature(), create_feature("a", 10.0, 50.0));
    }

    /// Serialize a value as JSON.
    fn to_json(value: &impl Serialize) -> String {
        serde_json::to_string(value).unwrap()
    }

    /// Serialize a view of a tile and read it back as a feature collection.
    fn read_back(view: FeatureCollectionView<'_, '_, Feature>) -> FeatureCollection {
        serde_json::from_str(&to_json(&view)).unwrap()
    }

    #[test]
    fn test_as_feature_collection_serializes_like_to_feature_collection() {
        let mut features: Vec<Feature> = (0..1500)
            .map(|i| create_feature("a", 10.0 + f64::from(i) * 1e-7, 50.0))
            .collect();

        // A point without an ID or properties, and one with a numeric ID
        features.push(Feature::from(Geometry::new_point([-100.0, -30.0])));
        features.push(Feature {
            id: Some(Id::Number(7.into())),
            ..create_feature("b", 100.0, 30.0)
        });

        let supercluster = Supercluster::load_features(Options::default(), features).unwrap();

        // A cluster and points
        let tile = supercluster.tile(0, 0, 0);

        assert_eq!(tile.items.len(), 3);
        assert_eq!(
            read_back(tile.as_feature_collection()),
            tile.to_feature_collection()
        );

        // Only points, with their properties
        let tile = supercluster.with_cluster_max_zoom(0).tile(1, 1, 0);

        assert!(tile.items.len() > 2);
        assert_eq!(
            read_back(tile.as_feature_collection()),
            tile.to_feature_collection()
        );
    }

    #[test]
    fn test_as_feature_collection_writes_whole_numbers() {
        let supercluster = load_features();
        let tile = supercluster.with_cluster_max_zoom(0).tile(1, 1, 0);
        let json = serde_json::to_value(tile.as_feature_collection()).unwrap();
        let features = json["features"].as_array().unwrap();

        // Written as whole numbers, such as `150` and not `150.0`
        assert_eq!(features.len(), 2);
        assert!(features
            .iter()
            .flat_map(|feature| feature["geometry"]["coordinates"].as_array().unwrap())
            .all(|value| value.is_i64()));
    }

    #[test]
    fn test_filtered_tile_as_feature_collection_serializes_like_to_feature_collection() {
        let mut features: Vec<Feature> = (0..1500)
            .map(|i| create_feature("a", 10.0 + f64::from(i) * 1e-7, 50.0))
            .collect();

        // A point without an ID or properties, and one with a numeric ID
        features.push(Feature::from(Geometry::new_point([-100.0, -30.0])));
        features.push(Feature {
            id: Some(Id::Number(7.into())),
            ..create_feature("b", 100.0, 30.0)
        });

        let supercluster = Supercluster::load_features(Options::default(), features).unwrap();

        // The western half of the cluster passes, so the cluster moves, and so do the points
        let west =
            |feature: &Feature| get_coordinates(feature).is_some_and(|[lng, _]| lng < 10.000075);
        let kept = supercluster.filter(|feature| {
            west(feature) || feature.id.is_none() || feature.id == Some(Id::Number(7.into()))
        });
        let tile = supercluster.with_filter(&kept).unwrap().tile(0, 0, 0);
        let unfiltered = supercluster.tile(0, 0, 0);

        assert_eq!(tile.items.len(), 3);
        assert_eq!(
            read_back(tile.as_feature_collection()),
            tile.to_feature_collection()
        );

        // A cluster counts the points left, at their center, with the ID it has without a filter
        for (item, feature) in tile.items.iter().zip(tile.to_feature_collection().features) {
            let Item::Cluster(cluster) = item.item() else {
                continue;
            };
            let plain = unfiltered
                .items
                .iter()
                .find(|plain| matches!(plain.item(), Item::Cluster(c) if c.id() == cluster.id()))
                .unwrap()
                .to_feature();

            assert_eq!(item.count(), 750);
            assert_eq!(feature.property("point_count"), Some(&JsonValue::from(750)));
            assert_eq!(feature.property("cluster_id"), plain.property("cluster_id"));
            assert_eq!(feature.id, plain.id);
            assert_eq!(
                feature.geometry,
                Some(Geometry::new_point([
                    f64::from(item.x()),
                    f64::from(item.y())
                ]))
            );
            assert_ne!(item.center(), Some(cluster.center()));
        }
    }
}
