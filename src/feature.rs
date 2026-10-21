//! # Feature module
//!
//! Contains the GeoJSON support of the supercluster crate, which the `geojson` feature enables.
//!
//! A list of GeoJSON features is loaded into a supercluster as it is, with `load_features`. The
//! clusters and tiles it returns convert back into GeoJSON features.

use crate::{Cluster, Item, Supercluster, SuperclusterError, SuperclusterOptions, Tile, TileItem};
use geojson::{
    feature::Id, Feature, FeatureCollection, Geometry, GeometryValue, JsonObject, JsonValue,
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
    /// Returns `SuperclusterError::TooManyPoints` if there are more than `u32::MAX` features.
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

impl Cluster {
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
        Feature {
            id: Some(Id::String(self.id.get().to_string())),
            bbox: None,
            geometry: Some(Geometry::new_point([self.x, self.y])),
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
        let geometry = Some(Geometry::new_point([self.x, self.y]));

        match &self.item {
            Item::Cluster(cluster) => Feature {
                geometry,
                ..cluster.to_feature()
            },
            Item::Point { point, .. } => Feature {
                id: point.id.clone(),
                bbox: None,
                geometry,
                properties: point.properties.clone(),
                foreign_members: None,
            },
        }
    }
}

impl Tile<'_, Feature> {
    /// Convert the tile into a GeoJSON feature collection.
    pub fn to_feature_collection(&self) -> FeatureCollection {
        FeatureCollection {
            bbox: None,
            features: self.items.iter().map(|item| item.to_feature()).collect(),
            foreign_members: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ClusterId;

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
        let cluster = Cluster {
            id: ClusterId::new(7),
            point_count: 1500,
            x: 10.0,
            y: 50.0,
        };
        let feature = cluster.to_feature();

        assert_eq!(feature.id, Some(Id::String("7".to_string())));
        assert_eq!(feature.geometry, Some(Geometry::new_point([10.0, 50.0])));
        assert_eq!(feature.property("cluster"), Some(&JsonValue::from(true)));
        assert_eq!(feature.property("cluster_id"), Some(&JsonValue::from(7)));
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
}
