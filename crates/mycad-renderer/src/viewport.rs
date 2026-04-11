use mycad_kernel::math::{Mat4, Point3, Scalar, Vec3};

pub const MIN_DISTANCE: Scalar = 0.05;
pub const MAX_DISTANCE: Scalar = 10_000.0;
pub const DEFAULT_FOV_Y_RADIANS: Scalar = 45.0_f64.to_radians();
pub const DEFAULT_NEAR: Scalar = 0.01;
pub const DEFAULT_FAR: Scalar = 1000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionMode {
    Perspective,
    Orthographic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardView {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    Isometric,
}

#[derive(Debug, Clone)]
pub struct ArcballCamera {
    pub target: Point3,
    pub distance: Scalar,
    pub yaw: Scalar,
    pub pitch: Scalar,
    pub projection: ProjectionMode,
    pub fov_y_radians: Scalar,
    pub near: Scalar,
    pub far: Scalar,
    pub ortho_scale: Scalar,
}

impl Default for ArcballCamera {
    fn default() -> Self {
        let mut camera = Self {
            target: Point3::ZERO,
            distance: 8.0,
            yaw: -35.0_f64.to_radians(),
            pitch: 25.0_f64.to_radians(),
            projection: ProjectionMode::Perspective,
            fov_y_radians: DEFAULT_FOV_Y_RADIANS,
            near: DEFAULT_NEAR,
            far: DEFAULT_FAR,
            ortho_scale: 4.0,
        };
        camera.clamp();
        camera
    }
}

impl ArcballCamera {
    pub fn position(&self) -> Point3 {
        self.target - self.forward() * self.distance
    }

    pub fn forward(&self) -> Vec3 {
        let (yaw_sin, yaw_cos) = self.yaw.sin_cos();
        let (pitch_sin, pitch_cos) = self.pitch.sin_cos();
        Vec3::new(yaw_cos * pitch_cos, yaw_sin * pitch_cos, pitch_sin).normalize()
    }

    pub fn right(&self) -> Vec3 {
        let right = self.forward().cross(Vec3::Z);
        if right.length_squared() < 1e-12 {
            Vec3::X
        } else {
            right.normalize()
        }
    }

    pub fn up(&self) -> Vec3 {
        self.right().cross(self.forward()).normalize_or_zero()
    }

    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_at_rh(self.position(), self.target, self.up())
    }

    pub fn projection_matrix(&self, aspect: Scalar) -> Mat4 {
        let aspect = aspect.max(0.0001);
        match self.projection {
            ProjectionMode::Perspective => Mat4::perspective_rh(self.fov_y_radians, aspect, self.near, self.far),
            ProjectionMode::Orthographic => {
                let half_h = self.ortho_scale.max(MIN_DISTANCE);
                let half_w = half_h * aspect;
                Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, -self.far, self.far)
            }
        }
    }

    pub fn view_projection_matrix(&self, aspect: Scalar) -> Mat4 {
        self.projection_matrix(aspect) * self.view_matrix()
    }

    pub fn orbit(&mut self, dx: Scalar, dy: Scalar) {
        let sensitivity = 0.01;
        self.yaw += dx * sensitivity;
        self.pitch += -dy * sensitivity;
        self.clamp();
    }

    pub fn pan(&mut self, dx: Scalar, dy: Scalar) {
        let scale = self.distance.max(1.0) * 0.0015;
        self.target -= self.right() * dx * scale;
        self.target += self.up() * dy * scale;
    }

    pub fn zoom(&mut self, delta: Scalar) {
        let zoom_factor = (1.0 - delta * 0.001).clamp(0.1, 10.0);
        self.distance = (self.distance * zoom_factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
        self.ortho_scale = (self.ortho_scale * zoom_factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    pub fn fit(&mut self, target: Point3, radius: Scalar) {
        self.target = target;
        let radius = radius.max(0.5);
        self.distance = (radius * 3.0).clamp(MIN_DISTANCE, MAX_DISTANCE);
        self.ortho_scale = radius * 1.5;
        self.clamp();
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn set_standard_view(&mut self, view: StandardView) {
        match view {
            StandardView::Front => {
                self.yaw = 90.0_f64.to_radians();
                self.pitch = 0.0;
            }
            StandardView::Back => {
                self.yaw = -90.0_f64.to_radians();
                self.pitch = 0.0;
            }
            StandardView::Left => {
                self.yaw = 180.0_f64.to_radians();
                self.pitch = 0.0;
            }
            StandardView::Right => {
                self.yaw = 0.0;
                self.pitch = 0.0;
            }
            StandardView::Top => {
                self.yaw = 0.0;
                self.pitch = 89.0_f64.to_radians();
            }
            StandardView::Bottom => {
                self.yaw = 0.0;
                self.pitch = -89.0_f64.to_radians();
            }
            StandardView::Isometric => {
                self.yaw = 45.0_f64.to_radians();
                self.pitch = 35.264389682754654_f64.to_radians();
            }
        }
        self.clamp();
    }

    pub fn toggle_projection(&mut self) {
        self.projection = match self.projection {
            ProjectionMode::Perspective => ProjectionMode::Orthographic,
            ProjectionMode::Orthographic => ProjectionMode::Perspective,
        };
    }

    fn clamp(&mut self) {
        let max_pitch = 89.0_f64.to_radians();
        self.pitch = self.pitch.clamp(-max_pitch, max_pitch);
        self.distance = self.distance.clamp(MIN_DISTANCE, MAX_DISTANCE);
        self.ortho_scale = self.ortho_scale.clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    /// Convert a normalized device coordinate (-1..1, -1..1) to a world-space ray (origin, direction).
    pub fn ndc_to_ray(&self, ndc_x: Scalar, ndc_y: Scalar, aspect: Scalar) -> (Point3, Vec3) {
        let inv_vp = self.view_projection_matrix(aspect).inverse();
        let near = inv_vp.project_point3(Vec3::new(ndc_x, ndc_y, -1.0));
        let far = inv_vp.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        let dir = (far - near).normalize_or_zero();
        (near, dir)
    }

    /// Convert a screen pixel position (relative to viewport rect) to a world-space ray.
    pub fn screen_to_ray(&self, screen_x: Scalar, screen_y: Scalar, viewport_width: Scalar, viewport_height: Scalar) -> (Point3, Vec3) {
        let ndc_x = (screen_x / viewport_width) * 2.0 - 1.0;
        let ndc_y = 1.0 - (screen_y / viewport_height) * 2.0;
        let aspect = viewport_width / viewport_height.max(0.0001);
        self.ndc_to_ray(ndc_x, ndc_y, aspect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_is_clamped() {
        let mut camera = ArcballCamera::default();
        camera.zoom(1_000_000.0);
        assert!(camera.distance >= MIN_DISTANCE);
        camera.zoom(-1_000_000.0);
        assert!(camera.distance <= MAX_DISTANCE);
    }

    #[test]
    fn standard_views_change_orientation() {
        let mut camera = ArcballCamera::default();
        camera.set_standard_view(StandardView::Top);
        assert!(camera.forward().z > 0.99);
        camera.set_standard_view(StandardView::Front);
        assert!(camera.forward().y > 0.99);
    }

    #[test]
    fn projection_switches() {
        let mut camera = ArcballCamera::default();
        assert_eq!(camera.projection, ProjectionMode::Perspective);
        camera.toggle_projection();
        assert_eq!(camera.projection, ProjectionMode::Orthographic);
    }
}
