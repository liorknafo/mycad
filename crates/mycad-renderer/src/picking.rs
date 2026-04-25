//! CPU ray-cast picking for mesh selection.
//!
//! GPU color-buffer picking (offscreen R32Uint, async readback) is the longer-
//! term plan — this module is the Phase-1 placeholder: iterate triangles,
//! Möller-Trumbore intersect, return the nearest hit.

use mycad_kernel::math::{Point3, Scalar, Vec3};
use mycad_kernel::tessellation::Mesh;

const RAY_EPSILON: Scalar = 1.0e-8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickHit {
    pub triangle_index: usize,
    pub distance: Scalar,
    pub point: Point3,
}

/// Intersect a ray against every triangle in `mesh`, return the nearest hit.
pub fn pick_triangle(mesh: &Mesh, origin: Point3, dir: Vec3) -> Option<PickHit> {
    let mut best: Option<PickHit> = None;

    for (i, tri) in mesh.indices.iter().enumerate() {
        let v0 = mesh.vertices[tri[0]];
        let v1 = mesh.vertices[tri[1]];
        let v2 = mesh.vertices[tri[2]];

        if let Some(t) = moller_trumbore(origin, dir, v0, v1, v2) {
            if t > 0.0 && best.is_none_or(|h| t < h.distance) {
                best = Some(PickHit {
                    triangle_index: i,
                    distance: t,
                    point: origin + dir * t,
                });
            }
        }
    }

    best
}

fn moller_trumbore(
    origin: Point3,
    dir: Vec3,
    v0: Point3,
    v1: Point3,
    v2: Point3,
) -> Option<Scalar> {
    let e1 = v1 - v0;
    let e2 = v2 - v0;
    let h = dir.cross(e2);
    let a = e1.dot(h);
    if a.abs() < RAY_EPSILON {
        return None;
    }
    let f = 1.0 / a;
    let s = origin - v0;
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = f * dir.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = f * e2.dot(q);
    if t > RAY_EPSILON {
        Some(t)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_axis_aligned_triangle() {
        let mesh = Mesh::from_data(
            vec![
                Point3::new(-1.0, -1.0, 2.0),
                Point3::new(1.0, -1.0, 2.0),
                Point3::new(0.0, 1.0, 2.0),
            ],
            vec![[0, 1, 2]],
        );
        let hit = pick_triangle(&mesh, Point3::ZERO, Vec3::new(0.0, 0.0, 1.0)).unwrap();
        assert_eq!(hit.triangle_index, 0);
        assert!((hit.distance - 2.0).abs() < 1e-6);
    }

    #[test]
    fn ray_parallel_to_triangle_misses() {
        let mesh = Mesh::from_data(
            vec![
                Point3::new(0.0, 0.0, 2.0),
                Point3::new(1.0, 0.0, 2.0),
                Point3::new(0.0, 1.0, 2.0),
            ],
            vec![[0, 1, 2]],
        );
        assert!(pick_triangle(&mesh, Point3::ZERO, Vec3::new(1.0, 0.0, 0.0)).is_none());
    }

    #[test]
    fn picks_nearest_of_two() {
        let mesh = Mesh::from_data(
            vec![
                Point3::new(-1.0, -1.0, 5.0),
                Point3::new(1.0, -1.0, 5.0),
                Point3::new(0.0, 1.0, 5.0),
                Point3::new(-1.0, -1.0, 2.0),
                Point3::new(1.0, -1.0, 2.0),
                Point3::new(0.0, 1.0, 2.0),
            ],
            vec![[0, 1, 2], [3, 4, 5]],
        );
        let hit = pick_triangle(&mesh, Point3::ZERO, Vec3::new(0.0, 0.0, 1.0)).unwrap();
        assert_eq!(hit.triangle_index, 1);
        assert!((hit.distance - 2.0).abs() < 1e-6);
    }
}
