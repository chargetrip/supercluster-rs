use geojson::{Feature, FeatureCollection};
use std::{fs, path::Path};
use supercluster::{CoordinateSystem, Options};

pub fn get_options(
    radius: f64,
    extent: f64,
    min_points: u8,
    max_zoom: u8,
    coordinate_system: CoordinateSystem,
) -> Options {
    Options {
        radius,
        extent,
        max_zoom,
        min_zoom: 0,
        min_points,
        node_size: 64,
        coordinate_system,
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
