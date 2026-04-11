pub mod viewport;
pub mod mesh;
pub mod overlay;
pub mod picking;

pub use crate::mesh::{MeshResources, MeshVertex, MeshUniforms};

use eframe::{egui, egui_wgpu};
use egui_wgpu::wgpu;

use crate::overlay::{LineVertex, OverlayResources};
pub use crate::viewport::{ArcballCamera, ProjectionMode, StandardView};
use mycad_kernel::math::{Plane, Point2, Point3, Scalar, Vec3, EPSILON};

pub struct Viewport3d {
    camera: ArcballCamera,
    sketch_lines: Vec<LineVertex>,
    last_rect: egui::Rect,
    /// Component → mesh for the current branch's visible components.
    /// Identified by a string key (the app passes a stable id per component).
    meshes: std::collections::HashMap<String, mycad_kernel::tessellation::Mesh>,
    meshes_dirty: bool,
}

impl Viewport3d {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        let render_state = cc.wgpu_render_state.as_ref()?;
        let device = &render_state.device;
        let target_format = render_state.target_format;

        render_state
            .renderer
            .write()
            .callback_resources
            .insert(ViewportResources {
                overlay: OverlayResources::new(device, target_format),
                mesh: None,
            });

        Some(Self {
            camera: ArcballCamera::default(),
            sketch_lines: Vec::new(),
            last_rect: egui::Rect::NOTHING,
            meshes: std::collections::HashMap::new(),
            meshes_dirty: false,
        })
    }

    pub fn camera(&self) -> &ArcballCamera {
        &self.camera
    }

    pub fn set_standard_view(&mut self, view: StandardView) {
        self.camera.set_standard_view(view);
    }

    pub fn fit_all(&mut self) {
        self.camera.fit(mycad_kernel::math::Point3::ZERO, 10.0);
    }

    pub fn toggle_projection(&mut self) {
        self.camera.toggle_projection();
    }

    pub fn last_rect(&self) -> egui::Rect {
        self.last_rect
    }

    pub fn set_sketch_lines(&mut self, lines: Vec<LineVertex>) {
        self.sketch_lines = lines;
    }

    pub fn clear_sketch_lines(&mut self) {
        self.sketch_lines.clear();
    }

    pub fn set_mesh(&mut self, mesh: Option<mycad_kernel::tessellation::Mesh>) {
        self.meshes.clear();
        if let Some(m) = mesh {
            self.meshes.insert("__default__".into(), m);
        }
        self.sketch_lines.clear();
        self.meshes_dirty = true;
        self.fit_mesh();
    }

    pub fn set_component_mesh(&mut self, component_key: String, mesh: mycad_kernel::tessellation::Mesh) {
        self.meshes.insert(component_key, mesh);
        self.meshes_dirty = true;
    }

    pub fn remove_component_mesh(&mut self, component_key: &str) {
        self.meshes.remove(component_key);
        self.meshes_dirty = true;
    }

    pub fn clear_mesh(&mut self) {
        self.meshes.clear();
        self.meshes_dirty = true;
    }

    pub fn iter_meshes(&self) -> impl Iterator<Item = (&String, &mycad_kernel::tessellation::Mesh)> {
        self.meshes.iter()
    }

    pub fn fit_mesh(&mut self) {
        let mut min = Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut max = Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);

        for mesh in self.meshes.values() {
            for vertex in &mesh.vertices {
                min.x = min.x.min(vertex.x);
                min.y = min.y.min(vertex.y);
                min.z = min.z.min(vertex.z);
                max.x = max.x.max(vertex.x);
                max.y = max.y.max(vertex.y);
                max.z = max.z.max(vertex.z);
            }
        }

        if min.x != f64::INFINITY && max.x != f64::NEG_INFINITY {
            let center = Point3::new(
                (min.x + max.x) * 0.5,
                (min.y + max.y) * 0.5,
                (min.z + max.z) * 0.5,
            );
            let size = ((max.x - min.x).powi(2) + (max.y - min.y).powi(2) + (max.z - min.z).powi(2)).sqrt();
            self.camera.fit(center, size);
        }
    }

    pub fn screen_to_sketch_point(
        &self,
        screen_pos: egui::Pos2,
        rect: egui::Rect,
        plane: &Plane,
    ) -> Option<Point2> {
        let local_x = (screen_pos.x - rect.left()) as Scalar;
        let local_y = (screen_pos.y - rect.top()) as Scalar;
        let (origin, dir) = self.camera.screen_to_ray(
            local_x,
            local_y,
            rect.width() as Scalar,
            rect.height() as Scalar,
        );
        ray_plane_intersection(origin, dir, plane).map(|world_pt| plane.project_point(world_pt))
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, sketch_mode: bool) -> ViewportResponse {
        let available = ui.available_size_before_wrap();
        let desired = egui::vec2(available.x.max(64.0), available.y.max(64.0));
        let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());
        self.last_rect = rect;

        let mut vr = ViewportResponse {
            clicked: false,
            hover_pos: response.hover_pos(),
            escape_pressed: false,
        };

        if sketch_mode {
            // In sketch mode: LMB = sketch tool, RMB/MMB = pan, scroll = zoom
            if response.dragged_by(egui::PointerButton::Middle) || response.dragged_by(egui::PointerButton::Secondary) {
                let delta = response.drag_delta();
                self.camera.pan(delta.x as f64, delta.y as f64);
                ui.ctx().request_repaint();
            }
            if response.clicked_by(egui::PointerButton::Primary) {
                vr.clicked = true;
            }
        } else {
            // Normal mode: LMB = orbit, RMB/MMB = pan
            if response.dragged_by(egui::PointerButton::Primary) {
                let delta = response.drag_delta();
                self.camera.orbit(delta.x as f64, delta.y as f64);
                ui.ctx().request_repaint();
            }
            if response.dragged_by(egui::PointerButton::Middle) || response.dragged_by(egui::PointerButton::Secondary) {
                let delta = response.drag_delta();
                self.camera.pan(delta.x as f64, delta.y as f64);
                ui.ctx().request_repaint();
            }
        }

        if response.hovered() {
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll.abs() > f32::EPSILON {
                self.camera.zoom(scroll as f64);
                ui.ctx().request_repaint();
            }
            ui.ctx().request_repaint();
        }

        vr.escape_pressed = ui.input(|i| i.key_pressed(egui::Key::Escape));

        let aspect = (rect.width() / rect.height().max(1.0)) as f64;

        // Concatenate all meshes for rendering
        let combined_mesh = if !self.meshes.is_empty() {
            let mut vertices = Vec::new();
            let mut indices = Vec::new();
            let mut current_index = 0usize;

            for mesh in self.meshes.values() {
                vertices.extend(&mesh.vertices);
                for &[i0, i1, i2] in &mesh.indices {
                    indices.push([
                        current_index + i0,
                        current_index + i1,
                        current_index + i2,
                    ]);
                }
                current_index += mesh.vertices.len();
            }

            Some(std::sync::Arc::new(mycad_kernel::tessellation::Mesh {
                vertices,
                indices,
                normals: Vec::new(),
            }))
        } else {
            None
        };

        let callback = ViewportCallback {
            view_proj: self.camera.view_projection_matrix(aspect),
            sketch_lines: self.sketch_lines.clone(),
            mesh_data: combined_mesh,
        };

        self.meshes_dirty = false;

        ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgb(26, 26, 34));
        ui.painter()
            .add(egui_wgpu::Callback::new_paint_callback(rect, callback));

        vr
    }
}

pub struct ViewportResponse {
    pub clicked: bool,
    pub hover_pos: Option<egui::Pos2>,
    pub escape_pressed: bool,
}

fn ray_plane_intersection(origin: Point3, dir: Vec3, plane: &Plane) -> Option<Point3> {
    let denom = dir.dot(plane.normal);
    if denom.abs() < EPSILON {
        return None;
    }
    let t = (plane.origin - origin).dot(plane.normal) / denom;
    if t < 0.0 {
        return None;
    }
    Some(origin + dir * t)
}

struct ViewportResources {
    overlay: OverlayResources,
    mesh: Option<MeshResources>,
}

struct ViewportCallback {
    view_proj: mycad_kernel::math::Mat4,
    sketch_lines: Vec<LineVertex>,
    mesh_data: Option<std::sync::Arc<mycad_kernel::tessellation::Mesh>>,
}

impl egui_wgpu::CallbackTrait for ViewportCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources: &mut ViewportResources = resources.get_mut().expect("ViewportResources missing");
        resources.overlay.update_camera(queue, self.view_proj);
        resources.overlay.update_sketch_lines(device, &self.sketch_lines);

        if let Some(ref mesh_data) = self.mesh_data {
            if resources.mesh.is_none() {
                resources.mesh = Some(MeshResources::new(device, resources.overlay.target_format()));
            }
            if let Some(ref mut mesh) = resources.mesh {
                mesh.update_mesh(device, queue, mesh_data);
                mesh.update_camera(queue, self.view_proj);

                let mut edge_lines = Vec::new();
                for [i0, i1, i2] in &mesh_data.indices {
                    let v0 = mesh_data.vertices[*i0];
                    let v1 = mesh_data.vertices[*i1];
                    let v2 = mesh_data.vertices[*i2];
                    edge_lines.extend(LineVertex::new([v0.x as f32, v0.y as f32, v0.z as f32], [v1.x as f32, v1.y as f32, v1.z as f32]));
                    edge_lines.extend(LineVertex::new([v1.x as f32, v1.y as f32, v1.z as f32], [v2.x as f32, v2.y as f32, v2.z as f32]));
                    edge_lines.extend(LineVertex::new([v2.x as f32, v2.y as f32, v2.z as f32], [v0.x as f32, v0.y as f32, v0.z as f32]));
                }
                resources.overlay.update_mesh_edges(device, &edge_lines);
            }
        } else {
            resources.mesh = None;
            resources.overlay.update_mesh_edges(device, &[]);
        }

        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let resources: &ViewportResources = resources.get().expect("ViewportResources missing");

        if let Some(ref mesh) = resources.mesh {
            mesh.paint(render_pass);
        }

        resources.overlay.paint(render_pass);
    }
}
