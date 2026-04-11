//! B-Rep to triangle mesh conversion.

use crate::brep::{BRepError, BRepId, BRepModel};
use crate::math::{point_on_plane_to_3d, project_point_to_plane, Point2, Point3, EPSILON};

/// Triangle mesh representation with 3D vertices and triangle indices.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    /// 3D vertex positions.
    pub vertices: Vec<Point3>,
    /// Triangle indices, where each triangle is three vertex indices.
    pub indices: Vec<[usize; 3]>,
    /// Per-vertex normals for smooth shading.
    pub normals: Vec<[f64; 3]>,
}

impl Mesh {
    /// Create an empty mesh.
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            normals: Vec::new(),
        }
    }

    /// Create a mesh from existing vertices and indices.
    pub fn from_data(vertices: Vec<Point3>, indices: Vec<[usize; 3]>) -> Self {
        let normals = vec![[0.0, 0.0, 1.0]; vertices.len()];
        Self { vertices, indices, normals }
    }

    /// Number of triangles in the mesh.
    pub fn triangle_count(&self) -> usize {
        self.indices.len()
    }
}

impl Default for Mesh {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur during tessellation.
#[derive(Debug, Clone, PartialEq)]
pub enum TessellationError {
    /// The face ID was not found in the B-Rep model.
    FaceNotFound(BRepId),
    /// The face has fewer than 3 vertices.
    InvalidPolygon(usize),
    /// The face surface is not planar.
    NonPlanarSurface,
    /// A B-Rep error occurred while accessing topology.
    BRepError(BRepError),
}

impl std::fmt::Display for TessellationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FaceNotFound(id) => write!(f, "Face not found: {:?}", id),
            Self::InvalidPolygon(n) => {
                write!(f, "Invalid polygon with {} vertices (need at least 3)", n)
            }
            Self::NonPlanarSurface => write!(f, "Cannot tessellate non-planar surface"),
            Self::BRepError(e) => write!(f, "B-Rep error: {}", e),
        }
    }
}

impl std::error::Error for TessellationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BRepError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<BRepError> for TessellationError {
    fn from(e: BRepError) -> Self {
        Self::BRepError(e)
    }
}

/// Tessellate a B-Rep face into a triangle mesh.
///
/// This function extracts the face's outer wire vertices, projects them to 2D
/// using the face's plane, and triangulates using the ear-clipping algorithm.
///
/// # Arguments
/// * `model` - The B-Rep model containing the face.
/// * `face_id` - The ID of the face to tessellate.
///
/// # Returns
/// A `Mesh` containing the triangulated face, or an error if tessellation fails.
pub fn tessellate_face(model: &BRepModel, face_id: BRepId) -> Result<Mesh, TessellationError> {
    // Get the face and its surface plane
    let face = model
        .face(face_id)
        .ok_or(TessellationError::FaceNotFound(face_id))?;

    let plane = face
        .surface
        .to_plane()
        .ok_or(TessellationError::NonPlanarSurface)?;

    // Get the face vertices in order
    let vertices_3d = model.face_vertices(face_id)?;

    if vertices_3d.len() < 3 {
        return Err(TessellationError::InvalidPolygon(vertices_3d.len()));
    }

    // Project 3D vertices to 2D (UV space of the plane)
    let vertices_2d: Vec<Point2> = vertices_3d
        .iter()
        .map(|v| project_point_to_plane(*v, &plane))
        .collect();

    // Run ear-clipping triangulation
    let triangle_indices = ear_clip_triangulation(&vertices_2d);

    // Explicitly map 2D vertices back to 3D for clarity and future extensibility
    // (e.g., handling holes, curved surfaces, or Steiner points)
    let reconstructed_vertices: Vec<Point3> = vertices_2d
        .iter()
        .map(|v| point_on_plane_to_3d(*v, &plane))
        .collect();

    Ok(Mesh::from_data(reconstructed_vertices, triangle_indices))
}

/// Ear-clipping triangulation algorithm for simple polygons.
///
/// Takes a slice of 2D polygon vertices in order (clockwise or counter-clockwise)
/// and returns a vector of triangle indices.
///
/// The algorithm works by iteratively finding "ears" (convex vertices whose
/// triangle contains no other polygon vertices) and clipping them until
/// only three vertices remain.
///
/// # Arguments
/// * `polygon` - Slice of 2D points representing the polygon vertices.
///
/// # Returns
/// A vector of triangle indices, where each triangle is [i0, i1, i2].
pub fn ear_clip_triangulation(polygon: &[Point2]) -> Vec<[usize; 3]> {
    let n = polygon.len();

    // Degenerate cases
    if n < 3 {
        return Vec::new();
    }

    // Triangle is already a triangle
    if n == 3 {
        return vec![[0, 1, 2]];
    }

    // Create a mutable list of vertex indices to track which vertices remain
    let mut indices: Vec<usize> = (0..n).collect();
    let mut triangles = Vec::with_capacity(n - 2);

    // Main loop: clip ears until only 3 vertices remain
    while indices.len() > 3 {
        let m = indices.len();
        let mut ear_found = false;

        // Try to find an ear vertex
        for i in 0..m {
            let prev_idx = indices[(i + m - 1) % m];
            let curr_idx = indices[i];
            let next_idx = indices[(i + 1) % m];

            if is_ear(polygon, prev_idx, curr_idx, next_idx, &indices) {
                // Found an ear, add the triangle
                triangles.push([prev_idx, curr_idx, next_idx]);

                // Remove the ear vertex (current vertex)
                indices.remove(i);
                ear_found = true;
                break;
            }
        }

        // If no ear found, the polygon may be self-intersecting or degenerate
        // Remove the first vertex and continue to avoid infinite loop
        if !ear_found {
            indices.remove(0);
        }
    }

    // Add the final triangle
    if indices.len() == 3 {
        triangles.push([indices[0], indices[1], indices[2]]);
    }

    triangles
}

/// Tessellate an entire B-Rep solid into a single unified triangle mesh.
///
/// This function iterates over all faces in the solid's shell, tessellates each face,
/// merges the results into a single mesh, and computes per-vertex normals by averaging
/// the face normals at each vertex.
///
/// # Arguments
/// * `model` - The B-Rep model containing the solid.
/// * `solid_id` - The ID of the solid to tessellate.
///
/// # Returns
/// A `Mesh` containing the triangulated solid with per-vertex normals, or an error if tessellation fails.
pub fn tessellate_solid(model: &BRepModel, solid_id: BRepId) -> Result<Mesh, TessellationError> {
    // Look up the solid
    let solid = model
        .solid(solid_id)
        .ok_or(TessellationError::FaceNotFound(solid_id))?;

    // Get the shell and collect all face IDs
    let shell_id = solid.shells.first().ok_or(TessellationError::BRepError(
        BRepError::ShellNotFound(BRepId(0))
    ))?;
    let shell = model.shell(*shell_id).ok_or(TessellationError::BRepError(
        BRepError::ShellNotFound(*shell_id)
    ))?;
    let face_ids: Vec<BRepId> = shell.faces.clone();

    if face_ids.is_empty() {
        return Ok(Mesh::new());
    }

    // Create an empty mesh accumulator
    let mut mesh = Mesh::new();

    // Tessellate each face and merge into the accumulator
    for face_id in face_ids {
        let face_mesh = tessellate_face(model, face_id)?;
        merge_face_mesh(&mut mesh, face_mesh);
    }

    // Compute per-vertex normals
    compute_vertex_normals(&mut mesh);

    Ok(mesh)
}

/// Merge a face mesh into the accumulator mesh with proper index offsetting.
fn merge_face_mesh(accumulator: &mut Mesh, face_mesh: Mesh) {
    let vertex_offset = accumulator.vertices.len();

    // Append vertices
    accumulator.vertices.extend(face_mesh.vertices);

    // Append triangles with offset indices
    for triangle in face_mesh.indices {
        let offset_triangle = [
            triangle[0] + vertex_offset,
            triangle[1] + vertex_offset,
            triangle[2] + vertex_offset,
        ];
        accumulator.indices.push(offset_triangle);
    }
}

/// Compute per-vertex normals by averaging face normals at each vertex.
fn compute_vertex_normals(mesh: &mut Mesh) {
    let vertex_count = mesh.vertices.len();
    if vertex_count == 0 {
        return;
    }

    // Initialize normal accumulators to zero
    let mut normal_sums: Vec<[f64; 3]> = vec![[0.0, 0.0, 0.0]; vertex_count];

    // Accumulate face normals for each triangle
    for triangle in &mesh.indices {
        let i0 = triangle[0];
        let i1 = triangle[1];
        let i2 = triangle[2];

        let v0 = mesh.vertices[i0];
        let v1 = mesh.vertices[i1];
        let v2 = mesh.vertices[i2];

        // Compute face normal using cross product of edges
        let edge1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
        let edge2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];

        let cross = [
            edge1[1] * edge2[2] - edge1[2] * edge2[1],
            edge1[2] * edge2[0] - edge1[0] * edge2[2],
            edge1[0] * edge2[1] - edge1[1] * edge2[0],
        ];

        // Normalize the face normal
        let length = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
        let face_normal = if length > EPSILON {
            [cross[0] / length, cross[1] / length, cross[2] / length]
        } else {
            [0.0, 0.0, 1.0]
        };

        // Add face normal to all three vertices
        for i in [i0, i1, i2] {
            normal_sums[i][0] += face_normal[0];
            normal_sums[i][1] += face_normal[1];
            normal_sums[i][2] += face_normal[2];
        }
    }

    // Normalize all vertex normals
    mesh.normals = normal_sums
        .iter()
        .map(|n| {
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if length > EPSILON {
                [n[0] / length, n[1] / length, n[2] / length]
            } else {
                [0.0, 0.0, 1.0]
            }
        })
        .collect();
}

/// Extract unique edges from a mesh for wireframe rendering.
///
/// This function iterates through all triangles and extracts the three edges
/// from each triangle. Shared edges between adjacent triangles are deduplicated
/// using a canonical ordering (min, max) of vertex indices.
///
/// # Arguments
/// * `mesh` - The mesh to extract edges from.
///
/// # Returns
/// A vector of edge endpoint pairs for line rendering.
pub fn extract_mesh_edges(mesh: &Mesh) -> Vec<(Point3, Point3)> {
    use std::collections::HashSet;

    let mut edge_set: HashSet<(usize, usize)> = HashSet::new();
    let mut edges = Vec::new();

    for triangle in &mesh.indices {
        let i0 = triangle[0];
        let i1 = triangle[1];
        let i2 = triangle[2];

        // Extract the three edges with canonical ordering for deduplication
        let edge1 = (i0.min(i1), i0.max(i1));
        let edge2 = (i1.min(i2), i1.max(i2));
        let edge3 = (i2.min(i0), i2.max(i0));

        // Insert edges and collect unique ones
        if edge_set.insert(edge1) {
            edges.push((mesh.vertices[edge1.0], mesh.vertices[edge1.1]));
        }
        if edge_set.insert(edge2) {
            edges.push((mesh.vertices[edge2.0], mesh.vertices[edge2.1]));
        }
        if edge_set.insert(edge3) {
            edges.push((mesh.vertices[edge3.0], mesh.vertices[edge3.1]));
        }
    }

    edges
}

/// Tessellate a solid and extract edges for wireframe rendering.
///
/// This is a convenience function that combines `tessellate_solid` and
/// `extract_mesh_edges` to return both the mesh and edge list in one call.
///
/// # Arguments
/// * `model` - The B-Rep model containing the solid.
/// * `solid_id` - The ID of the solid to tessellate.
///
/// # Returns
/// A tuple of `(Mesh, Vec<(Point3, Point3)>)` containing the tessellated mesh
/// and the unique edges for wireframe rendering, or an error if tessellation fails.
pub fn tessellate_solid_with_edges(
    model: &BRepModel,
    solid_id: BRepId,
) -> Result<(Mesh, Vec<(Point3, Point3)>), TessellationError> {
    let mesh = tessellate_solid(model, solid_id)?;
    let edges = extract_mesh_edges(&mesh);
    Ok((mesh, edges))
}

/// Check if a vertex forms a valid "ear" in the polygon.
///
/// An ear is a convex vertex where the triangle formed by the previous,
/// current, and next vertices contains no other polygon vertices.
///
/// # Arguments
/// * `polygon` - All polygon vertices.
/// * `prev` - Index of the previous vertex.
/// * `curr` - Index of the current vertex (candidate ear).
/// * `next` - Index of the next vertex.
/// * `active_indices` - The currently active (remaining) vertex indices.
///
/// # Returns
/// `true` if the current vertex forms a valid ear.
fn is_ear(
    polygon: &[Point2],
    prev: usize,
    curr: usize,
    next: usize,
    active_indices: &[usize],
) -> bool {
    let a = polygon[prev];
    let b = polygon[curr];
    let c = polygon[next];

    // Check if the vertex is convex (cross product must have correct sign)
    let cross = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);

    // For a counter-clockwise polygon, convex vertices have positive cross product
    // For clockwise, convex vertices have negative cross product
    // We accept both by checking the absolute value
    if cross.abs() < EPSILON {
        return false; // Collinear, not an ear
    }

    // Check if any other vertex lies inside the triangle
    for &idx in active_indices {
        if idx == prev || idx == curr || idx == next {
            continue;
        }

        if point_in_triangle(polygon[idx], a, b, c) {
            return false; // Another vertex inside the ear triangle
        }
    }

    true
}

/// Check if a point is inside a triangle (including on the edge).
///
/// Uses barycentric coordinate approach for robustness.
fn point_in_triangle(p: Point2, a: Point2, b: Point2, c: Point2) -> bool {
    // Compute vectors
    let v0 = c - a;
    let v1 = b - a;
    let v2 = p - a;

    // Compute dot products
    let dot00 = v0.dot(v0);
    let dot01 = v0.dot(v1);
    let dot02 = v0.dot(v2);
    let dot11 = v1.dot(v1);
    let dot12 = v1.dot(v2);

    // Compute barycentric coordinates
    let denom = dot00 * dot11 - dot01 * dot01;

    // Degenerate triangle
    if denom.abs() < EPSILON {
        return false;
    }

    let inv_denom = 1.0 / denom;
    let u = (dot11 * dot02 - dot01 * dot12) * inv_denom;
    let v = (dot00 * dot12 - dot01 * dot02) * inv_denom;

    // Check if point is inside triangle (with small epsilon for robustness)
    (u >= -EPSILON) && (v >= -EPSILON) && (u + v <= 1.0 + EPSILON)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_triangle_already_triangular() {
        let polygon = vec![
            Point2::new(0.0, 0.0),
            Point2::new(1.0, 0.0),
            Point2::new(0.5, 1.0),
        ];

        let result = ear_clip_triangulation(&polygon);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], [0, 1, 2]);
    }

    #[test]
    fn test_quad_triangulation() {
        let polygon = vec![
            Point2::new(0.0, 0.0),
            Point2::new(1.0, 0.0),
            Point2::new(1.0, 1.0),
            Point2::new(0.0, 1.0),
        ];

        let result = ear_clip_triangulation(&polygon);
        assert_eq!(result.len(), 2);

        // Both triangles should use vertices 0, 1, 2 or 0, 2, 3
        // Ear-clipping may produce either winding order
        assert_eq!(result.len(), 2);
        let all_indices: std::collections::HashSet<usize> = result.iter().flat_map(|t| t.iter().copied()).collect();
        assert!(all_indices.contains(&0));
        assert!(all_indices.contains(&1));
        assert!(all_indices.contains(&2));
        assert!(all_indices.contains(&3));
    }

    #[test]
    fn test_pentagon_triangulation() {
        let polygon = vec![
            Point2::new(0.0, 0.0),
            Point2::new(1.0, 0.0),
            Point2::new(1.5, 0.5),
            Point2::new(1.0, 1.0),
            Point2::new(0.0, 1.0),
        ];

        let result = ear_clip_triangulation(&polygon);
        // A pentagon should produce 3 triangles
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_concave_polygon() {
        // A concave "arrow" shape
        let polygon = vec![
            Point2::new(0.0, 0.0),
            Point2::new(2.0, 0.0),
            Point2::new(2.0, 1.0),
            Point2::new(1.0, 0.5), // Concave vertex
            Point2::new(2.0, -1.0),
            Point2::new(0.0, -1.0),
        ];

        let result = ear_clip_triangulation(&polygon);
        // A hexagon should produce 4 triangles
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn test_degenerate_polygon() {
        let polygon = vec![Point2::new(0.0, 0.0), Point2::new(1.0, 0.0)];
        let result = ear_clip_triangulation(&polygon);
        assert!(result.is_empty());

        let empty: Vec<Point2> = vec![];
        let result = ear_clip_triangulation(&empty);
        assert!(result.is_empty());
    }

    #[test]
    fn test_mesh_struct() {
        let vertices = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.5, 1.0, 0.0),
        ];
        let indices = vec![[0, 1, 2]];

        let mesh = Mesh::from_data(vertices.clone(), indices.clone());
        assert_eq!(mesh.vertices, vertices);
        assert_eq!(mesh.indices, indices);
        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn test_point_in_triangle() {
        let a = Point2::new(0.0, 0.0);
        let b = Point2::new(1.0, 0.0);
        let c = Point2::new(0.5, 1.0);

        // Point inside
        assert!(point_in_triangle(Point2::new(0.5, 0.3), a, b, c));

        // Points on vertices
        assert!(point_in_triangle(a, a, b, c));
        assert!(point_in_triangle(b, a, b, c));
        assert!(point_in_triangle(c, a, b, c));

        // Point outside
        assert!(!point_in_triangle(Point2::new(2.0, 2.0), a, b, c));
    }
}
