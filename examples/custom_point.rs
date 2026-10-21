use supercluster::{Item, Supercluster, SuperclusterBuilder, SuperclusterError};

/// A charging station, which is clustered as it is.
struct Station {
    name: &'static str,
    lng: f64,
    lat: f64,
}

fn main() -> Result<(), SuperclusterError> {
    let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build()?);

    // The stations are kept as they are, nothing is converted or copied. The closure tells where
    // a station is.
    supercluster.load_with(
        vec![
            Station {
                name: "Dam",
                lng: 4.8936,
                lat: 52.3731,
            },
            Station {
                name: "Leidseplein",
                lng: 4.8828,
                lat: 52.3643,
            },
            Station {
                name: "Vondelpark",
                lng: 4.8686,
                lat: 52.3579,
            },
            Station {
                name: "Rotterdam",
                lng: 4.4777,
                lat: 51.9244,
            },
        ],
        |station| Some([station.lng, station.lat]),
    )?;

    // Get a tile, here the only one on zoom level 0
    let tile = supercluster.get_tile(0, 0, 0);

    for tile_item in &tile.items {
        match &tile_item.item {
            Item::Cluster(cluster) => {
                let names: Vec<_> = supercluster
                    .get_leaves(cluster.id, 10, 0)?
                    .iter()
                    .map(|station| station.name)
                    .collect();

                println!("Cluster of {} stations: {:?}", cluster.point_count, names);
            }
            Item::Point { point, .. } => println!("Station {}", point.name),
        }
    }

    Ok(())
}
