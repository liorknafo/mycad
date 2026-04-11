//! `CreateDatumPlaneOp`: adds a datum plane to the cumulative feature output.

use crate::math::Plane;
use crate::parametric::errors::ParametricError;
use crate::parametric::feature::{
    BuildContext, DatumPlaneEntry, DatumPlaneId, Feature, FeatureOutput, InputRef, WorldRef,
};
use serde::{Deserialize, Serialize};

/// Construction methods for a datum plane. For Spec #1, only `World` (one of
/// the three world planes) is fully supported. Spec #2 adds `Offset`,
/// `ThreePoints`, `ThroughPointParallelTo`, and `Midplane`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DatumPlaneConstruction {
    World(WorldRef),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDatumPlaneOp {
    pub id: DatumPlaneId,
    pub name: String,
    pub construction: DatumPlaneConstruction,
}

impl CreateDatumPlaneOp {
    /// Convenience constructor for the three world planes.
    pub fn world(world_ref: WorldRef, name: impl Into<String>) -> Self {
        Self {
            id: DatumPlaneId::new(),
            name: name.into(),
            construction: DatumPlaneConstruction::World(world_ref),
        }
    }

    fn resolve_plane(&self) -> Result<Plane, ParametricError> {
        match self.construction {
            DatumPlaneConstruction::World(WorldRef::PlaneXY) => Ok(Plane::xy()),
            DatumPlaneConstruction::World(WorldRef::PlaneXZ) => Ok(Plane::xz()),
            DatumPlaneConstruction::World(WorldRef::PlaneYZ) => Ok(Plane::yz()),
            DatumPlaneConstruction::World(other) => Err(ParametricError::BuildFailed {
                node: uuid::Uuid::nil(),
                reason: format!("WorldRef::{:?} is not a plane", other),
            }),
        }
    }
}

impl Feature for CreateDatumPlaneOp {
    fn inputs(&self) -> Vec<InputRef> {
        match self.construction {
            DatumPlaneConstruction::World(world_ref) => vec![InputRef::World(world_ref)],
        }
    }

    fn build(&self, ctx: &BuildContext) -> Result<FeatureOutput, ParametricError> {
        let plane = self.resolve_plane()?;
        let mut out = ctx.parent.clone();
        out.datum_planes.push(DatumPlaneEntry {
            id: self.id,
            name: self.name.clone(),
            plane,
        });
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parametric::feature::Operation;
    use crate::parametric::rebuild::rebuild;
    use crate::parametric::types::Document;

    #[test]
    fn world_xy_builds_with_plane_in_output() {
        let mut doc = Document::new();
        let op = CreateDatumPlaneOp::world(WorldRef::PlaneXY, "XY");
        let node_id = doc
            .append_op(Operation::CreateDatumPlane(op.clone()))
            .unwrap();

        rebuild(&mut doc).unwrap();

        let node = doc.node(node_id).unwrap();
        let out = node.cached_output.as_ref().unwrap();
        assert_eq!(out.datum_planes.len(), 1);
        assert_eq!(out.datum_planes[0].name, "XY");
    }

    #[test]
    fn inputs_is_world_planexy() {
        let op = CreateDatumPlaneOp::world(WorldRef::PlaneXY, "XY");
        let inputs = op.inputs();
        assert_eq!(inputs.len(), 1);
        assert!(matches!(inputs[0], InputRef::World(WorldRef::PlaneXY)));
    }
}
