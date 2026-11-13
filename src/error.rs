//! # Error module
//!
//! Contains the error type for the supercluster crate.

use thiserror::Error;

use crate::{MAX_POINTS, MAX_ZOOM_LIMIT};

/// Supercluster error.
/// Represents the different errors that can occur in the supercluster crate.
#[derive(Clone, Debug, Error, PartialEq, Eq, Hash)]
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

    /// The cluster radius is not a finite number above 0, or the extent plus the radius is above
    /// `i32::MAX`, which the positions in a tile could then not hold.
    #[error(
        "The radius must be a finite number above 0, and the extent plus the radius cannot be above {}.",
        i32::MAX
    )]
    InvalidRadius,

    /// The tile extent is 0 or above `i32::MAX`.
    #[error("The extent must be from 1 to {}.", i32::MAX)]
    InvalidExtent,

    /// The data range of the `Cartesian` coordinate system cannot normalize coordinates.
    #[error(
        "The data range must have finite bounds, no maximum below its minimum, more than one value and a width that an f64 holds."
    )]
    InvalidDataRange,

    /// The node size of the KD-tree is 0.
    #[error("The node size cannot be 0.")]
    InvalidNodeSize,

    /// There are more points than the index can address.
    #[error("The number of points cannot be above {}.", MAX_POINTS)]
    TooManyPoints,

    /// A cached filter that `with_filter` got, on its own or in a combined filter, comes from
    /// another supercluster.
    #[error("The cached filter comes from another supercluster.")]
    ForeignCachedFilter,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_max_zoom_message_has_the_limit() {
        assert_eq!(
            SuperclusterError::InvalidMaxZoom.to_string(),
            format!("The maximum zoom level cannot be above {MAX_ZOOM_LIMIT}.")
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
    fn test_invalid_radius_message_has_the_limit() {
        assert_eq!(
            SuperclusterError::InvalidRadius.to_string(),
            format!(
                "The radius must be a finite number above 0, and the extent plus the radius cannot be above {}.",
                i32::MAX
            )
        );
    }

    #[test]
    fn test_invalid_extent_message_has_the_limit() {
        assert_eq!(
            SuperclusterError::InvalidExtent.to_string(),
            format!("The extent must be from 1 to {}.", i32::MAX)
        );
    }

    #[test]
    fn test_invalid_data_range_message() {
        assert_eq!(
            SuperclusterError::InvalidDataRange.to_string(),
            "The data range must have finite bounds, no maximum below its minimum, more than one value and a width that an f64 holds."
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
            format!("The number of points cannot be above {MAX_POINTS}.")
        );
    }

    #[test]
    fn test_foreign_cached_filter_message() {
        assert_eq!(
            SuperclusterError::ForeignCachedFilter.to_string(),
            "The cached filter comes from another supercluster."
        );
    }
}
