use supercluster::{Item, Supercluster, SuperclusterError, SuperclusterOptions};

/// A charging station that can be in use.
struct Station {
    name: &'static str,
    lng: f64,
    lat: f64,
    available: bool,
}

fn main() -> Result<(), SuperclusterError> {
    let supercluster = Supercluster::load(
        SuperclusterOptions::default(),
        vec![
            Station {
                name: "Dam",
                lng: 4.8936,
                lat: 52.3731,
                available: true,
            },
            Station {
                name: "Leidseplein",
                lng: 4.8828,
                lat: 52.3643,
                available: false,
            },
            Station {
                name: "Vondelpark",
                lng: 4.8686,
                lat: 52.3579,
                available: true,
            },
            Station {
                name: "Rotterdam",
                lng: 4.4777,
                lat: 51.9244,
                available: false,
            },
        ],
        |station| Some([station.lng, station.lat]),
    )?;

    // The cluster keeps the available stations only and moves to their center, Rotterdam is left out
    let descriptions = supercluster
        .tile_filtered(0, 0, 0, |station| station.available)
        .into_iter()
        .map(|tile_item| {
            let names: Vec<_> = tile_item.points().map(|station| station.name).collect();
            let [lng, lat] = tile_item.coordinates;

            match tile_item.item {
                Item::Cluster(cluster) => {
                    // The zoom level a map zooms in to, counting the available stations only
                    let expansion = tile_item.expansion_zoom().map_or(String::new(), |zoom| {
                        format!(", expands on zoom level {zoom}")
                    });

                    format!(
                        "cluster {} with {} of {} stations {:?} at {lng}, {lat}{expansion}",
                        cluster.id.get(),
                        tile_item.count(),
                        cluster.point_count,
                        names,
                    )
                }
                Item::Point { .. } => format!("station {names:?} at {lng}, {lat}"),
            }
        });

    for description in descriptions {
        println!("{description}");
    }

    Ok(())
}
