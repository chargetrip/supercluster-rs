//! Time one request for a busy tile at every zoom level from 0 to 30, for "CCS or CHAdeMO, at least
//! 50 kW, available", four ways: a closure, cached selections computed into a combination per
//! request, cached selections combined lazily per request, and a cached combination. It prints the
//! tables of "What a request costs" in the README. Run it with:
//!
//! ```sh
//! cargo bench --bench request_cost_bench
//! ```
//!
//! It loads ten million stations. Add `-- 1000000` for one million, or `-- 1000000 7` for zoom
//! levels up to 7.

use std::hint::black_box;
use std::time::Instant;

use supercluster::{FilteredTileItem, Item, Supercluster, SuperclusterOptions};

const TYPE2: u8 = 1;
const CCS: u8 = 2;
const CHADEMO: u8 = 4;
const TESLA: u8 = 8;

struct Station {
    lng: f64,
    lat: f64,
    connectors: u8,
    power_kw: u16,
    available: bool,
}

/// A fast random number generator, so every run gets the same stations.
struct Random(u64);

impl Random {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// One million stations around 3,000 places, dense in the middle and sparse around it, and the
/// position of the busiest place.
fn create_stations(count: usize) -> (Vec<Station>, [f64; 2]) {
    let mut random = Random(0x9E37_79B9_7F4A_7C15);
    let places: Vec<([f64; 2], f64)> = (0..3000)
        .map(|_| {
            (
                [
                    -130.0 + 280.0 * random.next(),
                    -45.0 + 110.0 * random.next(),
                ],
                0.02 + 0.5 * random.next().powi(3),
            )
        })
        .collect();
    let stations = (0..count)
        .map(|_| {
            let (center, spread) = places[(random.next().powi(2) * places.len() as f64) as usize];
            let radius = (-2.0 * random.next().max(1e-12).ln()).sqrt() * spread;
            let angle = 2.0 * std::f64::consts::PI * random.next();
            let mut connectors = 0;

            for (connector, share) in [(TYPE2, 0.7), (CCS, 0.3), (CHADEMO, 0.1), (TESLA, 0.1)] {
                if random.next() < share {
                    connectors |= connector;
                }
            }

            let power_kw = [11, 22, 22, 50, 150, 350][(random.next() * 6.0) as usize];

            Station {
                lng: (center[0] + radius * angle.cos()).clamp(-180.0, 180.0),
                lat: (center[1] + radius * angle.sin()).clamp(-85.0, 85.0),
                connectors: connectors.max(TYPE2),
                power_kw,
                available: random.next() < 0.7,
            }
        })
        .collect();

    (stations, places[0].0)
}

/// The median time of one call, in microseconds. Each sample runs the call for about 20 ms, so
/// a slow call runs fewer times.
fn time<T>(mut f: impl FnMut() -> T) -> f64 {
    let start = Instant::now();

    black_box(f());

    let once = start.elapsed().as_secs_f64();
    let iterations = ((0.02 / once) as usize).clamp(1, 200);

    for _ in 0..iterations.min(20) {
        black_box(f());
    }

    let mut samples: Vec<f64> = (0..15)
        .map(|_| {
            let start = Instant::now();

            for _ in 0..iterations {
                black_box(f());
            }

            start.elapsed().as_secs_f64() * 1e6 / iterations as f64
        })
        .collect();

    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

/// Write a number with a comma every three digits.
fn thousands(number: u64) -> String {
    let digits = number.to_string();
    let mut text = String::new();

    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            text.push(',');
        }

        text.push(digit);
    }

    text
}

/// Write a duration in microseconds with three significant digits, and a comma every three digits.
fn duration(micros: f64) -> String {
    if micros >= 100.0 {
        format!("{} µs", thousands(micros.round() as u64))
    } else if micros >= 10.0 {
        format!("{micros:.1} µs")
    } else {
        format!("{micros:.2} µs")
    }
}

/// Write a duration and how many times faster it is than the closure, or slower below 1.
fn compared(micros: f64, closure: f64) -> String {
    let factor = closure / micros;
    let factor = if factor >= 10.0 {
        thousands(factor.round() as u64)
    } else {
        format!("{factor:.1}")
    };

    format!("{} ({factor}x)", duration(micros))
}

/// The tile at zoom level `z` that holds a position.
fn tile_of([lng, lat]: [f64; 2], z: u8) -> (u32, u32) {
    let n = f64::from(1u32 << z);
    let sin = lat.to_radians().sin();
    let y = 0.5 - 0.25 * ((1.0 + sin) / (1.0 - sin)).ln() / std::f64::consts::PI;

    (((lng + 180.0) / 360.0 * n) as u32, (y * n) as u32)
}

fn main() {
    // The arguments after `--`, without the `--bench` that `cargo bench` adds
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with("--"))
        .collect();

    // The number of stations, ten million unless the first argument gives another
    let count = args
        .first()
        .map_or(10_000_000, |count| count.parse().unwrap());

    // The highest zoom level, 30 unless the second argument gives a lower one
    let max_zoom: u8 = args.get(1).map_or(30, |zoom| zoom.parse().unwrap());
    let (stations, busiest) = create_stations(count);

    println!("{} points", thousands(count as u64));
    let supercluster = Supercluster::load(SuperclusterOptions::default(), stations, |station| {
        Some([station.lng, station.lat])
    })
    .unwrap();
    let wanted = |station: &Station| {
        station.connectors & (CCS | CHADEMO) != 0 && station.power_kw >= 50 && station.available
    };

    // Cached, for example every 30 seconds: one selection per connector type, power and
    // availability
    let ccs = supercluster.select(|station| station.connectors & CCS != 0);
    let chademo = supercluster.select(|station| station.connectors & CHADEMO != 0);
    let fast = supercluster.select(|station| station.power_kw >= 50);
    let available = supercluster.select(|station| station.available);
    let cached = ccs.or(&chademo).and(&fast).and(&available).compute();

    println!("| Zoom | Points under the tile | Closure | Computed per query | Lazy combination | Cached selection |");
    println!("| ---: | ---: | ---: | ---: | ---: | ---: |");

    for z in 0..=max_zoom.min(30) {
        let (x, y) = tile_of(busiest, z);
        let under: u64 = supercluster
            .tile(z, x, y)
            .items
            .iter()
            .map(|tile_item| match tile_item.item {
                Item::Cluster(cluster) => u64::from(cluster.point_count),
                Item::Point { .. } => 1,
            })
            .sum();
        let expansion_zooms = |items: Vec<FilteredTileItem<'_, Station>>| {
            items
                .iter()
                .filter_map(|item| item.expansion_zoom())
                .map(u32::from)
                .sum::<u32>()
        };

        let closure = time(|| expansion_zooms(supercluster.tile_filtered(z, x, y, wanted).items));
        let computed = time(|| {
            let selection = ccs.or(&chademo).and(&fast).and(&available).compute();

            expansion_zooms(supercluster.tile_selected(z, x, y, &selection).items)
        });
        let lazy = time(|| {
            let combination = ccs.or(&chademo).and(&fast).and(&available);

            expansion_zooms(supercluster.tile_selected(z, x, y, &combination).items)
        });
        let reused = time(|| expansion_zooms(supercluster.tile_selected(z, x, y, &cached).items));

        println!(
            "| {z} | {} | {} | {} | {} | {} |",
            thousands(under),
            duration(closure),
            compared(computed, closure),
            compared(lazy, closure),
            compared(reused, closure),
        );
    }
}
