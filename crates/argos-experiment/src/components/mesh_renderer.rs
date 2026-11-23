use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use gpui_kit::RenderImage;
use image::{Frame, RgbaImage};
use smallvec::smallvec;
use terra_comms::MeshChunk;
use wgpu::util::DeviceExt;

/// How `MeshRenderer` colours mesh triangles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MeshColorMode {
    /// Use per-vertex RGB from `MeshChunk.colors` when available, falling back
    /// to `ARMeshClassification` colours when `colors` is empty.
    #[default]
    VertexColor,
    /// Always colour faces by `ARMeshClassification`.
    Classification,
}

/// Convert an 8-bit sRGB channel `[0, 255]` to linear light `[0.0, 1.0]`
/// for the `Bgra8UnormSrgb` render target.
fn srgb_u8_to_linear(channel: u8) -> f32 {
    let s = channel as f32 / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

fn srgb_rgb_to_linear(rgb: [u8; 3]) -> [f32; 3] {
    [
        srgb_u8_to_linear(rgb[0]),
        srgb_u8_to_linear(rgb[1]),
        srgb_u8_to_linear(rgb[2]),
    ]
}

/// One vertex sent to the GPU.
/// Each triangle is expanded into 3 vertices so every face gets its own crisp
/// flat surface normal, plus either per-vertex camera colour (`MeshChunk.colors`)
/// or `ARMeshClassification` colour.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct MeshVertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
}

impl MeshVertex {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: 12,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: 24,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x3,
            },
        ],
    };
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
}

/// Interactive 3D orbit camera in Terra's Z-up world frame.
#[derive(Clone, Copy, Debug)]
pub struct OrbitCamera {
    pub target: [f32; 3],
    pub yaw: f32,      // radians around +Z
    pub pitch: f32,    // radians above XY plane
    pub distance: f32, // metres from target
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: [0.0, 0.0, 0.0],
            yaw: -0.7,
            pitch: 0.55,
            distance: 5.5,
        }
    }
}

impl OrbitCamera {
    pub fn orbit(&mut self, dx_px: f32, dy_px: f32) {
        self.yaw -= dx_px * 0.008;
        self.pitch = (self.pitch + dy_px * 0.008).clamp(-1.45, 1.45);
    }

    pub fn pan(&mut self, dx_px: f32, dy_px: f32) {
        let scale = self.distance * 0.002;
        // Camera right vector in XY plane (perpendicular to view yaw).
        let right = [-self.yaw.sin(), self.yaw.cos(), 0.0];
        let forward_xy = [self.yaw.cos(), self.yaw.sin(), 0.0];
        self.target[0] -= (right[0] * dx_px - forward_xy[0] * dy_px) * scale;
        self.target[1] -= (right[1] * dx_px - forward_xy[1] * dy_px) * scale;
    }

    pub fn zoom(&mut self, delta: f32) {
        let factor = (1.0 - delta * 0.004).clamp(0.8, 1.25);
        self.distance = (self.distance * factor).clamp(0.5, 40.0);
    }

    /// Column-major 4×4 `projection * view` matrix (wgpu depth in `[0, 1]`, Z-up).
    fn view_proj(&self, aspect: f32) -> [[f32; 4]; 4] {
        let eye = [
            self.target[0] + self.distance * self.pitch.cos() * self.yaw.cos(),
            self.target[1] + self.distance * self.pitch.cos() * self.yaw.sin(),
            self.target[2] + self.distance * self.pitch.sin(),
        ];
        let view = look_at_rh(eye, self.target, [0.0, 0.0, 1.0]);
        let proj = perspective_rh_zo(50.0_f32.to_radians(), aspect.max(0.1), 0.05, 100.0);
        mat4_mul(proj, view)
    }
}

/// A rover to draw in the scene: where it is and how to colour it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoverMarker {
    /// World frame, metres. The rover stands on the ground (z = 0).
    pub position: [f32; 2],
    /// Radians, counter-clockwise from +X.
    pub yaw: f32,
    /// Physical rover body length (forward, m) and width (left-right, m).
    pub length: f32,
    pub width: f32,
    pub color: [f32; 3],
}

/// The marker starts slightly above the ground plane, metres.
const ROVER_BOTTOM_M: f32 = 0.01;

/// A planned path to draw in the 3D scene as a flat ribbon slightly above the floor.
#[derive(Clone, Debug, PartialEq)]
pub struct PathOverlay {
    /// Waypoints in the world frame, metres (`[x, y]`).
    pub points: Vec<[f32; 2]>,
    pub color: [f32; 3],
}

/// Height of the planned-path ribbon above the ground plane, metres.
const PATH_Z_M: f32 = 0.04;
/// Half-width of the planned-path ribbon, metres.
const PATH_HALF_WIDTH_M: f32 = 0.035;
/// Radius of the endpoint disc at the end of a planned path, metres.
const PATH_END_RADIUS_M: f32 = 0.09;

struct GpuChunk {
    buffer: wgpu::Buffer,
    vertex_count: u32,
}

/// Offscreen `wgpu` 3D renderer for `ARMeshAnchor` chunks, returning a BGRA
/// `RenderImage` ready for GPUI's `img(...)`.
pub struct MeshRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,

    width: u32,
    height: u32,
    color_tex: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    readback_buffer: wgpu::Buffer,
    padded_bytes_per_row: u32,

    grid_buffer: wgpu::Buffer,
    grid_vertex_count: u32,
    chunks: HashMap<(u64, String), GpuChunk>,
    /// One buffer with every rover marker; `None` when there are none.
    rovers: Option<GpuChunk>,
    /// One buffer with every rover's planned path ribbon; `None` when empty.
    paths: Option<GpuChunk>,
}

impl MeshRenderer {
    pub fn new(width: u32, height: u32) -> anyhow::Result<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| anyhow::anyhow!("No compatible GPU adapter found"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("argos-mesh-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        ))?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_WGSL.into()),
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera-uniform"),
            contents: bytemuck::bytes_of(&CameraUniform {
                view_proj: OrbitCamera::default().view_proj(width as f32 / height as f32),
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera-bgl"),
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

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera-bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[MeshVertex::LAYOUT],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Bgra8UnormSrgb,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None, // show both sides of ARKit triangles
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let (color_tex, color_view, depth_view, readback_buffer, padded_bytes_per_row) =
            create_targets(&device, width, height);

        let grid_verts = build_ground_grid(6, 0.015);
        let grid_vertex_count = grid_verts.len() as u32;
        let grid_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ground-grid"),
            contents: bytemuck::cast_slice(&grid_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            camera_buffer,
            camera_bind_group,
            width,
            height,
            color_tex,
            color_view,
            depth_view,
            readback_buffer,
            padded_bytes_per_row,
            grid_buffer,
            grid_vertex_count,
            chunks: HashMap::new(),
            rovers: None,
            paths: None,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.clamp(64, 2048);
        let height = height.clamp(64, 2048);
        if width == self.width && height == self.height {
            return;
        }
        let (color_tex, color_view, depth_view, readback_buffer, padded_bytes_per_row) =
            create_targets(&self.device, width, height);
        self.width = width;
        self.height = height;
        self.color_tex = color_tex;
        self.color_view = color_view;
        self.depth_view = depth_view;
        self.readback_buffer = readback_buffer;
        self.padded_bytes_per_row = padded_bytes_per_row;
    }

    /// Upload or replace one `MeshChunk` on the GPU.
    /// Empty `vertices` / `indices` removes the chunk.
    pub fn upsert_chunk(
        &mut self,
        key: (u64, String),
        chunk: &MeshChunk,
        hide_ceiling: bool,
        color_mode: MeshColorMode,
    ) {
        if chunk.vertices.is_empty() || chunk.indices.is_empty() {
            self.chunks.remove(&key);
            return;
        }

        let has_vertex_colors =
            color_mode == MeshColorMode::VertexColor && chunk.colors.len() == chunk.vertices.len();

        let mut verts = Vec::with_capacity(chunk.indices.len() * 3);
        for (face_i, &[i0, i1, i2]) in chunk.indices.iter().enumerate() {
            let cls = chunk.classes.get(face_i).copied().unwrap_or(0);
            if hide_ceiling && cls == 3 {
                continue;
            }
            let (Some(&p0), Some(&p1), Some(&p2)) = (
                chunk.vertices.get(i0 as usize),
                chunk.vertices.get(i1 as usize),
                chunk.vertices.get(i2 as usize),
            ) else {
                continue;
            };
            let normal = normalize(cross(sub(p1, p0), sub(p2, p0)));
            let (c0, c1, c2) = if has_vertex_colors {
                (
                    srgb_rgb_to_linear(chunk.colors[i0 as usize]),
                    srgb_rgb_to_linear(chunk.colors[i1 as usize]),
                    srgb_rgb_to_linear(chunk.colors[i2 as usize]),
                )
            } else {
                let c = class_rgb(cls);
                (c, c, c)
            };
            verts.push(MeshVertex { position: p0, normal, color: c0 });
            verts.push(MeshVertex { position: p1, normal, color: c1 });
            verts.push(MeshVertex { position: p2, normal, color: c2 });
        }

        if verts.is_empty() {
            self.chunks.remove(&key);
            return;
        }

        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh-chunk"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
        self.chunks.insert(
            key,
            GpuChunk {
                buffer,
                vertex_count: verts.len() as u32,
            },
        );
    }

    /// Replace the rover markers drawn in the scene.
    pub fn set_rovers(&mut self, markers: &[RoverMarker]) {
        let verts: Vec<MeshVertex> = markers.iter().flat_map(rover_triangles).collect();
        if verts.is_empty() {
            self.rovers = None;
            return;
        }
        let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("rover-markers"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        self.rovers = Some(GpuChunk { buffer, vertex_count: verts.len() as u32 });
    }

    /// Replace the planned-path ribbons drawn in the scene.
    pub fn set_paths(&mut self, paths: &[PathOverlay]) {
        let verts: Vec<MeshVertex> = paths.iter().flat_map(path_triangles).collect();
        if verts.is_empty() {
            self.paths = None;
            return;
        }
        let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("planned-paths"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        self.paths = Some(GpuChunk { buffer, vertex_count: verts.len() as u32 });
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
    }

    /// Render all uploaded chunks from `camera` into a GPUI `RenderImage` (BGRA).
    pub fn render(&self, camera: &OrbitCamera) -> Arc<RenderImage> {
        let pixels = self.render_bgra_pixels(camera);
        let img = RgbaImage::from_raw(self.width, self.height, pixels)
            .expect("dimensions match BGRA buffer");
        Arc::new(RenderImage::new(smallvec![Frame::new(img)]))
    }

    /// Render all uploaded chunks from `camera` into a standard RGBA image
    /// suitable for saving to PNG/JPEG on disk.
    pub fn render_rgba(&self, camera: &OrbitCamera) -> RgbaImage {
        let mut pixels = self.render_bgra_pixels(camera);
        for px in pixels.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        RgbaImage::from_raw(self.width, self.height, pixels)
            .expect("dimensions match RGBA buffer")
    }

    fn render_bgra_pixels(&self, camera: &OrbitCamera) -> Vec<u8> {
        let uniform = CameraUniform {
            view_proj: camera.view_proj(self.width as f32 / self.height as f32),
        };
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("mesh-render-encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mesh-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.012,
                            g: 0.014,
                            b: 0.018,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });

            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);

            // Reference 1m ground grid
            pass.set_vertex_buffer(0, self.grid_buffer.slice(..));
            pass.draw(0..self.grid_vertex_count, 0..1);

            // All ARKit mesh chunks, then planned paths, then the rovers standing in them.
            for chunk in self.chunks.values().chain(&self.paths).chain(&self.rovers) {
                pass.set_vertex_buffer(0, chunk.buffer.slice(..));
                pass.draw(0..chunk.vertex_count, 0..1);
            }
        }

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.color_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(Some(encoder.finish()));

        // Map readback buffer and strip row alignment padding.
        let slice = self.readback_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        self.device.poll(wgpu::Maintain::Wait);
        let _ = rx.recv();

        let mapped = slice.get_mapped_range();
        let unpadded_row = (self.width * 4) as usize;
        let padded_row = self.padded_bytes_per_row as usize;
        let mut pixels = Vec::with_capacity(unpadded_row * self.height as usize);
        for row in 0..self.height as usize {
            let start = row * padded_row;
            pixels.extend_from_slice(&mapped[start..start + unpadded_row]);
        }
        drop(mapped);
        self.readback_buffer.unmap();
        pixels
    }
}

fn create_targets(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::TextureView, wgpu::Buffer, u32) {
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let color_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mesh-color"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_view = color_tex.create_view(&Default::default());

    let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mesh-depth"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth_tex.create_view(&Default::default());

    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bytes_per_row = (width * 4).div_ceil(align) * align;
    let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mesh-readback"),
        size: u64::from(padded_bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    (color_tex, color_view, depth_view, readback_buffer, padded_bytes_per_row)
}

/// RGB colour for `ARMeshClassification` (0=none, 1=wall, 2=floor, 3=ceiling,
/// 4=table, 5=seat, 6=window, 7=door).
/// The triangles of one rover marker: a pointed-nose prism matching the
/// rover's real physical `length × width` turned to its heading.
fn rover_triangles(marker: &RoverMarker) -> Vec<MeshVertex> {
    let half_l = (marker.length * 0.5).max(0.05);
    let half_w = (marker.width * 0.5).max(0.04);
    let shoulder = half_l * 0.5;
    let outline: [[f32; 2]; 5] = [
        [-half_l, -half_w],
        [shoulder, -half_w],
        [half_l, 0.0],
        [shoulder, half_w],
        [-half_l, half_w],
    ];
    let top_z = (marker.width * 0.45).clamp(0.08, 0.28);

    let (sin, cos) = marker.yaw.sin_cos();
    let corner = |[forward, left]: [f32; 2], z: f32| {
        [
            marker.position[0] + forward * cos - left * sin,
            marker.position[1] + forward * sin + left * cos,
            z,
        ]
    };
    let bottom: Vec<[f32; 3]> = outline.iter().map(|&point| corner(point, ROVER_BOTTOM_M)).collect();
    let top: Vec<[f32; 3]> = outline.iter().map(|&point| corner(point, top_z)).collect();

    let mut verts = Vec::new();
    let mut triangle = |p0: [f32; 3], p1: [f32; 3], p2: [f32; 3]| {
        let normal = normalize(cross(sub(p1, p0), sub(p2, p0)));
        for position in [p0, p1, p2] {
            verts.push(MeshVertex { position, normal, color: marker.color });
        }
    };

    // The lid: a fan from the first corner.
    for i in 1..top.len() - 1 {
        triangle(top[0], top[i], top[i + 1]);
    }
    // The walls: two triangles per edge of the outline.
    for i in 0..top.len() {
        let next = (i + 1) % top.len();
        triangle(bottom[i], bottom[next], top[next]);
        triangle(bottom[i], top[next], top[i]);
    }
    verts
}

/// The triangles of one planned path: a flat ribbon at [`PATH_Z_M`] with
/// smooth disc joins at corners and a small endpoint disc at the destination.
fn path_triangles(path: &PathOverlay) -> Vec<MeshVertex> {
    if path.points.len() < 2 {
        return Vec::new();
    }
    let normal = [0.0, 0.0, 1.0];
    let color = path.color;
    let mut verts = Vec::new();
    let push_tri = |verts: &mut Vec<MeshVertex>, p0: [f32; 3], p1: [f32; 3], p2: [f32; 3]| {
        verts.push(MeshVertex { position: p0, normal, color });
        verts.push(MeshVertex { position: p1, normal, color });
        verts.push(MeshVertex { position: p2, normal, color });
    };

    for seg in path.points.windows(2) {
        let [a, b] = [seg[0], seg[1]];
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let len = dx.hypot(dy);
        if len < 1e-4 {
            continue;
        }
        let px = -dy / len * PATH_HALF_WIDTH_M;
        let py = dx / len * PATH_HALF_WIDTH_M;
        let a_l = [a[0] + px, a[1] + py, PATH_Z_M];
        let a_r = [a[0] - px, a[1] - py, PATH_Z_M];
        let b_l = [b[0] + px, b[1] + py, PATH_Z_M];
        let b_r = [b[0] - px, b[1] - py, PATH_Z_M];
        push_tri(&mut verts, a_l, a_r, b_r);
        push_tri(&mut verts, a_l, b_r, b_l);
    }

    if verts.is_empty() {
        return verts;
    }

    let mut push_disc = |center: [f32; 2], radius: f32, z: f32, sides: usize| {
        let c = [center[0], center[1], z];
        for i in 0..sides {
            let a0 = (i as f32 / sides as f32) * std::f32::consts::TAU;
            let a1 = ((i + 1) as f32 / sides as f32) * std::f32::consts::TAU;
            let p0 = [center[0] + radius * a0.cos(), center[1] + radius * a0.sin(), z];
            let p1 = [center[0] + radius * a1.cos(), center[1] + radius * a1.sin(), z];
            push_tri(&mut verts, c, p0, p1);
        }
    };

    for &pt in &path.points[1..path.points.len() - 1] {
        push_disc(pt, PATH_HALF_WIDTH_M, PATH_Z_M, 6);
    }
    if let Some(&last) = path.points.last() {
        push_disc(last, PATH_END_RADIUS_M, PATH_Z_M + 0.002, 10);
    }
    verts
}

fn class_rgb(cls: u8) -> [f32; 3] {
    match cls {
        2 => [0.10, 0.72, 0.48], // floor: emerald
        1 => [0.90, 0.30, 0.38], // wall: coral
        4 => [0.95, 0.62, 0.12], // table: amber
        5 => [0.92, 0.80, 0.20], // seat: yellow
        7 => [0.22, 0.72, 0.96], // door: sky
        6 => [0.65, 0.55, 0.96], // window: violet
        3 => [0.35, 0.40, 0.50], // ceiling: muted slate
        _ => [0.48, 0.55, 0.65], // none: cool grey
    }
}

/// Thin quad strips forming a 1m reference grid on the `z = -0.002` plane.
fn build_ground_grid(half_extent: i32, half_width: f32) -> Vec<MeshVertex> {
    let mut out = Vec::new();
    let z = -0.002;
    let n = [0.0, 0.0, 1.0];
    let e = half_extent as f32;

    let mut push_quad = |p0: [f32; 3], p1: [f32; 3], p2: [f32; 3], p3: [f32; 3], color: [f32; 3]| {
        out.push(MeshVertex { position: p0, normal: n, color });
        out.push(MeshVertex { position: p1, normal: n, color });
        out.push(MeshVertex { position: p2, normal: n, color });
        out.push(MeshVertex { position: p0, normal: n, color });
        out.push(MeshVertex { position: p2, normal: n, color });
        out.push(MeshVertex { position: p3, normal: n, color });
    };

    for i in -half_extent..=half_extent {
        let f = i as f32;
        let hw = if i == 0 { half_width * 1.6 } else { half_width };
        let col_x = if i == 0 { [0.20, 0.55, 0.30] } else { [0.16, 0.20, 0.27] };
        let col_y = if i == 0 { [0.60, 0.22, 0.25] } else { [0.16, 0.20, 0.27] };
        // Line parallel to Y at x = f
        push_quad([f - hw, -e, z], [f + hw, -e, z], [f + hw, e, z], [f - hw, e, z], col_x);
        // Line parallel to X at y = f
        push_quad([-e, f - hw, z], [e, f - hw, z], [e, f + hw, z], [-e, f + hw, z], col_y);
    }
    out
}

const SHADER_WGSL: &str = r#"
struct Camera {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_pos = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.normal = in.normal;
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let light_dir = normalize(vec3<f32>(0.45, 0.35, 0.82));
    let diffuse = abs(dot(normalize(in.normal), light_dir));
    let shade = 0.38 + 0.62 * diffuse;
    return vec4<f32>(in.color * shade, 1.0);
}
"#;

// --- Minimal 3D vector/matrix math (column-major, right-handed, depth [0, 1]) ---

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = dot(v, v).sqrt();
    if len > 1e-6 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn look_at_rh(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> [[f32; 4]; 4] {
    let f = normalize(sub(target, eye));
    let s = normalize(cross(f, up));
    let u = cross(s, f);
    [
        [s[0], u[0], -f[0], 0.0],
        [s[1], u[1], -f[1], 0.0],
        [s[2], u[2], -f[2], 0.0],
        [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.0],
    ]
}

fn perspective_rh_zo(fov_y: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let f = 1.0 / (fov_y * 0.5).tan();
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, (near * far) / (near - far), 0.0],
    ]
}

fn mat4_mul(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            out[col][row] = a[0][row] * b[col][0]
                + a[1][row] * b[col][1]
                + a[2][row] * b[col][2]
                + a[3][row] * b[col][3];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_triangles_handles_empty_and_multi_segment_paths() {
        let color = [0.5, 0.6, 1.0];
        assert!(path_triangles(&PathOverlay { points: vec![], color }).is_empty());
        assert!(path_triangles(&PathOverlay { points: vec![[0.0, 0.0]], color }).is_empty());
        assert!(path_triangles(&PathOverlay {
            points: vec![[1.0, 1.0], [1.0, 1.0]],
            color,
        })
        .is_empty());

        let verts = path_triangles(&PathOverlay {
            points: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
            color,
        });
        // 2 segments (2 * 6 verts) + 1 interior 6-sided join disc (6 * 3 verts) + 1 10-sided end disc (10 * 3 verts)
        assert_eq!(verts.len(), 12 + 18 + 30);
        assert!(verts.iter().all(|v| v.position[2] >= PATH_Z_M));
    }
}
