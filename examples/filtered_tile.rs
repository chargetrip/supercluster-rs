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

    // A filter runs on each request, on the stations of the tile only. The cluster keeps the
    // available stations only and moves to their center, and Rotterdam is left out
    let available = supercluster.filter(|station| station.available);
    let descriptions = supercluster
        .with_filter(&available)?
        .tile(0, 0, 0)
        .items
        .into_iter()
        .map(|tile_item| {
            let names: Vec<_> = tile_item.leaves().map(|station| station.name).collect();
            let [lng, lat] = tile_item.coordinates();

            match tile_item.item() {
                Item::Cluster(cluster) => {
                    // The zoom level a map zooms in to, counting the available stations only
                    let expansion = tile_item.expansion_zoom().map_or(String::new(), |zoom| {
                        format!(", expands on zoom level {zoom}")
                    });

                    format!(
                        "cluster {} with {} of {} stations {:?} at {lng}, {lat}{expansion}",
                        cluster.id().get(),
                        tile_item.count(),
                        cluster.total_count(),
                        names,
                    )
                }
                Item::Point { .. } => format!("station {names:?} at {lng}, {lat}"),
            }
        });

    for description in descriptions {
        println!("{description}");
    }

    // To filter many tiles the same way, run the filter once and reuse it until the availability
    // changes. The tiles then cost the same however many stations they hold.
    let cached = supercluster.cache_filter(|station| station.available);

    for tile_item in supercluster.with_filter(&cached)?.tile(0, 0, 0).items {
        println!(
            "{} available station(s), zoom in to {:?}",
            tile_item.count(),
            tile_item.expansion_zoom()
        );
    }

    Ok(())
}
