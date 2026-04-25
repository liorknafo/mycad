//! File format import/export: STEP, IGES, STL, OBJ, DXF, glTF.
//!
//! Currently only binary STL export is implemented.

use crate::tessellation::Mesh;

/// Write the mesh as binary STL (little-endian).
///
/// Layout: 80-byte header + u32 triangle count + per-triangle {3×f32 normal,
/// 3×(3×f32) vertices, u16 attribute}. Normals are computed per-triangle from
/// the vertex positions (right-hand rule) — the mesh's per-vertex normals
/// would not match STL's per-face convention.
pub fn mesh_to_stl_binary(mesh: &Mesh) -> Vec<u8> {
    let tri_count = mesh.indices.len();
    let mut buf = Vec::with_capacity(84 + tri_count * 50);

    buf.extend_from_slice(&[0u8; 80]);
    buf.extend_from_slice(&(tri_count as u32).to_le_bytes());

    for tri in &mesh.indices {
        let v0 = mesh.vertices[tri[0]];
        let v1 = mesh.vertices[tri[1]];
        let v2 = mesh.vertices[tri[2]];

        let ax = v1.x - v0.x;
        let ay = v1.y - v0.y;
        let az = v1.z - v0.z;
        let bx = v2.x - v0.x;
        let by = v2.y - v0.y;
        let bz = v2.z - v0.z;
        let nx = ay * bz - az * by;
        let ny = az * bx - ax * bz;
        let nz = ax * by - ay * bx;
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        let (nxf, nyf, nzf) = if len > 0.0 {
            ((nx / len) as f32, (ny / len) as f32, (nz / len) as f32)
        } else {
            (0.0, 0.0, 0.0)
        };

        buf.extend_from_slice(&nxf.to_le_bytes());
        buf.extend_from_slice(&nyf.to_le_bytes());
        buf.extend_from_slice(&nzf.to_le_bytes());
        for v in [v0, v1, v2] {
            buf.extend_from_slice(&(v.x as f32).to_le_bytes());
            buf.extend_from_slice(&(v.y as f32).to_le_bytes());
            buf.extend_from_slice(&(v.z as f32).to_le_bytes());
        }
        buf.extend_from_slice(&0u16.to_le_bytes());
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::{extrude, ExtrudeParams};
    use crate::math::{Plane, Point2};
    use crate::sketch::Sketch;
    use crate::tessellation::tessellate_solid;

    #[test]
    fn empty_mesh_writes_header_and_zero_count() {
        let mesh = Mesh::new();
        let bytes = mesh_to_stl_binary(&mesh);
        assert_eq!(bytes.len(), 84);
        assert_eq!(&bytes[80..84], &0u32.to_le_bytes());
    }

    #[test]
    fn cube_stl_has_12_triangles() {
        let mut sketch = Sketch::new(Plane::world_xy());
        sketch.add_rectangle(Point2::new(0.0, 0.0), Point2::new(1.0, 1.0));
        let solid = extrude(&sketch, ExtrudeParams::new(1.0)).unwrap();
        let mesh = tessellate_solid(&solid.model, solid.solid_id).unwrap();

        let bytes = mesh_to_stl_binary(&mesh);
        let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap());
        assert_eq!(count, 12);
        assert_eq!(bytes.len(), 84 + 12 * 50);
    }

    #[test]
    fn stl_triangle_normal_nonzero_for_axis_aligned_face() {
        let mesh = Mesh::from_data(
            vec![
                crate::math::Point3::new(0.0, 0.0, 0.0),
                crate::math::Point3::new(1.0, 0.0, 0.0),
                crate::math::Point3::new(0.0, 1.0, 0.0),
            ],
            vec![[0, 1, 2]],
        );
        let bytes = mesh_to_stl_binary(&mesh);
        let nz = f32::from_le_bytes(bytes[84 + 8..84 + 12].try_into().unwrap());
        assert!((nz - 1.0).abs() < 1e-6);
    }
}
