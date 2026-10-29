use geojson::{Feature, FeatureCollection, GeometryValue};
use std::{fs, path::Path};
use supercluster::DataRange;

pub fn get_data_range(data: &[Feature]) -> Option<DataRange> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    let points = data
        .iter()
        .filter_map(|feature| match &feature.geometry.as_ref()?.value {
            GeometryValue::Point { coordinates } => Some([coordinates[0], coordinates[1]]),
            _ => None,
        });

    for [x, y] in points {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    if min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite() {
        Some(DataRange::new(min_x, min_y, max_x, max_y))
    } else {
        None
    }
}

pub fn load_places() -> Vec<Feature> {
    let file_path = Path::new("./tests/common/places.json");
    let json_string = fs::read_to_string(file_path).expect("places.json was not found");

    serde_json::from_str(&json_string).expect("places.json was not parsed")
}

pub fn load_tile_places() -> FeatureCollection {
    let file_path = Path::new("./tests/common/places-tile-0-0-0.json");
    let json_string = fs::read_to_string(file_path).expect("places-tile-0-0-0.json was not found");

    serde_json::from_str(&json_string).expect("places-tile-0-0-0.json was not parsed")
}

pub fn load_tile_places_with_min_5() -> FeatureCollection {
    let file_path = Path::new("./tests/common/places-tile-0-0-0-min-5.json");
    let json_string =
        fs::read_to_string(file_path).expect("places-tile-0-0-0-min-5.json was not found");

    serde_json::from_str(&json_string).expect("places-z0-0-0-min5.json was not parsed")
}

pub fn load_cartesian() -> Vec<Feature> {
    let file_path = Path::new("./tests/common/cartesian.json");
    let json_string = fs::read_to_string(file_path).expect("cartesian.json was not found");

    serde_json::from_str(&json_string).expect("cartesian.json was not parsed")
}
