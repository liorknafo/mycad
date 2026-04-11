use bytemuck::{Pod, Zeroable};
use eframe::egui_wgpu::wgpu;
use eframe::egui_wgpu::wgpu::util::DeviceExt;
use mycad_kernel::math::{Mat4, Vec3};

const GRID_EXTENT: i32 = 20;
const GRID_STEP: f32 = 1.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct LineVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

impl LineVertex {
    pub fn new(start: [f32; 3], end: [f32; 3]) -> [LineVertex; 2] {
        const EDGE_COLOR: [f32; 4] = [0.6, 0.6, 0.6, 1.0];
        [
            LineVertex { position: start, color: EDGE_COLOR },
            LineVertex { position: end, color: EDGE_COLOR },
        ]
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
}

pub struct OverlayGeometry {
    pub vertices: Vec<LineVertex>,
}

impl Default for OverlayGeometry {
    fn default() -> Self {
        Self::new()
    }
}

impl OverlayGeometry {
    pub fn new() -> Self {
        let mut vertices = Vec::new();
        let minor = [0.28, 0.28, 0.32, 1.0];
        let major = [0.38, 0.38, 0.45, 1.0];

        for i in -GRID_EXTENT..=GRID_EXTENT {
            let x = i as f32 * GRID_STEP;
            let color = if i == 0 { major } else { minor };
            vertices.push(LineVertex { position: [x, -(GRID_EXTENT as f32) * GRID_STEP, 0.0], color });
            vertices.push(LineVertex { position: [x, (GRID_EXTENT as f32) * GRID_STEP, 0.0], color });

            let y = i as f32 * GRID_STEP;
            vertices.push(LineVertex { position: [-(GRID_EXTENT as f32) * GRID_STEP, y, 0.0], color });
            vertices.push(LineVertex { position: [(GRID_EXTENT as f32) * GRID_STEP, y, 0.0], color });
        }

        let axis_len = 3.0;
        vertices.extend_from_slice(&[
            LineVertex { position: [0.0, 0.0, 0.0], color: [1.0, 0.25, 0.25, 1.0] },
            LineVertex { position: [axis_len, 0.0, 0.0], color: [1.0, 0.25, 0.25, 1.0] },
            LineVertex { position: [0.0, 0.0, 0.0], color: [0.25, 1.0, 0.25, 1.0] },
            LineVertex { position: [0.0, axis_len, 0.0], color: [0.25, 1.0, 0.25, 1.0] },
            LineVertex { position: [0.0, 0.0, 0.0], color: [0.35, 0.55, 1.0, 1.0] },
            LineVertex { position: [0.0, 0.0, axis_len], color: [0.35, 0.55, 1.0, 1.0] },
        ]);

        Self { vertices }
    }
}

pub struct OverlayResources {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group: wgpu::BindGroup,
    pub uniform_buffer: wgpu::Buffer,
    pub vertex_buffer: wgpu::Buffer,
    pub vertex_count: u32,
    pub sketch_vertex_buffer: Option<wgpu::Buffer>,
    pub sketch_vertex_count: u32,
    pub mesh_edge_vertex_buffer: Option<wgpu::Buffer>,
    pub mesh_edge_vertex_count: u32,
    target_format: wgpu::TextureFormat,
}

impl OverlayResources {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mycad-overlay-shader"),
            source: wgpu::ShaderSource::Wgsl(OVERLAY_SHADER.into()),
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mycad-camera-uniform"),
            contents: bytemuck::bytes_of(&CameraUniform { view_proj: Mat4::IDENTITY.to_cols_array_2d().map(|c| c.map(|v| v as f32)) }),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mycad-overlay-bind-group-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mycad-overlay-bind-group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mycad-overlay-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mycad-overlay-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<LineVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
                }],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(target_format.into())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let geometry = OverlayGeometry::new();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mycad-overlay-vertices"),
            contents: bytemuck::cast_slice(&geometry.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            pipeline,
            bind_group,
            uniform_buffer,
            vertex_buffer,
            vertex_count: geometry.vertices.len() as u32,
            sketch_vertex_buffer: None,
            sketch_vertex_count: 0,
            mesh_edge_vertex_buffer: None,
            mesh_edge_vertex_count: 0,
            target_format,
        }
    }

    pub fn target_format(&self) -> wgpu::TextureFormat {
        self.target_format
    }

    pub fn update_camera(&self, queue: &wgpu::Queue, view_proj: Mat4) {
        let vp = view_proj.to_cols_array_2d().map(|c| c.map(|v| v as f32));
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&CameraUniform { view_proj: vp }));
    }

    pub fn update_sketch_lines(&mut self, device: &wgpu::Device, vertices: &[LineVertex]) {
        if vertices.is_empty() {
            self.sketch_vertex_buffer = None;
            self.sketch_vertex_count = 0;
            return;
        }
        self.sketch_vertex_buffer = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mycad-sketch-vertices"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }));
        self.sketch_vertex_count = vertices.len() as u32;
    }

    pub fn update_mesh_edges(&mut self, device: &wgpu::Device, vertices: &[LineVertex]) {
        if vertices.is_empty() {
            self.mesh_edge_vertex_buffer = None;
            self.mesh_edge_vertex_count = 0;
            return;
        }
        self.mesh_edge_vertex_buffer = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mycad-mesh-edge-vertices"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }));
        self.mesh_edge_vertex_count = vertices.len() as u32;
    }

    pub fn paint(&self, render_pass: &mut wgpu::RenderPass<'static>) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.draw(0..self.vertex_count, 0..1);

        if let Some(sketch_buffer) = &self.sketch_vertex_buffer {
            render_pass.set_vertex_buffer(0, sketch_buffer.slice(..));
            render_pass.draw(0..self.sketch_vertex_count, 0..1);
        }

        if let Some(mesh_edge_buffer) = &self.mesh_edge_vertex_buffer {
            render_pass.set_vertex_buffer(0, mesh_edge_buffer.slice(..));
            render_pass.draw(0..self.mesh_edge_vertex_count, 0..1);
        }
    }
}

pub fn fit_radius_from_points(points: &[Vec3]) -> f64 {
    points.iter().map(|p| p.length()).fold(1.0, f64::max)
}

const OVERLAY_SHADER: &str = r#"
struct CameraUniform {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(input.position, 1.0);
    out.color = input.color;
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
"#;
