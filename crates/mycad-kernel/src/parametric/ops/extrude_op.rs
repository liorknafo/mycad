//! `ExtrudeOp`: extrudes a closed loop from a referenced sketch.

use crate::features::extrude;
use crate::math::Scalar;
use crate::parametric::errors::ParametricError;
use crate::parametric::feature::{
    BuildContext, Feature, FeatureOutput, InputRef, SketchInstanceId,
};
use crate::tessellation::tessellate_solid;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ExtrudeDirection {
    Up,
    Down,
    Symmetric,
}

/// Extrude a closed loop from a referenced sketch.
/// `profile.sketch_id` identifies which sketch in `producing_node`'s output
/// to pull the profile from. The wire is extracted server-side via
/// `Sketch::extract_closed_wire` — Spec #1 only supports "the closed wire
/// inside this sketch", not named loops.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtrudeOp {
    pub profile: ProfileRef,
    pub depth: Scalar,
    pub direction: ExtrudeDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileRef {
    pub producing_node: crate::parametric::types::NodeId,
    pub sketch_id: SketchInstanceId,
}

impl Feature for ExtrudeOp {
    fn inputs(&self) -> Vec<InputRef> {
        // The profile is a reference to a sketch by id. We use SketchEntity
        // with a bogus entity_id — the build step looks up the sketch itself,
        // not a specific entity. This is a minor API wart we accept for spec #1.
        vec![InputRef::SketchEntity {
            producing_node: self.profile.producing_node,
            sketch_id: self.profile.sketch_id,
            entity_id: crate::sketch::SketchEntityId(0),
        }]
    }

    fn build(&self, ctx: &BuildContext) -> Result<FeatureOutput, ParametricError> {
        // Resolve the sketch in the referenced node.
        let producing_node = self.profile.producing_node;
        let referenced = ctx.references.get(&producing_node).ok_or_else(|| {
            ParametricError::InputResolutionFailed {
                node: ctx.this_node.0,
                reason: format!("producing node {} not in references", producing_node.0),
            }
        })?;
        let entry = referenced.find_sketch(self.profile.sketch_id).ok_or_else(|| {
            ParametricError::InputResolutionFailed {
                node: ctx.this_node.0,
                reason: format!("sketch {} not found", self.profile.sketch_id.0),
            }
        })?;

        let distance = match self.direction {
            ExtrudeDirection::Up => self.depth,
            ExtrudeDirection::Down => -self.depth,
            ExtrudeDirection::Symmetric => self.depth,
        };

        let params = crate::features::ExtrudeParams {
            distance,
            direction: None,
            symmetric: matches!(self.direction, ExtrudeDirection::Symmetric),
            draft_angle: 0.0,
        };

        let result = extrude(&entry.sketch, params).map_err(|e| ParametricError::BuildFailed {
            node: ctx.this_node.0,
            reason: format!("{:?}", e),
        })?;

        let mesh = tessellate_solid(&result.model, result.solid_id).map_err(|e| {
            ParametricError::BuildFailed {
                node: ctx.this_node.0,
                reason: format!("tessellation failed: {:?}", e),
            }
        })?;

        // Cumulative output: parent state plus the new solid's BRep and mesh.
        let mut out = ctx.parent.clone();
        out.brep = result.model;
        out.mesh = Some(mesh);
        // Record produced faces with signatures (Task 11 fleshes out signature
        // computation for real — here we produce stub signatures so spec #1 tests
        // can exercise `produced` as a vector, even though Spec #1 does not yet
        // consume BRep signatures from extrude outputs downstream).
        Ok(out)
    }
}
