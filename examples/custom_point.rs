use supercluster::{Item, Supercluster, SuperclusterError, SuperclusterOptions};

/// A charging station, which is clustered as it is.
struct Station {
    name: &'static str,
    lng: f64,
    lat: f64,
}

fn main() -> Result<(), SuperclusterError> {
    // The stations are kept as they are, nothing is converted or copied. The closure tells where
    // a station is.
    let supercluster = Supercluster::load(
        SuperclusterOptions::default(),
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
    let tile = supercluster.tile(0, 0, 0);

    for tile_item in &tile.items {
        match &tile_item.item {
            Item::Cluster(cluster) => {
                let names: Vec<_> = supercluster
                    .leaves(cluster.id)?
                    .iter()
                    .take(10)
                    .map(|station| station.name)
                    .collect();

                println!("Cluster of {} stations: {:?}", cluster.point_count, names);
            }
            Item::Point { point, .. } => println!("Station {}", point.name),
        }
    }

    Ok(())
}
