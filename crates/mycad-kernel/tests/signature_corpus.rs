//! Signature corpus: fixed cases that exercise the topological naming resolver.
//!
//! Each case builds a document, applies a known edit, runs a rebuild, and
//! asserts the downstream resolution outcome.

use mycad_kernel::math::Point2;
use mycad_kernel::parametric::feature::{InputRef, Operation, WorldRef};
use mycad_kernel::parametric::ops::datum_plane::CreateDatumPlaneOp;
use mycad_kernel::parametric::ops::extrude_op::{ExtrudeDirection, ExtrudeOp, ProfileRef};
use mycad_kernel::parametric::ops::sketch_op::CreateSketchOp;
use mycad_kernel::parametric::rebuild::{mark_dirty, rebuild};
use mycad_kernel::parametric::types::Document;
use mycad_kernel::sketch::{LineSegment, Sketch, SketchGeometry};

/// Build a baseline document with datum + sketch(rectangle) + extrude, rebuilt.
fn baseline_rect_extrude(depth: f64, dims: (f64, f64)) -> (Document, mycad_kernel::parametric::types::NodeId, mycad_kernel::parametric::types::NodeId) {
    let mut doc = Document::new();

    doc.append_op(Operation::CreateDatumPlane(CreateDatumPlaneOp::world(
        WorldRef::PlaneXY,
        "XY",
    )))
    .unwrap();

    let mut sketch = Sketch::world_xy();
    sketch.add_rectangle(Point2::new(0.0, 0.0), Point2::new(dims.0, dims.1));
    let sketch_op = CreateSketchOp::on_datum_plane(
        InputRef::World(WorldRef::PlaneXY),
        sketch,
        "Rect",
    );
    let sketch_node = doc.append_op(Operation::CreateSketch(Box::new(sketch_op))).unwrap();
    let sketch_instance_id = match &doc.node(sketch_node).unwrap().operation {
        Operation::CreateSketch(op) => op.id,
        _ => unreachable!(),
    };

    let extrude_node = doc
        .append_op(Operation::Extrude(ExtrudeOp {
            profile: ProfileRef {
                producing_node: sketch_node,
                sketch_id: sketch_instance_id,
            },
            depth,
            direction: ExtrudeDirection::Up,
        }))
        .unwrap();

    rebuild(&mut doc).unwrap();
    (doc, sketch_node, extrude_node)
}

/// Case 1: extrude_depth_change — changing the depth must keep the mesh non-empty
/// and all existing references valid.
#[test]
fn case_1_extrude_depth_change() {
    let (mut doc, _sketch_node, extrude_node) = baseline_rect_extrude(5.0, (10.0, 5.0));

    // Mutate the extrude depth.
    let node = doc.node_mut(extrude_node).unwrap();
    if let Operation::Extrude(op) = &mut node.operation {
        op.depth = 15.0;
    }
    mark_dirty(&mut doc, extrude_node).unwrap();
    rebuild(&mut doc).unwrap();

    // Result must have a mesh.
    let out = doc.node(extrude_node).unwrap().cached_output.as_ref().unwrap();
    assert!(out.mesh.is_some());
    assert!(!out.mesh.as_ref().unwrap().vertices.is_empty());
}

/// Case 2: sketch_rectangle_resize — changing the sketch's rectangle size must
/// re-extrude correctly.
#[test]
fn case_2_sketch_rectangle_resize() {
    let (mut doc, sketch_node, extrude_node) = baseline_rect_extrude(5.0, (10.0, 5.0));

    // Replace the sketch with a larger rectangle.
    {
        let node = doc.node_mut(sketch_node).unwrap();
        if let Operation::CreateSketch(op) = &mut node.operation {
            op.sketch = {
                let mut s = Sketch::world_xy();
                s.add_rectangle(Point2::new(0.0, 0.0), Point2::new(20.0, 10.0));
                s
            };
        }
    }
    mark_dirty(&mut doc, sketch_node).unwrap();
    rebuild(&mut doc).unwrap();

    // Both sketch and extrude must be re-solved.
    assert!(!doc.node(sketch_node).unwrap().dirty);
    assert!(!doc.node(extrude_node).unwrap().dirty);
    let out = doc.node(extrude_node).unwrap().cached_output.as_ref().unwrap();
    assert!(out.mesh.is_some());
}

/// Case 3: datum_plane_name_change — for spec #1 we only have world planes,
/// so this case verifies that editing the datum plane's name (the only editable
/// thing in spec #1) triggers a re-solve and the downstream remains valid.
/// The "offset" variant tests land in spec #2.
#[test]
fn case_3_datum_plane_name_change() {
    let (mut doc, _sketch_node, extrude_node) = baseline_rect_extrude(5.0, (10.0, 5.0));

    // Find the datum plane node (immediate child of the root).
    let datum_node_id = {
        let root = doc.root_node;
        doc.nodes
            .iter()
            .find(|(_, n)| n.parent == Some(root))
            .map(|(id, _)| *id)
            .unwrap()
    };
    // Rename the datum plane.
    {
        let node = doc.node_mut(datum_node_id).unwrap();
        if let Operation::CreateDatumPlane(op) = &mut node.operation {
            op.name = "XY-renamed".into();
        }
    }
    mark_dirty(&mut doc, datum_node_id).unwrap();
    rebuild(&mut doc).unwrap();

    let out = doc.node(extrude_node).unwrap().cached_output.as_ref().unwrap();
    assert_eq!(out.datum_planes.len(), 1);
    assert_eq!(out.datum_planes[0].name, "XY-renamed");
    assert!(out.mesh.is_some());
}

/// Case 4: sketch_dimension_change — changing a constraint value in the sketch
/// must propagate downstream correctly and maintain extrude validity.
#[test]
fn case_4_sketch_dimension_change() {
    let (mut doc, sketch_node, extrude_node) = baseline_rect_extrude(5.0, (10.0, 5.0));

    // Replace the sketch with one of different dimensions but still a valid rectangle.
    {
        let node = doc.node_mut(sketch_node).unwrap();
        if let Operation::CreateSketch(op) = &mut node.operation {
            op.sketch = {
                let mut s = Sketch::world_xy();
                // Different dimensions from the original
                s.add_rectangle(Point2::new(0.0, 0.0), Point2::new(15.0, 8.0));
                s
            };
        }
    }
    mark_dirty(&mut doc, sketch_node).unwrap();
    rebuild(&mut doc).unwrap();

    // The extrude must still succeed with the new sketch dimensions.
    let out = doc.node(extrude_node).unwrap().cached_output.as_ref().unwrap();
    assert!(out.mesh.is_some());
    assert!(!out.mesh.as_ref().unwrap().vertices.is_empty());
}

/// Case 5: extrude_profile_swap_hard_fails — replacing the sketch with one that has no
/// closed loop must cause the extrude to hard-fail on rebuild.
#[test]
fn case_5_extrude_profile_swap_hard_fails() {
    let (mut doc, sketch_node, extrude_node) = baseline_rect_extrude(5.0, (10.0, 5.0));

    // Replace the sketch with one that has only a single open line segment.
    {
        let node = doc.node_mut(sketch_node).unwrap();
        if let Operation::CreateSketch(op) = &mut node.operation {
            let mut s = Sketch::world_xy();
            s.add_entity(
                SketchGeometry::LineSegment(LineSegment {
                    start: Point2::new(0.0, 0.0),
                    end: Point2::new(1.0, 0.0),
                }),
                false,
                None,
            );
            op.sketch = s;
        }
    }
    mark_dirty(&mut doc, sketch_node).unwrap();

    let err = rebuild(&mut doc).unwrap_err();
    assert!(
        matches!(
            err,
            mycad_kernel::parametric::errors::ParametricError::BuildFailed { .. }
        ),
        "expected BuildFailed, got {:?}",
        err
    );
    // The extrude node must have an error set.
    let node = doc.node(extrude_node).unwrap();
    assert!(node.error.is_some());
}
