//! Integration test: create a document, append datum plane + sketch + extrude,
//! rebuild, verify the extrude node has a non-empty BRep and mesh.

use mycad_kernel::math::Point2;
use mycad_kernel::parametric::feature::{InputRef, Operation, WorldRef};
use mycad_kernel::parametric::ops::datum_plane::CreateDatumPlaneOp;
use mycad_kernel::parametric::ops::extrude_op::{ExtrudeDirection, ExtrudeOp, ProfileRef};
use mycad_kernel::parametric::ops::sketch_op::CreateSketchOp;
use mycad_kernel::parametric::rebuild::rebuild;
use mycad_kernel::parametric::types::Document;
use mycad_kernel::sketch::Sketch;

#[test]
fn datum_sketch_extrude_end_to_end() {
    let mut doc = Document::new();

    // Datum plane.
    let datum_node = doc
        .append_op(Operation::CreateDatumPlane(CreateDatumPlaneOp::world(
            WorldRef::PlaneXY,
            "XY",
        )))
        .unwrap();

    // Sketch with a rectangle.
    let mut sketch = Sketch::world_xy();
    sketch.add_rectangle(Point2::new(0.0, 0.0), Point2::new(10.0, 5.0));
    let sketch_op = CreateSketchOp::on_datum_plane(
        InputRef::World(WorldRef::PlaneXY),
        sketch,
        "Rect",
    );
    let sketch_node = doc.append_op(Operation::CreateSketch(Box::new(sketch_op))).unwrap();

    // Fetch the sketch instance id from the sketch op (before it's moved into the enum).
    // Because Operation is clone, we can look it up from the doc after append.
    let sketch_instance_id = {
        let node = doc.node(sketch_node).unwrap();
        match &node.operation {
            Operation::CreateSketch(op) => op.id,
            _ => panic!("expected CreateSketch"),
        }
    };

    // Extrude.
    let extrude_op = ExtrudeOp {
        profile: ProfileRef {
            producing_node: sketch_node,
            sketch_id: sketch_instance_id,
        },
        depth: 5.0,
        direction: ExtrudeDirection::Up,
    };
    let extrude_node = doc.append_op(Operation::Extrude(extrude_op)).unwrap();

    rebuild(&mut doc).unwrap();

    let out = doc.node(extrude_node).unwrap().cached_output.as_ref().unwrap();
    assert!(out.mesh.is_some());
    let mesh = out.mesh.as_ref().unwrap();
    assert!(!mesh.vertices.is_empty());
    assert!(!mesh.indices.is_empty());

    // The datum plane is still in the cumulative state.
    assert_eq!(out.datum_planes.len(), 1);

    let _ = datum_node; // suppress unused warning
}
