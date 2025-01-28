use thiserror::Error;

/// Supercluster error.
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
