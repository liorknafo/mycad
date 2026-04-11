//! Errors raised by the parametric framework.

use thiserror::Error;
use uuid::Uuid;

/// Errors returned from [`crate::parametric`] operations and feature builds.
#[derive(Debug, Error)]
pub enum ParametricError {
    #[error("input resolution failed for node {node}: {reason}")]
    InputResolutionFailed { node: Uuid, reason: String },

    #[error("feature build failed at node {node}: {reason}")]
    BuildFailed { node: Uuid, reason: String },

    #[error("dependency cycle detected among nodes {nodes:?}")]
    CycleDetected { nodes: Vec<Uuid> },

    #[error("invalid branch operation: {0}")]
    InvalidBranchOperation(String),

    #[error("no active component on the current branch")]
    ActiveComponentMissing,

    #[error("cannot edit the root node")]
    RootNodeEdit,

    #[error("node {0} not found in the document")]
    NodeNotFound(Uuid),

    #[error("component {0} not found in the document")]
    ComponentNotFound(Uuid),

    #[error("branch {0} not found in the document")]
    BranchNotFound(Uuid),

    #[error("unknown parametric error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ParametricError>;
