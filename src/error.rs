//! # Error module
//!
//! Contains the error type for the supercluster crate.

use thiserror::Error;

use crate::{MAX_POINTS, MAX_ZOOM_LIMIT};

/// Supercluster error.
/// Represents the different errors that can occur in the supercluster crate.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum SuperclusterError {
    /// Cluster not found with the specified ID.
    #[error("Cluster not found with the specified ID.")]
    ClusterNotFound,

    /// The maximum zoom level is above the highest supported one.
    #[error("The maximum zoom level cannot be above {}.", MAX_ZOOM_LIMIT)]
    InvalidMaxZoom,

    /// The minimum zoom level is above the maximum zoom level.
    #[error("The minimum zoom level cannot be above the maximum zoom level.")]
    InvalidMinZoom,

    /// The cluster radius is not a finite number above 0.
    #[error("The radius must be a finite number above 0.")]
    InvalidRadius,

    /// The tile extent is not a finite number above 0.
    #[error("The extent must be a finite number above 0.")]
    InvalidExtent,

    /// The data range of the `Cartesian` coordinate system cannot normalize coordinates.
    #[error(
        "The data range must have finite bounds, no maximum below its minimum and more than one value."
    )]
    InvalidDataRange,

    /// The node size of the KD-tree is 0.
    #[error("The node size cannot be 0.")]
    InvalidNodeSize,

    /// There are more points than the index can address.
    #[error("The number of points cannot be above {}.", MAX_POINTS)]
    TooManyPoints,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_max_zoom_message_has_the_limit() {
        assert_eq!(
            SuperclusterError::InvalidMaxZoom.to_string(),
            format!("The maximum zoom level cannot be above {}.", MAX_ZOOM_LIMIT)
        );
    }

    #[test]
    fn test_invalid_min_zoom_message() {
        assert_eq!(
            SuperclusterError::InvalidMinZoom.to_string(),
            "The minimum zoom level cannot be above the maximum zoom level."
        );
    }

    #[test]
    fn test_invalid_radius_message() {
        assert_eq!(
            SuperclusterError::InvalidRadius.to_string(),
            "The radius must be a finite number above 0."
        );
    }

    #[test]
    fn test_invalid_extent_message() {
        assert_eq!(
            SuperclusterError::InvalidExtent.to_string(),
            "The extent must be a finite number above 0."
        );
    }

    #[test]
    fn test_invalid_data_range_message() {
        assert_eq!(
            SuperclusterError::InvalidDataRange.to_string(),
            "The data range must have finite bounds, no maximum below its minimum and more than one value."
        );
    }

    #[test]
    fn test_invalid_node_size_message() {
        assert_eq!(
            SuperclusterError::InvalidNodeSize.to_string(),
            "The node size cannot be 0."
        );
    }

    #[test]
    fn test_too_many_points_message_has_the_limit() {
        assert_eq!(
            SuperclusterError::TooManyPoints.to_string(),
            format!("The number of points cannot be above {}.", MAX_POINTS)
        );
    }
}
