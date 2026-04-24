//! Parametric feature operations: extrude, revolve, boolean, fillet, chamfer.

use crate::brep::{BRepId, BRepModel, CoEdge, CurveGeometry, Edge, Face, Solid, SurfaceGeometry, Vertex, Wire};
use crate::math::{Point2, Scalar, Vec3};
use crate::sketch::{Sketch, SketchGeometry, WireExtractionError};

/// Errors that can occur during extrusion operations.
#[derive(Debug, Clone, PartialEq)]
pub enum ExtrudeError {
    /// Failed to extract a closed wire from the sketch.
    WireExtraction(WireExtractionError),
    /// No sketch entities found to extrude.
    EmptySketch,
    /// Invalid extrusion distance (must be non-zero).
    InvalidDistance,
    /// B-Rep topology error during solid creation.
    BRepTopology(String),
}

impl std::fmt::Display for ExtrudeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WireExtraction(e) => write!(f, "Wire extraction failed: {:?}", e),
            Self::EmptySketch => write!(f, "Sketch contains no entities"),
            Self::InvalidDistance => write!(f, "Extrusion distance must be non-zero"),
            Self::BRepTopology(msg) => write!(f, "B-Rep topology error: {}", msg),
        }
    }
}

impl std::error::Error for ExtrudeError {}

impl From<WireExtractionError> for ExtrudeError {
    fn from(e: WireExtractionError) -> Self {
        ExtrudeError::WireExtraction(e)
    }
}

/// Parameters for an extrude operation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExtrudeParams {
    /// Distance to extrude along the direction vector.
    pub distance: Scalar,
    /// Direction vector for extrusion (default is sketch plane normal).
    pub direction: Option<Vec3>,
    /// Whether to create a symmetric extrusion (both directions).
    pub symmetric: bool,
    /// Draft angle for tapered extrusion (in radians, 0 = no taper).
    pub draft_angle: Scalar,
}

impl ExtrudeParams {
    /// Create a simple extrusion with given distance along the sketch normal.
    pub fn new(distance: Scalar) -> Self {
        Self {
            distance,
            direction: None,
            symmetric: false,
            draft_angle: 0.0,
        }
    }

    /// Create an extrusion with a specific direction vector.
    pub fn with_direction(distance: Scalar, direction: Vec3) -> Self {
        Self {
            distance,
            direction: Some(direction),
            symmetric: false,
            draft_angle: 0.0,
        }
    }

    /// Set symmetric extrusion.
    pub fn symmetric(mut self, symmetric: bool) -> Self {
        self.symmetric = symmetric;
        self
    }

    /// Set draft angle for tapered extrusion.
    pub fn draft_angle(mut self, angle: Scalar) -> Self {
        self.draft_angle = angle;
        self
    }

    /// Get the effective direction vector (uses sketch normal if not specified).
    pub fn direction(&self, sketch_normal: Vec3) -> Vec3 {
        self.direction.unwrap_or(sketch_normal).normalize()
    }

    /// Get the total extrusion distance (accounts for symmetric extrusion).
    pub fn total_distance(&self) -> Scalar {
        if self.symmetric {
            self.distance * 2.0
        } else {
            self.distance
        }
    }
}

/// Result of an extrude operation containing the created B-Rep solid.
#[derive(Debug, Clone)]
pub struct ExtrudeResult {
    /// The B-Rep model containing the solid.
    pub model: BRepModel,
    /// ID of the created solid.
    pub solid_id: BRepId,
    /// ID of the bottom face (original sketch profile).
    pub bottom_face_id: BRepId,
    /// ID of the top face (extruded profile).
    pub top_face_id: BRepId,
    /// IDs of the side faces created from extruding edges.
    pub side_face_ids: Vec<BRepId>,
}

/// Extrude a closed wire from a sketch into a 3D solid.
///
/// This function:
/// 1. Extracts the closed wire from the sketch
/// 2. Creates the bottom face from the wire
/// 3. Creates the top face by translating the wire along the extrusion direction
/// 4. Creates side faces from each extruded edge
/// 5. Combines all faces into a solid
///
/// # Arguments
/// * `sketch` - The sketch containing the profile to extrude
/// * `params` - Extrusion parameters (distance, direction, etc.)
///
/// # Returns
/// * `Ok(ExtrudeResult)` - The created solid and related topology
/// * `Err(ExtrudeError)` - If extrusion fails
///
pub(crate) fn extrude(sketch: &Sketch, params: ExtrudeParams) -> Result<ExtrudeResult, ExtrudeError> {
    // Validate parameters
    if nearly_zero(params.distance) {
        return Err(ExtrudeError::InvalidDistance);
    }

    // Extract the closed wire from the sketch
    let wire_loop = sketch.extract_closed_wire()?;

    if wire_loop.edges.is_empty() {
        return Err(ExtrudeError::EmptySketch);
    }

    // Create a new B-Rep model
    let mut model = BRepModel::new();

    // Get the sketch plane and determine extrusion direction
    let sketch_normal = sketch.plane.normal;
    let direction = params.direction(sketch_normal);

    // Calculate offset for top face
    let offset = direction * params.distance;

    // Create vertices and edges for both bottom and top profiles
    let bottom_vertex_ids: Vec<BRepId> = wire_loop
        .edges
        .iter()
        .map(|&entity_id| {
            let entity = sketch.entity(entity_id).ok_or(ExtrudeError::EmptySketch)?;
            if let SketchGeometry::LineSegment(line) = &entity.geometry {
                // Create vertex at start point (in 3D world coordinates)
                let world_point = sketch.local_point_to_world(line.start);
                let vertex_id = model.next_id();
                let vertex = Vertex {
                    id: vertex_id,
                    position: world_point,
                };
                model.vertices.insert(vertex_id, vertex);
                Ok(vertex_id)
            } else {
                Err(ExtrudeError::BRepTopology("Only line segments supported".to_string()))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Create top vertices (offset from bottom)
    let top_vertex_ids: Vec<BRepId> = wire_loop
        .edges
        .iter()
        .map(|&entity_id| {
            let entity = sketch.entity(entity_id).ok_or(ExtrudeError::EmptySketch)?;
            if let SketchGeometry::LineSegment(line) = &entity.geometry {
                let world_point = sketch.local_point_to_world(line.start);
                let top_point = world_point + offset;
                let vertex_id = model.next_id();
                let vertex = Vertex {
                    id: vertex_id,
                    position: top_point,
                };
                model.vertices.insert(vertex_id, vertex);
                Ok(vertex_id)
            } else {
                Err(ExtrudeError::BRepTopology("Only line segments supported".to_string()))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Create bottom edges
    let n = wire_loop.edges.len();
    let mut bottom_edge_ids: Vec<BRepId> = Vec::with_capacity(n);
    let mut top_edge_ids: Vec<BRepId> = Vec::with_capacity(n);
    let mut side_edge_bottom_ids: Vec<BRepId> = Vec::with_capacity(n);
    let _side_edge_top_ids: Vec<BRepId> = Vec::with_capacity(n);

    for i in 0..n {
        // Bottom edge
        let entity = sketch
            .entity(wire_loop.edges[i])
            .ok_or(ExtrudeError::EmptySketch)?;
        if let SketchGeometry::LineSegment(line) = &entity.geometry {
            let start_3d = sketch.local_point_to_world(line.start);
            let end_3d = sketch.local_point_to_world(line.end);

            let edge_id = model.next_id();
            let edge = Edge {
                id: edge_id,
                vertices: [bottom_vertex_ids[i], bottom_vertex_ids[(i + 1) % n]],
                curve: CurveGeometry::LineCurve {
                    start: start_3d,
                    end: end_3d,
                },
            };
            model.edges.insert(edge_id, edge);
            bottom_edge_ids.push(edge_id);
        }

        // Top edge
        let entity = sketch
            .entity(wire_loop.edges[i])
            .ok_or(ExtrudeError::EmptySketch)?;
        if let SketchGeometry::LineSegment(line) = &entity.geometry {
            let start_3d = sketch.local_point_to_world(line.start);
            let end_3d = sketch.local_point_to_world(line.end);
            let top_start = start_3d + offset;
            let top_end = end_3d + offset;

            let edge_id = model.next_id();
            let edge = Edge {
                id: edge_id,
                vertices: [top_vertex_ids[i], top_vertex_ids[(i + 1) % n]],
                curve: CurveGeometry::LineCurve {
                    start: top_start,
                    end: top_end,
                },
            };
            model.edges.insert(edge_id, edge);
            top_edge_ids.push(edge_id);
        }

        // Vertical side edges connecting bottom to top
        let side_bottom_id = model.next_id();
        let side_bottom_edge = Edge {
            id: side_bottom_id,
            vertices: [bottom_vertex_ids[i], top_vertex_ids[i]],
            curve: CurveGeometry::LineCurve {
                start: sketch.local_point_to_world(
                    sketch
                        .entity(wire_loop.edges[i])
                        .and_then(|e| e.geometry.point(crate::sketch::EntityPointKind::Start))
                        .ok_or(ExtrudeError::EmptySketch)?,
                ),
                end: sketch.local_point_to_world(
                    sketch
                        .entity(wire_loop.edges[i])
                        .and_then(|e| e.geometry.point(crate::sketch::EntityPointKind::Start))
                        .ok_or(ExtrudeError::EmptySketch)?,
                ) + offset,
            },
        };
        model.edges.insert(side_bottom_id, side_bottom_edge);
        side_edge_bottom_ids.push(side_bottom_id);
    }

    // Create wires from edges
    let bottom_wire_id = model
        .create_wire_from_edges(bottom_edge_ids.clone())
        .map_err(|e| ExtrudeError::BRepTopology(format!("{:?}", e)))?;
    let top_wire_id = model
        .create_wire_from_edges(top_edge_ids.clone())
        .map_err(|e| ExtrudeError::BRepTopology(format!("{:?}", e)))?;

    // Validate wires are closed
    if !model.validate_wire(bottom_wire_id) {
        return Err(ExtrudeError::BRepTopology(
            "Bottom wire is not closed".to_string(),
        ));
    }
    if !model.validate_wire(top_wire_id) {
        return Err(ExtrudeError::BRepTopology(
            "Top wire is not closed".to_string(),
        ));
    }

    // Create bottom face (Step 2 of extrude operation)
    let bottom_face_id = create_bottom_face(&mut model, bottom_wire_id, sketch, sketch_normal)?;

    // Create top face (Step 3 of extrude operation)
    let top_face_id = create_top_face(&mut model, top_wire_id, sketch, sketch_normal, offset)?;

    // Create side faces
    let mut side_face_ids = Vec::with_capacity(n);
    for i in 0..n {
        let side_wire_id = model.next_id();

        // Create coedges for the side face wire
        // Side face has 4 edges: bottom edge, right vertical, top edge, left vertical
        let mut side_coedge_ids = Vec::with_capacity(4);

        // Bottom edge coedge
        let coedge_bottom = CoEdge {
            id: model.next_id(),
            edge: bottom_edge_ids[i],
            partner: None,
            next: None,
            prev: None,
            face: Some(side_wire_id),
            orientation: true,
        };
        let coedge_bottom_id = coedge_bottom.id;
        side_coedge_ids.push(coedge_bottom_id);
        model.coedges.insert(coedge_bottom_id, coedge_bottom);

        // Right vertical edge coedge
        let coedge_right = CoEdge {
            id: model.next_id(),
            edge: side_edge_bottom_ids[(i + 1) % n],
            partner: None,
            next: None,
            prev: None,
            face: Some(side_wire_id),
            orientation: true,
        };
        let coedge_right_id = coedge_right.id;
        side_coedge_ids.push(coedge_right_id);
        model.coedges.insert(coedge_right_id, coedge_right);

        // Top edge coedge (reversed direction)
        let coedge_top = CoEdge {
            id: model.next_id(),
            edge: top_edge_ids[i],
            partner: None,
            next: None,
            prev: None,
            face: Some(side_wire_id),
            orientation: false, // Reversed
        };
        let coedge_top_id = coedge_top.id;
        side_coedge_ids.push(coedge_top_id);
        model.coedges.insert(coedge_top_id, coedge_top);

        // Left vertical edge coedge (reversed direction)
        let coedge_left = CoEdge {
            id: model.next_id(),
            edge: side_edge_bottom_ids[i],
            partner: None,
            next: None,
            prev: None,
            face: Some(side_wire_id),
            orientation: false, // Reversed
        };
        let coedge_left_id = coedge_left.id;
        side_coedge_ids.push(coedge_left_id);
        model.coedges.insert(coedge_left_id, coedge_left);

        // Link coedges
        for j in 0..4 {
            let coedge_id = side_coedge_ids[j];
            let next_id = side_coedge_ids[(j + 1) % 4];
            let prev_id = side_coedge_ids[(j + 3) % 4];

            if let Some(coedge) = model.coedges.get_mut(&coedge_id) {
                coedge.next = Some(next_id);
                coedge.prev = Some(prev_id);
            }
        }

        let side_wire = Wire {
            id: side_wire_id,
            coedges: side_coedge_ids,
        };
        model.wires.insert(side_wire_id, side_wire);

        // Compute side face plane: normal = edge_direction × extrusion_direction
        // Origin = midpoint of bottom edge
        let bottom_edge = model.edge(bottom_edge_ids[i]).expect("bottom edge must exist");
        let bottom_start_vertex = model.vertex(bottom_edge.vertices[0]).expect("vertex must exist");
        let bottom_end_vertex = model.vertex(bottom_edge.vertices[1]).expect("vertex must exist");
        let edge_direction = (bottom_end_vertex.position - bottom_start_vertex.position).normalize();
        // Side face normal = edge_dir × extrude_dir (outward-pointing)
        let side_normal = edge_direction.cross(direction).normalize();
        let side_origin = bottom_start_vertex.position;

        // Create side face
        let side_face_id = model.next_id();
        let side_face = Face {
            id: side_face_id,
            surface: SurfaceGeometry::PlaneSurface {
                origin: side_origin,
                normal: side_normal,
            },
            outer_wire: side_wire_id,
            inner_wires: vec![],
        };
        model.faces.insert(side_face_id, side_face);
        side_face_ids.push(side_face_id);
    }

    // Create shell containing all faces
    let shell_id = model.next_id();
    let mut all_face_ids = vec![bottom_face_id, top_face_id];
    all_face_ids.extend(&side_face_ids);
    let shell = crate::brep::Shell {
        id: shell_id,
        faces: all_face_ids,
    };
    model.shells.insert(shell_id, shell);

    // Create solid from shell
    let solid_id = model.next_id();
    let solid = Solid {
        id: solid_id,
        shells: vec![shell_id],
    };
    model.solids.insert(solid_id, solid);

    Ok(ExtrudeResult {
        model,
        solid_id,
        bottom_face_id,
        top_face_id,
        side_face_ids,
    })
}

/// Step 2: Create the bottom face from the bottom wire.
///
/// This function creates a planar face using the sketch's plane geometry,
/// with the normal pointing in the negative sketch normal direction (downward).
///
/// # Arguments
/// * `model` - The B-Rep model to insert the face into
/// * `wire_id` - ID of the closed wire forming the outer boundary
/// * `sketch` - The sketch containing the plane geometry
/// * `sketch_normal` - The normal vector of the sketch plane
///
/// # Returns
/// * `Ok(BRepId)` - The ID of the created face
/// * `Err(ExtrudeError)` - If face creation fails
fn create_bottom_face(
    model: &mut BRepModel,
    wire_id: BRepId,
    sketch: &Sketch,
    sketch_normal: Vec3,
) -> Result<BRepId, ExtrudeError> {
    let face_id = model.next_id();
    let face = Face {
        id: face_id,
        surface: SurfaceGeometry::PlaneSurface {
            origin: sketch.local_point_to_world(Point2::new(0.0, 0.0)),
            normal: -sketch_normal, // Bottom face points downward
        },
        outer_wire: wire_id,
        inner_wires: vec![],
    };
    model.faces.insert(face_id, face);
    Ok(face_id)
}

/// Step 3: Create the top face from the top wire.
///
/// This function creates a planar face at the extruded position, with the normal
/// pointing in the positive sketch normal direction (upward). The top face is
/// the extruded profile translated along the direction vector by the offset amount.
///
/// # Arguments
/// * `model` - The B-Rep model to insert the face into
/// * `wire_id` - ID of the closed wire forming the outer boundary
/// * `sketch` - The sketch containing the plane geometry
/// * `sketch_normal` - The normal vector of the sketch plane
/// * `offset` - The offset vector from the sketch plane to the top face position
///
/// # Returns
/// * `Ok(BRepId)` - The ID of the created face
/// * `Err(ExtrudeError)` - If face creation fails
fn create_top_face(
    model: &mut BRepModel,
    wire_id: BRepId,
    sketch: &Sketch,
    sketch_normal: Vec3,
    offset: Vec3,
) -> Result<BRepId, ExtrudeError> {
    let face_id = model.next_id();
    let face = Face {
        id: face_id,
        surface: SurfaceGeometry::PlaneSurface {
            origin: sketch.local_point_to_world(Point2::new(0.0, 0.0)) + offset,
            normal: sketch_normal, // Top face points upward
        },
        outer_wire: wire_id,
        inner_wires: vec![],
    };
    model.faces.insert(face_id, face);
    Ok(face_id)
}

/// Check if a scalar value is nearly zero.
fn nearly_zero(value: Scalar) -> bool {
    value.abs() < 1.0e-10
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{Plane, Point2};
    use crate::tessellation::tessellate_solid;

    #[test]
    fn test_extrude_params_new() {
        let params = ExtrudeParams::new(10.0);
        assert_eq!(params.distance, 10.0);
        assert!(params.direction.is_none());
        assert!(!params.symmetric);
        assert_eq!(params.draft_angle, 0.0);
    }

    #[test]
    fn test_extrude_params_with_direction() {
        let direction = Vec3::new(0.0, 0.0, 1.0);
        let params = ExtrudeParams::with_direction(5.0, direction);
        assert_eq!(params.distance, 5.0);
        assert_eq!(params.direction, Some(direction));
    }

    #[test]
    fn test_extrude_params_builder_methods() {
        let params = ExtrudeParams::new(10.0)
            .symmetric(true)
            .draft_angle(0.1);
        assert!(params.symmetric);
        assert_eq!(params.draft_angle, 0.1);
    }

    #[test]
    fn test_extrude_params_total_distance() {
        let params = ExtrudeParams::new(5.0);
        assert_eq!(params.total_distance(), 5.0);

        let symmetric_params = ExtrudeParams::new(5.0).symmetric(true);
        assert_eq!(symmetric_params.total_distance(), 10.0);
    }

    #[test]
    fn test_extrude_rectangle_creates_solid() {
        let mut sketch = Sketch::new(Plane::world_xy());
        sketch.add_rectangle(Point2::new(0.0, 0.0), Point2::new(10.0, 5.0));

        let params = ExtrudeParams::new(3.0);
        let result = extrude(&sketch, params).expect("extrude should succeed");

        // Verify the result contains a solid
        assert!(result.model.solid(result.solid_id).is_some());

        // Verify bottom and top faces exist
        assert!(result.model.face(result.bottom_face_id).is_some());
        assert!(result.model.face(result.top_face_id).is_some());

        // Should have 4 side faces for a rectangle
        assert_eq!(result.side_face_ids.len(), 4);
        for side_id in &result.side_face_ids {
            assert!(result.model.face(*side_id).is_some());
        }
    }

    #[test]
    fn test_extrude_invalid_distance_fails() {
        let mut sketch = Sketch::new(Plane::world_xy());
        sketch.add_rectangle(Point2::new(0.0, 0.0), Point2::new(10.0, 5.0));

        let params = ExtrudeParams::new(0.0);
        let result = extrude(&sketch, params);

        assert!(matches!(result, Err(ExtrudeError::InvalidDistance)));
    }

    #[test]
    fn test_extrude_empty_sketch_fails() {
        let sketch = Sketch::new(Plane::world_xy());

        let params = ExtrudeParams::new(5.0);
        let result = extrude(&sketch, params);

        // Should fail because there's no closed loop to extract
        assert!(result.is_err());
    }

    #[test]
    fn test_extrude_result_contains_valid_brep() {
        let mut sketch = Sketch::new(Plane::world_xy());
        sketch.add_rectangle(Point2::new(0.0, 0.0), Point2::new(4.0, 2.0));

        let params = ExtrudeParams::new(1.0);
        let result = extrude(&sketch, params).expect("extrude should succeed");

        // Verify the model has the expected topology
        let solid = result.model.solid(result.solid_id).expect("solid should exist");
        assert!(!solid.shells.is_empty());

        let shell = result.model.shell(solid.shells[0]).expect("shell should exist");
        // 6 faces: bottom, top, and 4 sides
        assert_eq!(shell.faces.len(), 6);
    }

    #[test]
    fn test_full_pipeline_rectangle_extrude_tessellate() {
        // This test mimics what the app does:
        // 1. Create sketch on XY plane
        // 2. Add rectangle (with coincident + H/V constraints)
        // 3. Extract closed wire
        // 4. Extrude
        // 5. Tessellate

        let mut sketch = Sketch::world_xy();

        // Step 1: Add rectangle (same as Sketch::add_rectangle)
        let ids = sketch.add_rectangle(Point2::new(0.0, 0.0), Point2::new(5.0, 3.0));
        eprintln!("Rectangle entities: {:?}", ids);

        // Verify all line segments are present
        let line_count = sketch.entities.iter()
            .filter(|e| matches!(&e.geometry, SketchGeometry::LineSegment(_)))
            .count();
        eprintln!("Line count: {}", line_count);
        assert_eq!(line_count, 4, "Should have 4 line segments");

        // Print line endpoints after solve
        for entity in sketch.iter() {
            if let SketchGeometry::LineSegment(ref line) = entity.geometry {
                eprintln!("  Line {}: ({:.10},{:.10}) -> ({:.10},{:.10})",
                    entity.id.0, line.start.x, line.start.y, line.end.x, line.end.y);
            }
        }

        // Step 2: Extract closed wire
        let wire = sketch.extract_closed_wire()
            .expect("Wire extraction should succeed for a rectangle");
        eprintln!("Wire has {} edges", wire.edges.len());
        assert_eq!(wire.edges.len(), 4, "Wire should have 4 edges");

        // Step 3: Extrude
        let params = ExtrudeParams::new(5.0);
        let result = extrude(&sketch, params)
            .expect("Extrude should succeed");
        eprintln!("Extrude succeeded: {} vertices, {} edges, {} faces",
            result.model.vertices.len(),
            result.model.edges.len(),
            result.model.faces.len());

        // Verify basic topology
        assert_eq!(result.model.vertices.len(), 8, "Box should have 8 vertices");
        assert_eq!(result.model.edges.len(), 12, "Box should have 12 edges");
        assert_eq!(result.model.faces.len(), 6, "Box should have 6 faces");

        // Step 4: Tessellate
        let mesh = tessellate_solid(&result.model, result.solid_id)
            .expect("Tessellation should succeed");
        eprintln!("Tessellation: {} vertices, {} triangles",
            mesh.vertices.len(), mesh.indices.len());

        // A box should have 12 triangles (2 per face * 6 faces)
        assert_eq!(mesh.indices.len(), 12, "Box should have 12 triangles");
        // Per-face tessellation produces 4 vertices per face × 6 faces = 24
        assert!(mesh.vertices.len() >= 8, "Box should have at least 8 unique vertices");
    }
}
