//! Boundary Representation: topology (vertex, edge, wire, face, shell, solid).

use crate::math::{Plane, Point3, Scalar, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Stable identifier for B-Rep topology elements.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BRepId(pub u64);

/// Curve geometry types for edges.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CurveGeometry {
    LineCurve { start: Point3, end: Point3 },
    CircleCurve {
        center: Point3,
        radius: Scalar,
        normal: Vec3,
        start_angle: Scalar,
        end_angle: Scalar,
    },
    ArcCurve {
        center: Point3,
        radius: Scalar,
        normal: Vec3,
        start_angle: Scalar,
        end_angle: Scalar,
    },
}

/// Surface geometry types for faces.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SurfaceGeometry {
    PlaneSurface { origin: Point3, normal: Vec3 },
    CylinderSurface {
        origin: Point3,
        radius: Scalar,
        axis: Vec3,
    },
}

impl SurfaceGeometry {
    /// Convert to a Plane if this is a planar surface.
    pub fn to_plane(&self) -> Option<Plane> {
        match self {
            SurfaceGeometry::PlaneSurface { origin, normal } => {
                Some(Plane::new(*origin, *normal))
            }
            SurfaceGeometry::CylinderSurface { .. } => None,
        }
    }
}

/// Vertex: 0-dimensional topology element (point in space).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vertex {
    pub id: BRepId,
    pub position: Point3,
}

/// Edge: 1-dimensional topology element connecting two vertices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub id: BRepId,
    pub vertices: [BRepId; 2],
    pub curve: CurveGeometry,
}

/// CoEdge: half-edge structure for traversing face loops.
/// Each edge has two CoEdges (one for each adjacent face), or one for boundary edges.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoEdge {
    pub id: BRepId,
    pub edge: BRepId,
    pub partner: Option<BRepId>,
    pub next: Option<BRepId>,
    pub prev: Option<BRepId>,
    pub face: Option<BRepId>,
    pub orientation: bool, // true if coedge direction matches edge direction
}

/// Wire: ordered sequence of coedges forming a loop (closed or open).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Wire {
    pub id: BRepId,
    pub coedges: Vec<BRepId>,
}

/// Face: 2-dimensional topology element bounded by wires.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Face {
    pub id: BRepId,
    pub surface: SurfaceGeometry,
    pub outer_wire: BRepId,
    pub inner_wires: Vec<BRepId>,
}

/// Shell: collection of faces forming a connected surface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shell {
    pub id: BRepId,
    pub faces: Vec<BRepId>,
}

/// Solid: 3-dimensional topology element bounded by shells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Solid {
    pub id: BRepId,
    pub shells: Vec<BRepId>,
}

/// Top-level container for all B-Rep topology.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BRepModel {
    pub vertices: HashMap<BRepId, Vertex>,
    pub edges: HashMap<BRepId, Edge>,
    pub coedges: HashMap<BRepId, CoEdge>,
    pub wires: HashMap<BRepId, Wire>,
    pub faces: HashMap<BRepId, Face>,
    pub shells: HashMap<BRepId, Shell>,
    pub solids: HashMap<BRepId, Solid>,
    pub next_id: u64,
}

/// Errors that can occur during B-Rep operations.
#[derive(Debug, Clone, PartialEq)]
pub enum BRepError {
    InvalidEdgeId(BRepId),
    DisconnectedEdges(BRepId, BRepId),
    EmptyEdgeList,
    WireNotFound(BRepId),
    InvalidCoEdgeId(BRepId),
    InvalidFaceId(BRepId),
    EmptyFaceList,
    ShellNotFound(BRepId),
    InvalidVertexId(BRepId),
}

impl std::fmt::Display for BRepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidEdgeId(id) => write!(f, "Invalid edge ID: {:?}", id),
            Self::DisconnectedEdges(a, b) => write!(f, "Edges {:?} and {:?} are not connected", a, b),
            Self::EmptyEdgeList => write!(f, "Cannot create wire from empty edge list"),
            Self::WireNotFound(id) => write!(f, "Wire not found: {:?}", id),
            Self::InvalidCoEdgeId(id) => write!(f, "Invalid coedge ID: {:?}", id),
            Self::InvalidFaceId(id) => write!(f, "Invalid face ID: {:?}", id),
            Self::EmptyFaceList => write!(f, "Cannot create shell from empty face list"),
            Self::ShellNotFound(id) => write!(f, "Shell not found: {:?}", id),
            Self::InvalidVertexId(id) => write!(f, "Invalid vertex ID: {:?}", id),
        }
    }
}

impl std::error::Error for BRepError {}

impl BRepModel {
    /// Create a new empty B-Rep model.
    pub fn new() -> Self {
        Self {
            vertices: HashMap::new(),
            edges: HashMap::new(),
            coedges: HashMap::new(),
            wires: HashMap::new(),
            faces: HashMap::new(),
            shells: HashMap::new(),
            solids: HashMap::new(),
            next_id: 1,
        }
    }

    /// Generate the next unique ID.
    pub fn next_id(&mut self) -> BRepId {
        let id = BRepId(self.next_id);
        self.next_id += 1;
        id
    }

    // Query methods - immutable
    pub fn vertex(&self, id: BRepId) -> Option<&Vertex> {
        self.vertices.get(&id)
    }

    pub fn edge(&self, id: BRepId) -> Option<&Edge> {
        self.edges.get(&id)
    }

    pub fn coedge(&self, id: BRepId) -> Option<&CoEdge> {
        self.coedges.get(&id)
    }

    pub fn wire(&self, id: BRepId) -> Option<&Wire> {
        self.wires.get(&id)
    }

    pub fn face(&self, id: BRepId) -> Option<&Face> {
        self.faces.get(&id)
    }

    pub fn shell(&self, id: BRepId) -> Option<&Shell> {
        self.shells.get(&id)
    }

    pub fn solid(&self, id: BRepId) -> Option<&Solid> {
        self.solids.get(&id)
    }

    // Query methods - mutable
    pub fn vertex_mut(&mut self, id: BRepId) -> Option<&mut Vertex> {
        self.vertices.get_mut(&id)
    }

    pub fn edge_mut(&mut self, id: BRepId) -> Option<&mut Edge> {
        self.edges.get_mut(&id)
    }

    pub fn coedge_mut(&mut self, id: BRepId) -> Option<&mut CoEdge> {
        self.coedges.get_mut(&id)
    }

    pub fn wire_mut(&mut self, id: BRepId) -> Option<&mut Wire> {
        self.wires.get_mut(&id)
    }

    pub fn face_mut(&mut self, id: BRepId) -> Option<&mut Face> {
        self.faces.get_mut(&id)
    }

    pub fn shell_mut(&mut self, id: BRepId) -> Option<&mut Shell> {
        self.shells.get_mut(&id)
    }

    pub fn solid_mut(&mut self, id: BRepId) -> Option<&mut Solid> {
        self.solids.get_mut(&id)
    }

    /// Create a wire from a sequence of edges.
    /// Validates that edges form a connected chain and creates CoEdges.
    pub fn create_wire_from_edges(&mut self, edge_ids: Vec<BRepId>) -> Result<BRepId, BRepError> {
        if edge_ids.is_empty() {
            return Err(BRepError::EmptyEdgeList);
        }

        // Validate all edges exist
        for edge_id in &edge_ids {
            if !self.edges.contains_key(edge_id) {
                return Err(BRepError::InvalidEdgeId(*edge_id));
            }
        }

        // If only one edge, it's trivially connected
        if edge_ids.len() == 1 {
            let wire_id = self.next_id();
            let coedge_id = self.next_id();

            let coedge = CoEdge {
                id: coedge_id,
                edge: edge_ids[0],
                partner: None,
                next: None,
                prev: None,
                face: None,
                orientation: true,
            };

            let wire = Wire {
                id: wire_id,
                coedges: vec![coedge_id],
            };

            self.coedges.insert(coedge_id, coedge);
            self.wires.insert(wire_id, wire);
            return Ok(wire_id);
        }

        // Validate edges form a connected chain
        for i in 0..edge_ids.len() - 1 {
            let edge_a = self.edges.get(&edge_ids[i]).unwrap();
            let edge_b = self.edges.get(&edge_ids[i + 1]).unwrap();

            let a_end = edge_a.vertices[1];
            let b_start = edge_b.vertices[0];
            let b_end = edge_b.vertices[1];

            // Check if end of edge_a connects to start or end of edge_b
            if a_end != b_start && a_end != b_end {
                return Err(BRepError::DisconnectedEdges(edge_ids[i], edge_ids[i + 1]));
            }
        }

        // Create CoEdges for the wire
        let wire_id = self.next_id();
        let mut coedge_ids = Vec::with_capacity(edge_ids.len());

        for (i, edge_id) in edge_ids.iter().enumerate() {
            let coedge_id = self.next_id();
            coedge_ids.push(coedge_id);

            let next = if i + 1 < edge_ids.len() {
                Some(BRepId(self.next_id))
            } else {
                None
            };

            let prev = if i > 0 {
                Some(BRepId(self.next_id - 2))
            } else {
                None
            };

            let coedge = CoEdge {
                id: coedge_id,
                edge: *edge_id,
                partner: None,
                next,
                prev,
                face: None,
                orientation: true,
            };

            self.coedges.insert(coedge_id, coedge);
        }

        // Fix up next/prev references (they were computed before IDs were assigned)
        for i in 0..coedge_ids.len() {
            let coedge_id = coedge_ids[i];
            let coedge = self.coedges.get_mut(&coedge_id).unwrap();

            coedge.next = if i + 1 < coedge_ids.len() {
                Some(coedge_ids[i + 1])
            } else {
                None
            };
            coedge.prev = if i > 0 {
                Some(coedge_ids[i - 1])
            } else {
                None
            };
        }

        let wire = Wire {
            id: wire_id,
            coedges: coedge_ids,
        };

        self.wires.insert(wire_id, wire);
        Ok(wire_id)
    }

    /// Validate that a wire is properly closed (last vertex matches first).
    pub fn validate_wire(&self, wire_id: BRepId) -> bool {
        let wire = match self.wires.get(&wire_id) {
            Some(w) => w,
            None => return false,
        };

        if wire.coedges.is_empty() {
            return false;
        }

        if wire.coedges.len() == 1 {
            // Single edge: check if it's a closed curve (start == end)
            let coedge = match self.coedges.get(&wire.coedges[0]) {
                Some(c) => c,
                None => return false,
            };
            let edge = match self.edges.get(&coedge.edge) {
                Some(e) => e,
                None => return false,
            };
            return edge.vertices[0] == edge.vertices[1];
        }

        // Get first coedge's start vertex
        let first_coedge = match self.coedges.get(&wire.coedges[0]) {
            Some(c) => c,
            None => return false,
        };
        let first_edge = match self.edges.get(&first_coedge.edge) {
            Some(e) => e,
            None => return false,
        };

        let start_vertex = if first_coedge.orientation {
            first_edge.vertices[0]
        } else {
            first_edge.vertices[1]
        };

        // Get last coedge's end vertex
        let last_coedge_id = wire.coedges.last().unwrap();
        let last_coedge = match self.coedges.get(last_coedge_id) {
            Some(c) => c,
            None => return false,
        };
        let last_edge = match self.edges.get(&last_coedge.edge) {
            Some(e) => e,
            None => return false,
        };

        let end_vertex = if last_coedge.orientation {
            last_edge.vertices[1]
        } else {
            last_edge.vertices[0]
        };

        start_vertex == end_vertex
    }

    /// Create a shell from a collection of face IDs.
    /// Validates that all faces exist before creating the shell.
    pub fn create_shell_from_faces(&mut self, face_ids: Vec<BRepId>) -> Result<BRepId, BRepError> {
        if face_ids.is_empty() {
            return Err(BRepError::EmptyFaceList);
        }

        // Validate all faces exist
        for face_id in &face_ids {
            if !self.faces.contains_key(face_id) {
                return Err(BRepError::InvalidFaceId(*face_id));
            }
        }

        let shell_id = self.next_id();
        let shell = Shell {
            id: shell_id,
            faces: face_ids,
        };

        self.shells.insert(shell_id, shell);
        Ok(shell_id)
    }

    /// Create a solid from a collection of shell IDs.
    /// Validates that all shells exist before creating the solid.
    pub fn create_solid_from_shells(&mut self, shell_ids: Vec<BRepId>) -> Result<BRepId, BRepError> {
        if shell_ids.is_empty() {
            return Err(BRepError::ShellNotFound(BRepId(0))); // Using ShellNotFound for empty case
        }

        // Validate all shells exist
        for shell_id in &shell_ids {
            if !self.shells.contains_key(shell_id) {
                return Err(BRepError::ShellNotFound(*shell_id));
            }
        }

        let solid_id = self.next_id();
        let solid = Solid {
            id: solid_id,
            shells: shell_ids,
        };

        self.solids.insert(solid_id, solid);
        Ok(solid_id)
    }

    /// Get vertex positions for a face by traversing its outer wire.
    pub fn face_vertices(&self, face_id: BRepId) -> Result<Vec<Point3>, BRepError> {
        let face = self.face(face_id).ok_or(BRepError::InvalidFaceId(face_id))?;
        let wire = self.wire(face.outer_wire).ok_or(BRepError::WireNotFound(face.outer_wire))?;

        let mut positions = Vec::new();
        for coedge_id in &wire.coedges {
            let coedge = self.coedge(*coedge_id).ok_or(BRepError::InvalidCoEdgeId(*coedge_id))?;
            let edge = self.edge(coedge.edge).ok_or(BRepError::InvalidEdgeId(coedge.edge))?;
            let vertex_id = if coedge.orientation {
                edge.vertices[0]
            } else {
                edge.vertices[1]
            };
            let vertex = self.vertex(vertex_id).ok_or(BRepError::InvalidVertexId(vertex_id))?;
            positions.push(vertex.position);
        }
        Ok(positions)
    }

    /// Get vertex IDs for a face by traversing its outer wire.
    pub fn face_vertex_ids(&self, face_id: BRepId) -> Result<Vec<BRepId>, BRepError> {
        let face = self.face(face_id).ok_or(BRepError::InvalidFaceId(face_id))?;
        let wire = self.wire(face.outer_wire).ok_or(BRepError::WireNotFound(face.outer_wire))?;

        let mut vertex_ids = Vec::new();
        for coedge_id in &wire.coedges {
            let coedge = self.coedge(*coedge_id).ok_or(BRepError::InvalidCoEdgeId(*coedge_id))?;
            let edge = self.edge(coedge.edge).ok_or(BRepError::InvalidEdgeId(coedge.edge))?;
            let vertex_id = if coedge.orientation {
                edge.vertices[0]
            } else {
                edge.vertices[1]
            };
            vertex_ids.push(vertex_id);
        }
        Ok(vertex_ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_vertex(model: &mut BRepModel, x: Scalar, y: Scalar, z: Scalar) -> BRepId {
        let id = model.next_id();
        let vertex = Vertex {
            id,
            position: Point3::new(x, y, z),
        };
        model.vertices.insert(id, vertex);
        id
    }

    fn create_test_edge(
        model: &mut BRepModel,
        v0: BRepId,
        v1: BRepId,
        start: Point3,
        end: Point3,
    ) -> BRepId {
        let id = model.next_id();
        let edge = Edge {
            id,
            vertices: [v0, v1],
            curve: CurveGeometry::LineCurve { start, end },
        };
        model.edges.insert(id, edge);
        id
    }

    #[test]
    fn test_brep_model_new() {
        let model = BRepModel::new();
        assert!(model.vertices.is_empty());
        assert!(model.edges.is_empty());
        assert_eq!(model.next_id, 1);
    }

    #[test]
    fn test_next_id_increments() {
        let mut model = BRepModel::new();
        let id1 = model.next_id();
        let id2 = model.next_id();
        let id3 = model.next_id();

        assert_eq!(id1.0, 1);
        assert_eq!(id2.0, 2);
        assert_eq!(id3.0, 3);
    }

    #[test]
    fn test_query_methods() {
        let mut model = BRepModel::new();
        let v_id = create_test_vertex(&mut model, 1.0, 2.0, 3.0);

        assert!(model.vertex(v_id).is_some());
        assert!(model.vertex(BRepId(999)).is_none());

        assert!(model.vertex_mut(v_id).is_some());
    }

    #[test]
    fn test_create_wire_from_edges_empty() {
        let mut model = BRepModel::new();
        let result = model.create_wire_from_edges(vec![]);
        assert!(matches!(result, Err(BRepError::EmptyEdgeList)));
    }

    #[test]
    fn test_create_wire_from_edges_invalid_id() {
        let mut model = BRepModel::new();
        let result = model.create_wire_from_edges(vec![BRepId(999)]);
        assert!(matches!(result, Err(BRepError::InvalidEdgeId(_))));
    }

    #[test]
    fn test_create_wire_from_single_edge() {
        let mut model = BRepModel::new();
        let v0 = create_test_vertex(&mut model, 0.0, 0.0, 0.0);
        let v1 = create_test_vertex(&mut model, 1.0, 0.0, 0.0);
        let e0 = create_test_edge(
            &mut model,
            v0,
            v1,
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
        );

        let wire_id = model.create_wire_from_edges(vec![e0]).unwrap();
        assert!(model.wire(wire_id).is_some());
        assert_eq!(model.wire(wire_id).unwrap().coedges.len(), 1);
    }

    #[test]
    fn test_create_wire_from_connected_edges() {
        let mut model = BRepModel::new();
        // Create a triangle
        let v0 = create_test_vertex(&mut model, 0.0, 0.0, 0.0);
        let v1 = create_test_vertex(&mut model, 1.0, 0.0, 0.0);
        let v2 = create_test_vertex(&mut model, 0.5, 1.0, 0.0);

        let e0 = create_test_edge(
            &mut model,
            v0,
            v1,
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
        );
        let e1 = create_test_edge(
            &mut model,
            v1,
            v2,
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.5, 1.0, 0.0),
        );
        let e2 = create_test_edge(
            &mut model,
            v2,
            v0,
            Point3::new(0.5, 1.0, 0.0),
            Point3::new(0.0, 0.0, 0.0),
        );

        let wire_id = model.create_wire_from_edges(vec![e0, e1, e2]).unwrap();
        let wire = model.wire(wire_id).unwrap();
        assert_eq!(wire.coedges.len(), 3);
    }

    #[test]
    fn test_validate_wire_closed() {
        let mut model = BRepModel::new();
        // Create a closed triangle
        let v0 = create_test_vertex(&mut model, 0.0, 0.0, 0.0);
        let v1 = create_test_vertex(&mut model, 1.0, 0.0, 0.0);
        let v2 = create_test_vertex(&mut model, 0.5, 1.0, 0.0);

        let e0 = create_test_edge(
            &mut model,
            v0,
            v1,
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
        );
        let e1 = create_test_edge(
            &mut model,
            v1,
            v2,
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.5, 1.0, 0.0),
        );
        let e2 = create_test_edge(
            &mut model,
            v2,
            v0,
            Point3::new(0.5, 1.0, 0.0),
            Point3::new(0.0, 0.0, 0.0),
        );

        let wire_id = model.create_wire_from_edges(vec![e0, e1, e2]).unwrap();
        assert!(model.validate_wire(wire_id));
    }

    #[test]
    fn test_validate_wire_open() {
        let mut model = BRepModel::new();
        // Create an open chain
        let v0 = create_test_vertex(&mut model, 0.0, 0.0, 0.0);
        let v1 = create_test_vertex(&mut model, 1.0, 0.0, 0.0);
        let v2 = create_test_vertex(&mut model, 2.0, 0.0, 0.0);

        let e0 = create_test_edge(
            &mut model,
            v0,
            v1,
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
        );
        let e1 = create_test_edge(
            &mut model,
            v1,
            v2,
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
        );

        let wire_id = model.create_wire_from_edges(vec![e0, e1]).unwrap();
        // Open wire should validate as not closed
        assert!(!model.validate_wire(wire_id));
    }
}
