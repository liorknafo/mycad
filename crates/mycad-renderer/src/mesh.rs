//! 3D mesh rendering: Phong shading, edge overlay, face coloring.

use bytemuck::{Pod, Zeroable};
use eframe::egui_wgpu::wgpu;
use eframe::egui_wgpu::wgpu::util::DeviceExt;
use mycad_kernel::math::{Mat4, Vec3};

use crate::overlay::LineVertex;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MeshUniforms {
    pub view_proj: [[f32; 4]; 4],
    pub light_dir: [f32; 3],
    pub _padding: u32,
}

pub struct MeshResources {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group: wgpu::BindGroup,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub uniform_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub edge_line_vertices: Vec<LineVertex>,
    pub edge_vertex_buffer: Option<wgpu::Buffer>,
    pub edge_vertex_count: u32,
}

impl MeshResources {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mycad-mesh-shader"),
            source: wgpu::ShaderSource::Wgsl(MESH_SHADER.into()),
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mycad-mesh-uniforms"),
            contents: bytemuck::bytes_of(&MeshUniforms {
                view_proj: Mat4::IDENTITY.to_cols_array_2d().map(|c| c.map(|v| v as f32)),
                light_dir: [0.57735026, 0.57735026, 0.57735026],
                _padding: 0,
            }),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mycad-mesh-bind-group-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mycad-mesh-bind-group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mycad-mesh-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mycad-mesh-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MeshVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x3,
                        2 => Float32x4
                    ],
                }],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Greater,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mycad-mesh-vertices"),
            contents: bytemuck::cast_slice(&[MeshVertex {
                position: [0.0, 0.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                color: [0.7, 0.7, 0.75, 1.0],
            }]),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mycad-mesh-indices"),
            contents: bytemuck::cast_slice(&[0u16]),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            pipeline,
            bind_group,
            vertex_buffer,
            index_buffer,
            uniform_buffer,
            index_count: 0,
            edge_line_vertices: Vec::new(),
            edge_vertex_buffer: None,
            edge_vertex_count: 0,
        }
    }

    pub fn update_mesh(&mut self, device: &wgpu::Device, _queue: &wgpu::Queue, mesh: &mycad_kernel::tessellation::Mesh) {
        if mesh.vertices.is_empty() || mesh.indices.is_empty() {
            self.index_count = 0;
            return;
        }

        let default_color = [0.7f32, 0.7, 0.75, 1.0];

        let mut vertex_normals: Vec<Vec3> = vec![Vec3::ZERO; mesh.vertices.len()];
        let mut vertex_normal_counts: Vec<u32> = vec![0; mesh.vertices.len()];

        for triangle in &mesh.indices {
            let i0 = triangle[0];
            let i1 = triangle[1];
            let i2 = triangle[2];

            let v0 = mesh.vertices[i0];
            let v1 = mesh.vertices[i1];
            let v2 = mesh.vertices[i2];

            let edge1 = v1 - v0;
            let edge2 = v2 - v0;
            let face_normal = edge1.cross(edge2).normalize();

            vertex_normals[i0] += face_normal;
            vertex_normals[i1] += face_normal;
            vertex_normals[i2] += face_normal;
            vertex_normal_counts[i0] += 1;
            vertex_normal_counts[i1] += 1;
            vertex_normal_counts[i2] += 1;
        }

        let mut mesh_vertices = Vec::with_capacity(mesh.vertices.len());
        for (i, vertex) in mesh.vertices.iter().enumerate() {
            let normal = if vertex_normal_counts[i] > 0 {
                (vertex_normals[i] / vertex_normal_counts[i] as f64).normalize()
            } else {
                Vec3::Z
            };

            mesh_vertices.push(MeshVertex {
                position: [vertex.x as f32, vertex.y as f32, vertex.z as f32],
                normal: [normal.x as f32, normal.y as f32, normal.z as f32],
                color: default_color,
            });
        }

        let mut indices: Vec<u32> = Vec::with_capacity(mesh.indices.len() * 3);
        for triangle in &mesh.indices {
            indices.push(triangle[0] as u32);
            indices.push(triangle[1] as u32);
            indices.push(triangle[2] as u32);
        }

        self.vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mycad-mesh-vertices"),
            contents: bytemuck::cast_slice(&mesh_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        self.index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mycad-mesh-indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        self.index_count = indices.len() as u32;
    }

    pub fn update_camera(&self, queue: &wgpu::Queue, view_proj: Mat4) {
        let vp = view_proj.to_cols_array_2d().map(|c| c.map(|v| v as f32));
        let light_dir = Vec3::new(1.0, 1.0, 1.0).normalize();
        
        let uniforms = MeshUniforms {
            view_proj: vp,
            light_dir: [light_dir.x as f32, light_dir.y as f32, light_dir.z as f32],
            _padding: 0,
        };
        
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    pub fn update_edges(&mut self, device: &wgpu::Device, edges: Vec<LineVertex>) {
        self.edge_line_vertices = edges;
        
        if self.edge_line_vertices.is_empty() {
            self.edge_vertex_buffer = None;
            self.edge_vertex_count = 0;
        } else {
            self.edge_vertex_buffer = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mycad-mesh-edges"),
                contents: bytemuck::cast_slice(&self.edge_line_vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }));
            self.edge_vertex_count = self.edge_line_vertices.len() as u32;
        }
    }

    pub fn paint(&self, render_pass: &mut wgpu::RenderPass<'static>) {
        if self.index_count == 0 {
            return;
        }
        
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..self.index_count, 0, 0..1);
    }

    pub fn paint_edges(&self, render_pass: &mut wgpu::RenderPass<'static>) {
        if let Some(edge_buffer) = &self.edge_vertex_buffer {
            render_pass.set_vertex_buffer(0, edge_buffer.slice(..));
            render_pass.draw(0..self.edge_vertex_count, 0..1);
        }
    }
}

const MESH_SHADER: &str = r#"
struct MeshUniforms {
    view_proj: mat4x4<f32>,
    light_dir: vec3<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: MeshUniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world_pos = vec4<f32>(input.position, 1.0);
    out.clip_position = uniforms.view_proj * world_pos;
    out.world_position = input.position;
    out.normal = normalize(input.normal);
    out.color = input.color;
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(input.normal);
    let light_dir = normalize(uniforms.light_dir);
    
    let ambient = 0.15;
    let diffuse = max(dot(normal, light_dir), 0.0);
    
    let view_dir = vec3<f32>(0.0, 0.0, 1.0);
    let half_vector = normalize(light_dir + view_dir);
    let specular = 0.3 * pow(max(dot(half_vector, normal), 0.0), 32.0);
    
    let lighting = ambient + diffuse + specular;
    return vec4<f32>(input.color.rgb * lighting, input.color.a);
}
"#;
