#![cfg(feature = "serde")]

use std::fs;

use serde_json::Value;
use supercluster::{Supercluster, SuperclusterOptions};

/// Create the supercluster the fixture holds: a few points, three of them close enough to cluster,
/// on a few zoom levels, so the fixture stays small.
fn create_supercluster() -> Supercluster<[f64; 2]> {
    let options = SuperclusterOptions::builder().max_zoom(3).build().unwrap();
    let points = vec![
        [10.0, 50.0],
        [10.001, 50.0],
        [10.002, 50.001],
        [-77.03, 38.91],
        [139.69, 35.69],
    ];

    Supercluster::load(options, points, |point| Some(*point)).unwrap()
}

/// Read the fixture, which holds what `create_supercluster` writes, with the version of its format.
fn read_fixture() -> Value {
    let json = fs::read_to_string("./tests/common/supercluster-format.json").unwrap();

    serde_json::from_str(&json).unwrap()
}

#[test]
fn test_serialize_matches_the_fixture() {
    // A change to what a supercluster writes needs a new version, which a reader of the old one
    // rejects, so the fixture only changes together with the version it holds
    assert_eq!(
        serde_json::to_value(create_supercluster()).unwrap(),
        read_fixture(),
        "What a supercluster writes changed. Raise VERSION in the format module of \
         src/supercluster.rs, then write the new format into tests/common/supercluster-format.json."
    );
}

#[test]
fn test_deserialize_reads_the_fixture() {
    let supercluster = create_supercluster();
    let restored: Supercluster<[f64; 2]> = serde_json::from_value(read_fixture()).unwrap();

    assert_eq!(restored.tile(0, 0, 0), supercluster.tile(0, 0, 0));
    assert_eq!(restored.tile(3, 4, 2), supercluster.tile(3, 4, 2));
}

#[test]
fn test_deserialize_reads_back_a_format_without_field_names() {
    let supercluster = create_supercluster();

    // Postcard reads every value in the order it was written, without its name, so what a
    // supercluster writes must read back as the same types
    let bytes = postcard::to_allocvec(&supercluster).unwrap();
    let restored: Supercluster<[f64; 2]> = postcard::from_bytes(&bytes).unwrap();

    assert_eq!(restored.tile(0, 0, 0), supercluster.tile(0, 0, 0));
    assert_eq!(restored.tile(3, 4, 2), supercluster.tile(3, 4, 2));
}
