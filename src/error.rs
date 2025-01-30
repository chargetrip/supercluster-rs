//! # Error module
//!
//! Contains the error type for the supercluster crate.

use thiserror::Error;

/// Supercluster error.
/// Represents the different errors that can occur in the supercluster crate.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SuperclusterError {
    /// Cluster not found with the specified ID.
    #[error("Cluster not found with the specified ID.")]
    ClusterNotFound,

    /// Tree not found at the specified zoom level.
    #[error("Tree not found at the specified zoom level.")]
    TreeNotFound,

    /// Tile not found at the specified coordinates and zoom level.
    #[error("Tile not found at the specified coordinates and zoom level.")]
    TileNotFound,
}
