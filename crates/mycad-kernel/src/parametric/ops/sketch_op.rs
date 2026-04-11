//! `CreateSketchOp`: places a sketch on a host datum plane (or, later, a face).

use crate::parametric::errors::ParametricError;
use crate::parametric::feature::{
    BuildContext, Feature, FeatureOutput, InputRef, SketchEntry, SketchInstanceId,
};
use crate::sketch::Sketch;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSketchOp {
    pub id: SketchInstanceId,
    pub name: String,
    pub host: InputRef,
    pub sketch: Sketch,
}

impl CreateSketchOp {
    pub fn on_datum_plane(
        host: InputRef,
        sketch: Sketch,
        name: impl Into<String>,
    ) -> Self {
        Self {
            id: SketchInstanceId::new(),
            name: name.into(),
            host,
            sketch,
        }
    }
}

impl Feature for CreateSketchOp {
    fn inputs(&self) -> Vec<InputRef> {
        vec![self.host.clone()]
    }

    fn build(&self, ctx: &BuildContext) -> Result<FeatureOutput, ParametricError> {
        // Verify the host reference resolves. For World references this is
        // trivial; for DatumPlane references we look it up in the referenced node.
        match &self.host {
            InputRef::World(_) => {
                // Always valid.
            }
            InputRef::DatumPlane { producing_node, plane_id } => {
                let referenced = ctx.references.get(producing_node).ok_or_else(|| {
                    ParametricError::InputResolutionFailed {
                        node: ctx.this_node.0,
                        reason: format!(
                            "producing node {} not in references map",
                            producing_node.0
                        ),
                    }
                })?;
                if referenced.find_datum_plane(*plane_id).is_none() {
                    return Err(ParametricError::InputResolutionFailed {
                        node: ctx.this_node.0,
                        reason: format!("datum plane {} not found in producing node", plane_id.0),
                    });
                }
            }
            InputRef::BRepEntity { .. } | InputRef::SketchEntity { .. } => {
                return Err(ParametricError::InputResolutionFailed {
                    node: ctx.this_node.0,
                    reason: "sketch host must be a DatumPlane or World plane in Spec #1".into(),
                });
            }
        }

        let mut out = ctx.parent.clone();
        out.sketches.push(SketchEntry {
            id: self.id,
            name: self.name.clone(),
            sketch: self.sketch.clone(),
        });
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parametric::feature::{Operation, WorldRef};
    use crate::parametric::ops::datum_plane::CreateDatumPlaneOp;
    use crate::parametric::rebuild::rebuild;
    use crate::parametric::types::Document;
    use crate::sketch::Sketch;

    #[test]
    fn sketch_on_world_xy_builds_successfully() {
        let mut doc = Document::new();
        let _datum = doc
            .append_op(Operation::CreateDatumPlane(CreateDatumPlaneOp::world(
                WorldRef::PlaneXY,
                "XY",
            )))
            .unwrap();

        let sketch = Sketch::world_xy();
        let op = CreateSketchOp::on_datum_plane(
            InputRef::World(WorldRef::PlaneXY),
            sketch,
            "Sketch1",
        );
        let node_id = doc.append_op(Operation::CreateSketch(Box::new(op))).unwrap();

        rebuild(&mut doc).unwrap();

        let node = doc.node(node_id).unwrap();
        let out = node.cached_output.as_ref().unwrap();
        assert_eq!(out.sketches.len(), 1);
        assert_eq!(out.sketches[0].name, "Sketch1");
        // The datum plane from the previous node should still be in cumulative state.
        assert_eq!(out.datum_planes.len(), 1);
    }
}
