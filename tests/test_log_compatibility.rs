#![cfg(feature = "tracing")]

//! An application that only uses the `log` crate must see what the supercluster reports, because
//! the `tracing` crate turns its events into `log` records when there is no tracing subscriber.

use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};
use supercluster::{Supercluster, SuperclusterBuilder};

/// Keeps the log records in memory.
struct MemoryLogger {
    /// The level and the message of every record.
    records: Mutex<Vec<(Level, String)>>,
}

impl Log for MemoryLogger {
    fn enabled(&self, _: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        self.records
            .lock()
            .unwrap()
            .push((record.level(), record.args().to_string()));
    }

    fn flush(&self) {}
}

/// The logger of this test, which is the only one in this test binary.
static LOGGER: MemoryLogger = MemoryLogger {
    records: Mutex::new(Vec::new()),
};

#[test]
fn test_load_is_logged_without_a_tracing_subscriber() {
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(LevelFilter::Debug);

    let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build().unwrap());

    supercluster
        // A point that is `None` has no coordinates
        .load_with(
            vec![Some([10.0, 50.0]), Some([10.0001, 50.0]), None],
            |point| *point,
        )
        .unwrap();

    let records = LOGGER.records.lock().unwrap();

    assert!(records.iter().any(|(level, message)| {
        *level == Level::Warn && message.contains("Points without finite coordinates are left out")
    }));
    assert!(records.iter().any(|(level, message)| {
        *level == Level::Debug && message.contains("Loaded points into the supercluster")
    }));
}
