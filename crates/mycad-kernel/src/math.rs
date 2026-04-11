//! Mathematical primitives, coordinate systems, and geometric helpers.

use glam::{DMat4, DQuat, DVec2, DVec3};
use serde::{Deserialize, Serialize};

pub type Scalar = f64;
pub type Vec2 = DVec2;
pub type Vec3 = DVec3;
pub type Mat4 = DMat4;
pub type Point2 = DVec2;
pub type Point3 = DVec3;
pub type Quat = DQuat;

pub const EPSILON: Scalar = 1.0e-7;
pub const DEFAULT_ANGULAR_EPSILON: Scalar = 1.0e-7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis3 {
    X,
    Y,
    Z,
}

impl Axis3 {
    pub fn unit(self) -> Vec3 {
        match self {
            Self::X => Vec3::X,
            Self::Y => Vec3::Y,
            Self::Z => Vec3::Z,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CoordinateSystem {
    pub origin: Point3,
    pub x_axis: Vec3,
    pub y_axis: Vec3,
    pub z_axis: Vec3,
}

impl CoordinateSystem {
    pub fn new(origin: Point3, x_axis: Vec3, y_axis: Vec3, z_axis: Vec3) -> Self {
        Self {
            origin,
            x_axis: x_axis.normalize_or_zero(),
            y_axis: y_axis.normalize_or_zero(),
            z_axis: z_axis.normalize_or_zero(),
        }
    }

    pub fn world() -> Self {
        Self {
            origin: Point3::ZERO,
            x_axis: Vec3::X,
            y_axis: Vec3::Y,
            z_axis: Vec3::Z,
        }
    }

    pub fn to_world_point(&self, local: Point3) -> Point3 {
        self.origin + self.x_axis * local.x + self.y_axis * local.y + self.z_axis * local.z
    }

    pub fn to_world_vector(&self, local: Vec3) -> Vec3 {
        self.x_axis * local.x + self.y_axis * local.y + self.z_axis * local.z
    }

    pub fn to_local_point(&self, world: Point3) -> Point3 {
        let delta = world - self.origin;
        Point3::new(
            delta.dot(self.x_axis),
            delta.dot(self.y_axis),
            delta.dot(self.z_axis),
        )
    }

    pub fn to_local_vector(&self, world: Vec3) -> Vec3 {
        Vec3::new(
            world.dot(self.x_axis),
            world.dot(self.y_axis),
            world.dot(self.z_axis),
        )
    }

    pub fn transform_matrix(&self) -> Mat4 {
        Mat4::from_cols(
            self.x_axis.extend(0.0),
            self.y_axis.extend(0.0),
            self.z_axis.extend(0.0),
            self.origin.extend(1.0),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Plane {
    pub origin: Point3,
    pub normal: Vec3,
    pub u_axis: Vec3,
    pub v_axis: Vec3,
}

impl Plane {
    pub fn new(origin: Point3, normal: Vec3) -> Self {
        let normal = normal.normalize_or_zero();
        let helper = if normal.abs_diff_eq(Vec3::Z, EPSILON) || normal.abs_diff_eq(-Vec3::Z, EPSILON) {
            Vec3::X
        } else {
            Vec3::Z
        };
        let u_axis = normal.cross(helper).normalize_or_zero();
        let v_axis = normal.cross(u_axis).normalize_or_zero();

        Self {
            origin,
            normal,
            u_axis,
            v_axis,
        }
    }

    pub fn from_axes(origin: Point3, u_axis: Vec3, v_axis: Vec3) -> Self {
        let u_axis = u_axis.normalize_or_zero();
        let v_axis = v_axis.normalize_or_zero();
        let normal = u_axis.cross(v_axis).normalize_or_zero();
        Self {
            origin,
            normal,
            u_axis,
            v_axis,
        }
    }

    pub fn world_xy() -> Self {
        Self {
            origin: Point3::ZERO,
            normal: Vec3::Z,
            u_axis: Vec3::X,
            v_axis: Vec3::Y,
        }
    }

    pub fn coordinate_system(&self) -> CoordinateSystem {
        CoordinateSystem::new(self.origin, self.u_axis, self.v_axis, self.normal)
    }

    pub fn point_from_uv(&self, point: Point2) -> Point3 {
        self.origin + self.u_axis * point.x + self.v_axis * point.y
    }

    pub fn project_point(&self, point: Point3) -> Point2 {
        let delta = point - self.origin;
        Point2::new(delta.dot(self.u_axis), delta.dot(self.v_axis))
    }

    pub fn signed_distance_to_point(&self, point: Point3) -> Scalar {
        (point - self.origin).dot(self.normal)
    }

    pub fn distance_to_point(&self, point: Point3) -> Scalar {
        self.signed_distance_to_point(point).abs()
    }

    pub fn closest_point(&self, point: Point3) -> Point3 {
        point - self.normal * self.signed_distance_to_point(point)
    }
}

pub fn nearly_equal(a: Scalar, b: Scalar) -> bool {
    (a - b).abs() <= EPSILON
}

pub fn nearly_zero(value: Scalar) -> bool {
    value.abs() <= EPSILON
}

pub fn angle_between_vectors(a: Vec3, b: Vec3) -> Scalar {
    let a = a.normalize_or_zero();
    let b = b.normalize_or_zero();
    if a.length_squared() <= EPSILON || b.length_squared() <= EPSILON {
        return 0.0;
    }
    a.dot(b).clamp(-1.0, 1.0).acos()
}

pub fn distance(a: Point3, b: Point3) -> Scalar {
    a.distance(b)
}

pub fn distance_squared(a: Point3, b: Point3) -> Scalar {
    a.distance_squared(b)
}

pub fn project_point_to_plane(point: Point3, plane: &Plane) -> Point2 {
    plane.project_point(plane.closest_point(point))
}

pub fn point_on_plane_to_3d(point: Point2, plane: &Plane) -> Point3 {
    plane.point_from_uv(point)
}

pub fn distance_point_to_plane(point: Point3, plane: &Plane) -> Scalar {
    plane.distance_to_point(point)
}

pub fn translation_matrix(offset: Vec3) -> Mat4 {
    Mat4::from_translation(offset)
}

pub fn rotation_matrix(rotation: Quat) -> Mat4 {
    Mat4::from_quat(rotation)
}

pub fn scale_matrix(scale: Vec3) -> Mat4 {
    Mat4::from_scale(scale)
}

pub fn transform_point(transform: Mat4, point: Point3) -> Point3 {
    transform.transform_point3(point)
}

pub fn transform_vector(transform: Mat4, vector: Vec3) -> Vec3 {
    transform.transform_vector3(vector)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn plane_project_and_unproject_round_trip() {
        let plane = Plane::world_xy();
        let point_3d = Point3::new(2.0, -3.0, 4.0);

        let point_2d = project_point_to_plane(point_3d, &plane);
        let round_trip = point_on_plane_to_3d(point_2d, &plane);

        assert_relative_eq!(point_2d.x, 2.0, epsilon = EPSILON);
        assert_relative_eq!(point_2d.y, -3.0, epsilon = EPSILON);
        assert_relative_eq!(round_trip.x, 2.0, epsilon = EPSILON);
        assert_relative_eq!(round_trip.y, -3.0, epsilon = EPSILON);
        assert_relative_eq!(round_trip.z, 0.0, epsilon = EPSILON);
    }

    #[test]
    fn plane_distance_is_signed_correctly() {
        let plane = Plane::world_xy();
        let above = Point3::new(0.0, 0.0, 5.0);
        let below = Point3::new(0.0, 0.0, -2.5);

        assert_relative_eq!(plane.signed_distance_to_point(above), 5.0, epsilon = EPSILON);
        assert_relative_eq!(plane.signed_distance_to_point(below), -2.5, epsilon = EPSILON);
        assert_relative_eq!(distance_point_to_plane(below, &plane), 2.5, epsilon = EPSILON);
    }

    #[test]
    fn coordinate_system_world_local_round_trip() {
        let cs = CoordinateSystem::world();
        let point = Point3::new(1.0, 2.0, 3.0);
        let local = cs.to_local_point(point);
        let world = cs.to_world_point(local);

        assert_relative_eq!(world.x, point.x, epsilon = EPSILON);
        assert_relative_eq!(world.y, point.y, epsilon = EPSILON);
        assert_relative_eq!(world.z, point.z, epsilon = EPSILON);
    }

    #[test]
    fn angle_between_axes_is_ninety_degrees() {
        let angle = angle_between_vectors(Vec3::X, Vec3::Y);
        assert_relative_eq!(angle, std::f64::consts::FRAC_PI_2, epsilon = DEFAULT_ANGULAR_EPSILON);
    }

    #[test]
    fn transform_helpers_work() {
        let transform = translation_matrix(Vec3::new(1.0, 2.0, 3.0))
            * rotation_matrix(Quat::IDENTITY)
            * scale_matrix(Vec3::splat(2.0));

        let point = transform_point(transform, Point3::new(1.0, 1.0, 1.0));
        let vector = transform_vector(transform, Vec3::new(1.0, 0.0, 0.0));

        assert_relative_eq!(point.x, 3.0, epsilon = EPSILON);
        assert_relative_eq!(point.y, 4.0, epsilon = EPSILON);
        assert_relative_eq!(point.z, 5.0, epsilon = EPSILON);
        assert_relative_eq!(vector.x, 2.0, epsilon = EPSILON);
        assert_relative_eq!(vector.y, 0.0, epsilon = EPSILON);
        assert_relative_eq!(vector.z, 0.0, epsilon = EPSILON);
    }
}
