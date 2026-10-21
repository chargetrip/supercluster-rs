use supercluster::{
    Item, Supercluster, SuperclusterBuilder, SuperclusterError, TileBuilder, TileCoordinates,
};

/// A charging station that can be in use.
struct Station {
    name: &'static str,
    lng: f64,
    lat: f64,
    available: bool,
}

/// Builds a tile with the available stations only.
struct AvailableStations {
    tile: TileCoordinates,
}

impl TileBuilder<Station, String> for AvailableStations {
    fn get_tile_coordinates(&self) -> TileCoordinates {
        self.tile
    }

    fn filter(&self, station: &Station) -> bool {
        station.available
    }

    fn map(
        &self,
        item: &Item<'_, Station>,
        stations: &[&Station],
        coordinates: [f64; 2],
    ) -> String {
        let names: Vec<_> = stations.iter().map(|station| station.name).collect();

        match item {
            Item::Cluster(cluster) => format!(
                "cluster {} with {} of {} stations {:?} at {:?}",
                cluster.id,
                stations.len(),
                cluster.point_count,
                names,
                coordinates
            ),
            Item::Point { .. } => format!("station {:?} at {:?}", names, coordinates),
        }
    }
}

fn main() -> Result<(), SuperclusterError> {
    let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build()?);

    supercluster.load_with(
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

    let builder = AvailableStations {
        tile: TileCoordinates { x: 0, y: 0, z: 0 },
    };

    // The cluster counts the available stations only and moves to their center, Rotterdam is left out
    for item in builder.build(&supercluster)? {
        println!("{item}");
    }

    Ok(())
}
