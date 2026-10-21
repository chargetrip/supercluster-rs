#![cfg(feature = "tracing")]

use std::{
    io,
    sync::{Arc, Mutex},
};

use supercluster::{Supercluster, SuperclusterBuilder};
use tracing::Level;
use tracing_subscriber::fmt::MakeWriter;

/// Keeps what a subscriber writes in memory.
#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    /// Get the text written so far.
    fn output(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Buffer {
    type Writer = Buffer;

    fn make_writer(&'a self) -> Buffer {
        self.clone()
    }
}

/// Run the function with a subscriber that writes every event, with its level and fields, to the
/// buffer.
fn with_subscriber<T>(buffer: &Buffer, f: impl FnOnce() -> T) -> T {
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::TRACE)
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_writer(buffer.clone())
        .finish();

    tracing::subscriber::with_default(subscriber, f)
}

/// Get the line of the output that has the message.
fn get_line<'a>(output: &'a str, message: &str) -> &'a str {
    output
        .lines()
        .find(|line| line.contains(message))
        .unwrap_or_else(|| panic!("no line with `{message}` in:\n{output}"))
}

#[test]
fn test_load_reports_points_without_coordinates() {
    let buffer = Buffer::default();

    with_subscriber(&buffer, || {
        let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build().unwrap());

        // A point that is `None` has no coordinates
        supercluster
            .load_with(
                vec![Some([10.0, 50.0]), Some([10.0001, 50.0]), None],
                |point| *point,
            )
            .unwrap();
    });

    let output = buffer.output();
    let line = get_line(&output, "Points without finite coordinates are left out");

    assert!(line.contains("WARN"));
    assert!(line.contains("total=3"));
    assert!(line.contains("left_out=1"));
}

#[test]
fn test_load_reports_points_with_coordinates_that_are_not_finite() {
    let buffer = Buffer::default();

    with_subscriber(&buffer, || {
        let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build().unwrap());

        supercluster
            .load_with(
                vec![[10.0, 50.0], [f64::NAN, 50.0], [10.0, f64::INFINITY]],
                |point| Some(*point),
            )
            .unwrap();
    });

    let output = buffer.output();
    let line = get_line(&output, "Points without finite coordinates are left out");

    assert!(line.contains("total=3"));
    assert!(line.contains("left_out=2"));
}

#[test]
fn test_load_does_not_warn_when_every_point_has_coordinates() {
    let buffer = Buffer::default();

    with_subscriber(&buffer, || {
        let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build().unwrap());

        supercluster
            .load_with(vec![[10.0, 50.0], [10.0001, 50.0]], |point| Some(*point))
            .unwrap();
    });

    assert!(!buffer.output().contains("WARN"));
}

#[test]
fn test_load_logs_a_summary() {
    let buffer = Buffer::default();

    with_subscriber(&buffer, || {
        let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build().unwrap());

        supercluster
            .load_with(vec![[10.0, 50.0], [10.0001, 50.0]], |point| Some(*point))
            .unwrap();
    });

    let output = buffer.output();
    let line = get_line(&output, "Loaded points into the supercluster");

    assert!(line.contains("DEBUG"));
    assert!(line.contains("points=2"));
    assert!(line.contains("clusters=1"));
    assert!(line.contains("min_zoom=0"));
    assert!(line.contains("max_zoom=16"));
}

#[test]
fn test_load_traces_every_zoom_level() {
    let buffer = Buffer::default();

    with_subscriber(&buffer, || {
        let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build().unwrap());

        supercluster
            .load_with(vec![[10.0, 50.0], [10.0001, 50.0]], |point| Some(*point))
            .unwrap();
    });

    let output = buffer.output();
    let zoom_levels: Vec<&str> = output
        .lines()
        .filter(|line| line.contains("Clustered zoom level"))
        .collect();

    // From the maximum zoom level of 16 down to 0
    assert_eq!(zoom_levels.len(), 17);
    assert!(zoom_levels.iter().all(|line| line.contains("TRACE")));
    assert!(zoom_levels[0].contains("zoom=16"));
    assert!(zoom_levels[16].contains("zoom=0"));
}

/// Run a query on a loaded supercluster and get what was reported while it ran.
/// A server queries all the time, so a query must not report anything.
fn get_output_of_query(query: impl FnOnce(&Supercluster<[f64; 2]>)) -> String {
    let buffer = Buffer::default();
    let mut supercluster = Supercluster::new(SuperclusterBuilder::new().build().unwrap());

    supercluster
        .load_with(vec![[10.0, 50.0], [10.0001, 50.0]], |point| Some(*point))
        .unwrap();

    with_subscriber(&buffer, || query(&supercluster));

    buffer.output()
}

#[test]
fn test_get_tile_traces_nothing() {
    assert_eq!(
        get_output_of_query(|supercluster| {
            supercluster.get_tile(0, 0, 0);
        }),
        ""
    );
}

#[test]
fn test_get_clusters_traces_nothing() {
    assert_eq!(
        get_output_of_query(|supercluster| {
            supercluster.get_clusters([-180.0, -90.0, 180.0, 90.0], 3);
        }),
        ""
    );
}

#[test]
fn test_get_children_traces_nothing() {
    assert_eq!(
        get_output_of_query(|supercluster| {
            let _ = supercluster.get_children(5);
        }),
        ""
    );
}

#[test]
fn test_get_leaves_traces_nothing() {
    assert_eq!(
        get_output_of_query(|supercluster| {
            let _ = supercluster.get_leaves(5, 10, 0);
        }),
        ""
    );
}

#[test]
fn test_get_cluster_expansion_zoom_traces_nothing() {
    assert_eq!(
        get_output_of_query(|supercluster| {
            let _ = supercluster.get_cluster_expansion_zoom(5);
        }),
        ""
    );
}
