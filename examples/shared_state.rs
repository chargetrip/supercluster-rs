//! Serve filtered tiles from state that every request shares, and refresh a selection when the data
//! it reads changes. A web server keeps `State` in its application state and calls `handle_tile`
//! from its tile route.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, PoisonError, RwLock,
};

use supercluster::{Combination, Selection, Supercluster, SuperclusterError, SuperclusterOptions};

/// A charging station.
struct Station {
    name: &'static str,
    lng: f64,
    lat: f64,
    power_kw: u32,

    /// Whether the station is free. It changes after loading, so it is shared through the station,
    /// and the selection that reads it is made again when it changes.
    available: AtomicBool,
}

/// What every request reads: the supercluster and the selections made from it. They are replaced
/// together, so a selection never meets another supercluster.
struct Index {
    supercluster: Arc<Supercluster<Station>>,

    /// The fast stations. The power of a station does not change, so this selection stays valid,
    /// and a refresh shares it with the new index instead of copying it.
    fast: Arc<Selection>,

    /// The free stations, made again when the availability changes.
    available: Selection,
}

/// The state that every request shares.
struct State {
    /// The current index. A request takes it and works with it, and a change swaps in a new one,
    /// which the requests after it take.
    index: RwLock<Arc<Index>>,

    /// Taken by every change of the index, such as a refresh or a load of new points, so changes
    /// run one at a time and one never puts back an index that another replaced. Requests do not
    /// take it, so they never wait for a change.
    changes: Mutex<()>,
}

/// Get the current index. A request holds it until it is done, without holding the lock.
fn current(state: &State) -> Arc<Index> {
    // A panic while the lock was held cannot leave the index half replaced, so a poisoned lock is
    // still safe to read
    Arc::clone(&state.index.read().unwrap_or_else(PoisonError::into_inner))
}

/// Answer a tile request: the position and the number of stations of every item, counting only
/// the free stations, and only the fast ones if the request asks for that.
fn handle_tile(state: &State, (z, x, y): (u8, u32, u32), only_fast: bool) -> Vec<([f64; 2], u32)> {
    let index = current(state);

    // A combination per request, which the query evaluates on the stations of the tile only
    let wanted = if only_fast {
        index.available.and(index.fast.as_ref())
    } else {
        Combination::from(&index.available)
    };

    // The items borrow the combination, which lives in a variable until the response is built
    let items = index.supercluster.tile_selected(z, x, y, &wanted);

    items
        .iter()
        .map(|item| (item.coordinates(), item.count()))
        .collect()
}

/// Make the selection of free stations again, after their availability changed, and swap it in.
/// Requests that already hold the old index finish with it.
fn refresh_availability(state: &State) {
    let _change = state.changes.lock().unwrap_or_else(PoisonError::into_inner);
    let index = current(state);

    // Relaxed is enough for a flag on its own. What makes a refresh see a change of availability
    // is how the change starts the refresh, for example through a channel or a lock
    let available = index
        .supercluster
        .select(|station| station.available.load(Ordering::Relaxed));
    let refreshed = Index {
        supercluster: Arc::clone(&index.supercluster),
        fast: Arc::clone(&index.fast),
        available,
    };

    *state.index.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(refreshed);
}

fn main() -> Result<(), SuperclusterError> {
    let station = |name, lng, lat, power_kw, available| Station {
        name,
        lng,
        lat,
        power_kw,
        available: AtomicBool::new(available),
    };
    let supercluster = Arc::new(Supercluster::load(
        SuperclusterOptions::default(),
        vec![
            station("Dam", 4.8936, 52.3731, 150, true),
            station("Leidseplein", 4.8828, 52.3643, 11, true),
            station("Vondelpark", 4.8686, 52.3579, 150, false),
        ],
        |station| Some([station.lng, station.lat]),
    )?);
    let index = Index {
        fast: Arc::new(supercluster.select(|station| station.power_kw >= 50)),
        available: supercluster.select(|station| station.available.load(Ordering::Relaxed)),
        supercluster,
    };
    let state = State {
        index: RwLock::new(Arc::new(index)),
        changes: Mutex::new(()),
    };

    println!("Free: {:?}", handle_tile(&state, (0, 0, 0), false));
    println!("Free and fast: {:?}", handle_tile(&state, (0, 0, 0), true));

    // Vondelpark becomes free, and the requests after the refresh count it. The points are in the
    // order of the clusters, so the station is found by its name.
    let index = current(&state);

    if let Some(station) = index
        .supercluster
        .points()
        .iter()
        .find(|station| station.name == "Vondelpark")
    {
        station.available.store(true, Ordering::Relaxed);
    }

    refresh_availability(&state);

    println!(
        "Free and fast after the refresh: {:?}",
        handle_tile(&state, (0, 0, 0), true)
    );

    Ok(())
}
