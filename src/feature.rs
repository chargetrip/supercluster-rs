//! # Feature module
//!
//! Contains the GeoJSON support of the supercluster crate, which the `geojson` feature enables.
//!
//! A list of GeoJSON features is loaded into a supercluster as it is, with `load_features`. The
//! clusters and tiles it returns convert back into GeoJSON features, or serialize as GeoJSON
//! without copying the features.

use geojson::{
    feature::Id, Feature, FeatureCollection, Geometry, GeometryValue, JsonObject, JsonValue,
};
use serde::{ser::SerializeMap, Serialize, Serializer};
use std::fmt::Display;

use crate::{
    supercluster::AbbreviatedCount, Cluster, Item, Supercluster, SuperclusterError,
    SuperclusterOptions, Tile, TileItem,
};

impl Supercluster<Feature> {
    /// Load GeoJSON features into a new supercluster, like `load`.
    /// A feature with a point geometry is clustered by its longitude and latitude, or by its X and
    /// Y values with the `Cartesian` coordinate system. A third value, the altitude, is ignored.
    /// Any other feature, and a point whose coordinates are not finite numbers, is left out of the
    /// clustering.
    ///
    /// # Errors
    ///
    /// Returns `SuperclusterError::TooManyPoints` if there are more than `MAX_POINTS` features.
    ///
    /// # Examples
    ///
    /// ```
    /// use geojson::{Feature, Geometry};
    /// use supercluster::{Supercluster, SuperclusterOptions};
    ///
    /// let features: Vec<Feature> = [[-77.032, 38.913], [-77.033, 38.913]]
    ///     .into_iter()
    ///     .map(|point| Feature::from(Geometry::new_point(point)))
    ///     .collect();
    ///
    /// let supercluster = Supercluster::load_features(SuperclusterOptions::default(), features)?;
    ///
    /// let tile = supercluster.tile(0, 0, 0).to_feature_collection();
    ///
    /// assert_eq!(tile.features.len(), 1);
    /// # Ok::<(), supercluster::SuperclusterError>(())
    /// ```
    pub fn load_features(
        options: SuperclusterOptions,
        features: Vec<Feature>,
    ) -> Result<Self, SuperclusterError> {
        Self::load(options, features, get_coordinates)
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
    /// Get the GeoJSON properties of the cluster: `cluster`, `cluster_id`, `point_count` and
    /// `point_count_abbreviated`.
    fn get_properties(&self) -> JsonObject {
        let mut properties = JsonObject::new();

        properties.insert("cluster".to_string(), JsonValue::from(true));
        properties.insert("cluster_id".to_string(), JsonValue::from(self.id.get()));
        properties.insert("point_count".to_string(), JsonValue::from(self.point_count));
        properties.insert(
            "point_count_abbreviated".to_string(),
            JsonValue::from(self.point_count_abbreviated()),
        );

        properties
    }

    /// Convert the cluster into a GeoJSON feature, with its center as the geometry.
    pub fn to_feature(&self) -> Feature {
        self.feature_at(Geometry::new_point(self.coordinates()))
    }

    /// Convert the cluster into a GeoJSON feature with the geometry, so a tile does not compute
    /// a center it replaces with the position in the tile.
    fn feature_at(&self, geometry: Geometry) -> Feature {
        Feature {
            id: Some(Id::String(self.id.get().to_string())),
            bbox: None,
            geometry: Some(geometry),
            properties: Some(self.get_properties()),
            foreign_members: None,
        }
    }
}

impl Item<'_, Feature> {
    /// Convert the item into a GeoJSON feature.
    pub fn to_feature(&self) -> Feature {
        match self {
            Item::Cluster(cluster) => cluster.to_feature(),
            Item::Point { point, .. } => (*point).clone(),
        }
    }
}

impl TileItem<'_, Feature> {
    /// Convert the item into a GeoJSON feature, with its position in the tile as the geometry.
    pub fn to_feature(&self) -> Feature {
        let geometry = Geometry::new_point([self.x, self.y]);

        match &self.item {
            Item::Cluster(cluster) => cluster.feature_at(geometry),
            Item::Point { point, .. } => Feature {
                id: point.id.clone(),
                bbox: None,
                geometry: Some(geometry),
                properties: point.properties.clone(),
                foreign_members: None,
            },
        }
    }
}

impl<'a> Tile<'a, Feature> {
    /// Convert the tile into a GeoJSON feature collection.
    pub fn to_feature_collection(&self) -> FeatureCollection {
        FeatureCollection {
            bbox: None,
            features: self.items.iter().map(|item| item.to_feature()).collect(),
            foreign_members: None,
        }
    }

    /// Get a view of the tile as a GeoJSON feature collection, which serializes to the same JSON
    /// as `to_feature_collection` but borrows the properties of the features instead of copying
    /// them. Use it to send a tile as GeoJSON, for example with `serde_json::to_writer`.
    ///
    /// # Examples
    ///
    /// ```
    /// use geojson::{Feature, Geometry};
    /// use supercluster::{Supercluster, SuperclusterOptions};
    ///
    /// let features = vec![Feature::from(Geometry::new_point([-77.032, 38.913]))];
    /// let supercluster = Supercluster::load_features(SuperclusterOptions::default(), features)?;
    /// let tile = supercluster.tile(0, 0, 0);
    ///
    /// let json = serde_json::to_string(&tile.as_feature_collection())?;
    ///
    /// assert_eq!(json, serde_json::to_string(&tile.to_feature_collection())?);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn as_feature_collection(&self) -> FeatureCollectionView<'_, 'a> {
        FeatureCollectionView { tile: self }
    }
}

/// A view of a tile as a GeoJSON feature collection, which `Tile::as_feature_collection` returns.
/// It serializes like the feature collection that `Tile::to_feature_collection` returns, without
/// copying the features.
#[derive(Clone, Copy, Debug)]
pub struct FeatureCollectionView<'t, 'a> {
    /// The tile to serialize.
    tile: &'t Tile<'a, Feature>,
}

impl Serialize for FeatureCollectionView<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;

        map.serialize_entry("type", "FeatureCollection")?;
        map.serialize_entry("features", &FeaturesView(&self.tile.items))?;
        map.end()
    }
}

/// The items of a tile, which serialize as a list of GeoJSON features.
struct FeaturesView<'t, 'a>(&'t [TileItem<'a, Feature>]);

impl Serialize for FeaturesView<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(FeatureView))
    }
}

/// An item of a tile, which serializes like `TileItem::to_feature` returns it: the fields in the
/// order of `geojson`, with the position in the tile as the geometry.
struct FeatureView<'t, 'a>(&'t TileItem<'a, Feature>);

impl Serialize for FeatureView<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let TileItem { x, y, item } = self.0;
        let mut map = serializer.serialize_map(None)?;

        map.serialize_entry("type", "Feature")?;
        map.serialize_entry("geometry", &PointView([*x, *y]))?;

        match item {
            Item::Cluster(cluster) => {
                map.serialize_entry("id", &DisplayView(cluster.id.get()))?;
                map.serialize_entry("properties", &ClusterPropertiesView(cluster))?;
            }
            Item::Point { point, .. } => {
                if let Some(id) = &point.id {
                    map.serialize_entry("id", id)?;
                }

                map.serialize_entry("properties", &point.properties)?;
            }
        }

        map.end()
    }
}

/// A point geometry, which serializes like `Geometry::new_point`.
struct PointView([f64; 2]);

impl Serialize for PointView {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;

        map.serialize_entry("type", "Point")?;
        map.serialize_entry("coordinates", &self.0)?;
        map.end()
    }
}

/// The GeoJSON properties of a cluster, which serialize like `Cluster::get_properties`, whose keys
/// come in alphabetical order.
struct ClusterPropertiesView<'c, 'a>(&'c Cluster<'a>);

impl Serialize for ClusterPropertiesView<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let cluster = self.0;
        let mut map = serializer.serialize_map(Some(4))?;

        map.serialize_entry("cluster", &true)?;
        map.serialize_entry("cluster_id", &cluster.id.get())?;
        map.serialize_entry("point_count", &cluster.point_count)?;
        map.serialize_entry(
            "point_count_abbreviated",
            &DisplayView(AbbreviatedCount(cluster.point_count)),
        )?;
        map.end()
    }
}

/// A value that serializes as the string `Display` writes, without allocating it.
struct DisplayView<T>(T);

impl<T: Display> Serialize for DisplayView<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            SuperclusterOptions::default(),
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
        let supercluster =
            Supercluster::load_features(SuperclusterOptions::default(), features).unwrap();

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
        let supercluster =
            Supercluster::load_features(SuperclusterOptions::default(), features).unwrap();
        let items = supercluster.clusters([-180.0, -90.0, 180.0, 90.0], 0);
        let [Item::Cluster(cluster)] = &items[..] else {
            panic!("expected a single cluster");
        };
        let feature = cluster.to_feature();
        let id = cluster.id.get();

        assert_eq!(feature.id, Some(Id::String(id.to_string())));
        assert_eq!(
            feature.geometry,
            Some(Geometry::new_point(cluster.coordinates()))
        );
        assert_eq!(feature.property("cluster"), Some(&JsonValue::from(true)));
        assert_eq!(feature.property("cluster_id"), Some(&JsonValue::from(id)));
        assert_eq!(
            feature.property("point_count"),
            Some(&JsonValue::from(1500))
        );
        assert_eq!(
            feature.property("point_count_abbreviated"),
            Some(&JsonValue::from("1.5k"))
        );
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

        let supercluster =
            Supercluster::load_features(SuperclusterOptions::default(), features).unwrap();

        // A cluster and points
        let tile = supercluster.tile(0, 0, 0);

        assert_eq!(tile.items.len(), 3);
        assert_eq!(
            to_json(&tile.as_feature_collection()),
            to_json(&tile.to_feature_collection())
        );

        // Only points, with their properties
        let tile = supercluster.with_cluster_max_zoom(0).tile(1, 1, 0);

        assert!(tile.items.len() > 2);
        assert_eq!(
            to_json(&tile.as_feature_collection()),
            to_json(&tile.to_feature_collection())
        );
    }
}
