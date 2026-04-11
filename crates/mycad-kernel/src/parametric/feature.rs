//! The feature abstraction: every operation that mutates the document is a
//! variant of [`Operation`], whose payload implements [`Feature`].
//!
//! A feature declares its inputs explicitly (`inputs()`) so the rebuild
//! engine can build a dependency graph and propagate dirty markers. The
//! `build()` method takes a `BuildContext` containing the parent output
//! and resolved references, and produces a new [`FeatureOutput`].

use crate::brep::{BRepId, BRepModel};
use crate::math::Plane;
use crate::parametric::errors::ParametricError;
use crate::parametric::naming::{EntitySignature, SignatureResolver};
use crate::parametric::types::{ComponentId, NodeId};
use crate::sketch::{Sketch, SketchEntityId};
use crate::tessellation::Mesh;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Stable identifier for a datum plane entry inside a `FeatureOutput`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DatumPlaneId(pub Uuid);

impl DatumPlaneId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for DatumPlaneId {
    fn default() -> Self {
        Self::new()
    }
}

/// Stable identifier for a sketch entry inside a `FeatureOutput`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SketchInstanceId(pub Uuid);

impl SketchInstanceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SketchInstanceId {
    fn default() -> Self {
        Self::new()
    }
}

/// Reference to a stable world-space entity. Always resolves; never breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorldRef {
    Origin,
    AxisX,
    AxisY,
    AxisZ,
    PlaneXY,
    PlaneXZ,
    PlaneYZ,
}

/// An explicit reference to an entity in some other feature node's output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputRef {
    BRepEntity {
        producing_node: NodeId,
        signature: EntitySignature,
    },
    DatumPlane {
        producing_node: NodeId,
        plane_id: DatumPlaneId,
    },
    SketchEntity {
        producing_node: NodeId,
        sketch_id: SketchInstanceId,
        entity_id: SketchEntityId,
    },
    World(WorldRef),
}

impl InputRef {
    /// The node this reference points into, if any. `None` for `World(..)`.
    pub fn producing_node(&self) -> Option<NodeId> {
        match self {
            Self::BRepEntity { producing_node, .. } => Some(*producing_node),
            Self::DatumPlane { producing_node, .. } => Some(*producing_node),
            Self::SketchEntity { producing_node, .. } => Some(*producing_node),
            Self::World(_) => None,
        }
    }
}

/// A datum plane entry inside a feature output. Has a stable id so later
/// features can reference it via `InputRef::DatumPlane`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatumPlaneEntry {
    pub id: DatumPlaneId,
    pub name: String,
    pub plane: Plane,
}

/// A sketch entry inside a feature output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SketchEntry {
    pub id: SketchInstanceId,
    pub name: String,
    pub sketch: Sketch,
}

/// A produced B-Rep entity and its computed signature, stored at build time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProducedEntity {
    pub brep_id: BRepId,
    pub signature: EntitySignature,
}

/// Cumulative output of a feature node: everything the component contains
/// after the operation runs, not just the delta.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeatureOutput {
    pub brep: BRepModel,
    pub produced: Vec<ProducedEntity>,
    pub mesh: Option<Mesh>,
    pub datum_planes: Vec<DatumPlaneEntry>,
    pub sketches: Vec<SketchEntry>,
}

impl FeatureOutput {
    /// The empty root output. Used as the synthetic starting state.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Lookup a datum plane entry by id.
    pub fn find_datum_plane(&self, id: DatumPlaneId) -> Option<&DatumPlaneEntry> {
        self.datum_planes.iter().find(|d| d.id == id)
    }

    /// Lookup a sketch entry by id.
    pub fn find_sketch(&self, id: SketchInstanceId) -> Option<&SketchEntry> {
        self.sketches.iter().find(|s| s.id == id)
    }
}

/// Context passed into `Feature::build`. Contains the parent node's output
/// and any cross-node inputs the feature declared via `inputs()`.
pub struct BuildContext<'a> {
    pub parent: &'a FeatureOutput,
    pub references: HashMap<NodeId, &'a FeatureOutput>,
    pub resolve: &'a dyn SignatureResolver,
    pub this_node: NodeId,
    pub target_component: ComponentId,
}

/// The feature abstraction. Every variant of [`Operation`] carries a payload
/// that implements this trait.
pub trait Feature {
    /// Declare every upstream entity this feature reads.
    fn inputs(&self) -> Vec<InputRef>;

    /// Run the operation. Must be deterministic: same inputs → same output.
    fn build(&self, ctx: &BuildContext) -> Result<FeatureOutput, ParametricError>;
}

/// Every concrete feature operation type. Variants are added in later tasks
/// as each feature migrates into the framework.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operation {
    /// Sentinel for the root node. Never produces real output — its build
    /// simply re-emits the parent context, so chaining Noops is a no-op.
    #[doc(hidden)]
    Noop,
    /// Create a datum plane (world XY/XZ/YZ for spec #1; offset/3-point/midplane in spec #2).
    CreateDatumPlane(crate::parametric::ops::datum_plane::CreateDatumPlaneOp),
    /// Create a sketch on a host datum plane.
    CreateSketch(Box<crate::parametric::ops::sketch_op::CreateSketchOp>),
}

impl Feature for Operation {
    fn inputs(&self) -> Vec<InputRef> {
        match self {
            Self::Noop => vec![],
            Self::CreateDatumPlane(op) => op.inputs(),
            Self::CreateSketch(op) => op.as_ref().inputs(),
        }
    }

    fn build(&self, ctx: &BuildContext) -> Result<FeatureOutput, ParametricError> {
        match self {
            Self::Noop => Ok(ctx.parent.clone()),
            Self::CreateDatumPlane(op) => op.build(ctx),
            Self::CreateSketch(op) => op.as_ref().build(ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_operation_has_no_inputs() {
        let op = Operation::Noop;
        assert!(op.inputs().is_empty());
    }

    #[test]
    fn world_ref_is_stable() {
        let r = InputRef::World(WorldRef::PlaneXY);
        assert!(r.producing_node().is_none());
    }

    #[test]
    fn datum_plane_id_is_unique() {
        let a = DatumPlaneId::new();
        let b = DatumPlaneId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn feature_output_empty_has_no_entities() {
        let out = FeatureOutput::empty();
        assert!(out.produced.is_empty());
        assert!(out.datum_planes.is_empty());
        assert!(out.sketches.is_empty());
        assert!(out.mesh.is_none());
    }
}
