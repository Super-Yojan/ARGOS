use super::api::{ObjectId, ObjectRecord};
use super::{
    camera::View,
    content::{TileContent, Vertex},
};
use anyhow::{Result, anyhow, ensure};
use glam::DMat4;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Instant,
};
use wgpu::util::DeviceExt;

struct GpuPrimitive {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    model: DMat4,
    pipeline: usize,
    texture: wgpu::TextureView,
    sampler: wgpu::Sampler,
    material: [f32; 24],
}
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    pipelines: Vec<wgpu::RenderPipeline>,
    tiles: HashMap<usize, Vec<GpuPrimitive>>,
    models: HashMap<PathBuf, Vec<GpuPrimitive>>,
    objects: HashMap<ObjectId, (PathBuf, Vec<GpuPrimitive>)>,
    target: Option<Target>,
    surfaces: HashMap<usize, Arc<super::surface::Surface>>,
    pub(super) surface: super::surface::SharedSurface,
    survey: bool,
    pub metrics: super::metrics::MapMetrics,
}
struct Target {
    color: wgpu::Texture,
    view: wgpu::TextureView,
    depth: wgpu::TextureView,
    readback: wgpu::Buffer,
    width: u32,
    height: u32,
    stride: u32,
}
impl Renderer {
    pub fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .map_err(|_| anyhow!("No native GPU adapter available"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("native map"),
            ..Default::default()
        }))
        .map_err(|_| anyhow!("Unable to initialize native map GPU"))?;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map material"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("map pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("map shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let mut pipelines = Vec::new();
        for blend in [false, true] {
            for double_sided in [false, true] {
                pipelines.push(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label:Some("textured map"),layout:Some(&pipeline_layout),
                vertex:wgpu::VertexState { module:&shader,entry_point:Some("vs_main"),compilation_options:Default::default(),buffers:&[wgpu::VertexBufferLayout { array_stride:size_of::<Vertex>() as u64,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Float32x4] }] },
                fragment:Some(wgpu::FragmentState { module:&shader,entry_point:Some("fs_main"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState { format:wgpu::TextureFormat::Bgra8UnormSrgb,blend:blend.then_some(wgpu::BlendState::ALPHA_BLENDING),write_mask:wgpu::ColorWrites::ALL })] }),
                primitive:wgpu::PrimitiveState { cull_mode:if double_sided{None}else{Some(wgpu::Face::Back)},..Default::default() },
                depth_stencil:Some(wgpu::DepthStencilState { format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:!blend,depth_compare:wgpu::CompareFunction::LessEqual,stencil:Default::default(),bias:Default::default() }),
                multisample:Default::default(),multiview:None,cache:None,
            }));
            }
        }
        Ok(Self {
            device,
            queue,
            layout,
            pipelines,
            tiles: HashMap::new(),
            models: HashMap::new(),
            objects: HashMap::new(),
            target: None,
            surfaces: HashMap::new(),
            surface: Default::default(),
            survey: false,
            metrics: Default::default(),
        })
    }
    pub fn upload(&mut self, id: usize, content: &TileContent, world: DMat4) -> Result<()> {
        let output = self.prepare(content, world, self.survey)?;
        self.surfaces.insert(
            id,
            Arc::new(super::surface::Surface::from_content(content, world)),
        );
        self.tiles.insert(id, output);
        Ok(())
    }
    fn prepare(
        &self,
        content: &TileContent,
        world: DMat4,
        survey: bool,
    ) -> Result<Vec<GpuPrimitive>> {
        let mut output = Vec::new();
        for p in &content.primitives {
            let (width, height) = p.texture.dimensions();
            ensure!(
                width > 0
                    && height > 0
                    && width <= self.device.limits().max_texture_dimension_2d
                    && height <= self.device.limits().max_texture_dimension_2d,
                "Tile texture exceeds GPU limits"
            );
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("tile texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.queue.write_texture(
                texture.as_image_copy(),
                p.texture.as_raw(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                texture.size(),
            );
            let address = |mode| match mode {
                gltf::texture::WrappingMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
                gltf::texture::WrappingMode::MirroredRepeat => wgpu::AddressMode::MirrorRepeat,
                gltf::texture::WrappingMode::Repeat => wgpu::AddressMode::Repeat,
            };
            let filter = if p.sampler.nearest {
                wgpu::FilterMode::Nearest
            } else {
                wgpu::FilterMode::Linear
            };
            let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("tile sampler"),
                address_mode_u: address(p.sampler.wrap_s),
                address_mode_v: address(p.sampler.wrap_t),
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            });
            let mut uniform_data = [0f32; 24];
            uniform_data[18] = if survey { 1. } else { 0. };
            uniform_data[16] = p.alpha_cutoff;
            uniform_data[17] = if p.blend { 0. } else { 1. };
            let uniform = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("tile transform"),
                    contents: bytemuck::cast_slice(&uniform_data),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });
            let view = texture.create_view(&Default::default());
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tile material"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });
            let model = world * p.transform;
            let mut indices = p.indices.clone();
            if model.determinant() < 0. {
                for triangle in indices.as_chunks_mut::<3>().0 {
                    triangle.swap(1, 2);
                }
            }
            output.push(GpuPrimitive {
                vertices: self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("tile vertices"),
                        contents: bytemuck::cast_slice(&p.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                indices: self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("tile indices"),
                        contents: bytemuck::cast_slice(&indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                count: indices.len() as u32,
                uniform,
                bind,
                model,
                pipeline: usize::from(p.blend) * 2 + usize::from(p.double_sided),
                texture: view,
                sampler,
                material: uniform_data,
            });
        }
        Ok(output)
    }
    /// Share vertex/index/texture handles; each instance owns only material uniforms.
    pub(super) fn sync_objects(
        &mut self,
        records: &[ObjectRecord],
        models: &HashMap<PathBuf, Arc<TileContent>>,
    ) -> Vec<(PathBuf, String)> {
        let mut errors = Vec::new();
        let ids: HashSet<_> = records.iter().map(|o| o.id).collect();
        let paths: HashSet<_> = records.iter().map(|o| &o.path).collect();
        self.objects.retain(|id, _| ids.contains(id));
        self.models
            .retain(|path, _| paths.contains(path) && models.contains_key(path));
        for object in records {
            if errors.iter().any(|(path, _)| path == &object.path) {
                continue;
            }
            if !self.models.contains_key(&object.path)
                && let Some(content) = models.get(&object.path)
            {
                match self.prepare(content, DMat4::IDENTITY, false) {
                    Ok(primitives) => {
                        self.models.insert(object.path.clone(), primitives);
                    }
                    Err(error) => {
                        errors.push((object.path.clone(), error.to_string()));
                        continue;
                    }
                }
            }
            let Some(template) = self.models.get(&object.path) else {
                continue;
            };
            if !self.objects.contains_key(&object.id) {
                let mut primitives = Vec::new();
                for p in template {
                    let uniform =
                        self.device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("object material"),
                                contents: bytemuck::cast_slice(&p.material),
                                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                            });
                    let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("object instance"),
                        layout: &self.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: uniform.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&p.texture),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&p.sampler),
                            },
                        ],
                    });
                    primitives.push(GpuPrimitive {
                        vertices: p.vertices.clone(),
                        indices: p.indices.clone(),
                        count: p.count,
                        uniform,
                        bind,
                        model: p.model,
                        pipeline: p.pipeline,
                        texture: p.texture.clone(),
                        sampler: p.sampler.clone(),
                        material: p.material,
                    });
                }
                self.objects
                    .insert(object.id, (object.path.clone(), primitives));
            }
            let world = super::objects::transform(object);
            let (_, primitives) = self.objects.get_mut(&object.id).unwrap();
            for (p, local) in primitives.iter_mut().zip(template) {
                p.model = world * local.model;
                let highlight = object.highlight.map(|c| c.linear()).unwrap_or([0.; 4]);
                self.queue
                    .write_buffer(&p.uniform, 80, bytemuck::cast_slice(&highlight));
            }
        }
        errors
    }
    pub(super) fn has_objects(&self) -> bool {
        !self.objects.is_empty()
    }
    pub fn set_survey(&mut self, survey: bool) {
        if self.survey == survey {
            return;
        }
        self.survey = survey;
        let value = if survey { 1f32 } else { 0f32 };
        for tile in self.tiles.values() {
            for p in tile {
                self.queue
                    .write_buffer(&p.uniform, 72, bytemuck::bytes_of(&value));
            }
        }
    }
    pub fn remove(&mut self, id: usize) {
        self.tiles.remove(&id);
        if let Some(removed) = self.surfaces.remove(&id) {
            self.surface
                .lock()
                .unwrap()
                .retain(|surface| !Arc::ptr_eq(surface, &removed));
        }
    }
    pub fn render(
        &mut self,
        view: &View,
        draw: &[usize],
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>> {
        *self.surface.lock().unwrap() = draw
            .iter()
            .filter_map(|id| self.surfaces.get(id).cloned())
            .collect();
        let started = Instant::now();
        ensure!(
            width > 0 && height > 0 && width <= 4096 && height <= 4096,
            "Invalid map render dimensions"
        );
        if self
            .target
            .as_ref()
            .is_none_or(|t| t.width != width || t.height != height)
        {
            let desc = wgpu::TextureDescriptor {
                label: Some("map frame"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            };
            let color = self.device.create_texture(&desc);
            let frame_view = color.create_view(&Default::default());
            let depth = self
                .device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("map depth"),
                    format: wgpu::TextureFormat::Depth32Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    ..desc
                })
                .create_view(&Default::default());
            let stride = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("map readback"),
                size: u64::from(stride) * u64::from(height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.target = Some(Target {
                color,
                view: frame_view,
                depth,
                readback,
                width,
                height,
                stride,
            });
        }
        let mut primitives: Vec<_> = draw
            .iter()
            .filter_map(|id| self.tiles.get(id))
            .flatten()
            .collect();
        primitives.extend(self.objects.values().flat_map(|(_, p)| p));
        primitives.sort_by(|a, b| {
            let blend = a.pipeline / 2;
            let other = b.pipeline / 2;
            blend.cmp(&other).then_with(|| {
                if blend == 1 {
                    b.model
                        .w_axis
                        .truncate()
                        .distance_squared(view.eye)
                        .total_cmp(&a.model.w_axis.truncate().distance_squared(view.eye))
                } else {
                    std::cmp::Ordering::Equal
                }
            })
        });
        for p in &primitives {
            let mut relative = p.model;
            relative.w_axis -= view.origin.extend(0.);
            let mvp = (view.view_projection * relative).as_mat4().to_cols_array();
            self.queue
                .write_buffer(&p.uniform, 0, bytemuck::cast_slice(&mvp));
        }
        let target = self.target.as_ref().unwrap();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("map frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("map scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(if self.survey {
                            wgpu::Color {
                                r: 0.00335,
                                g: 0.007,
                                b: 0.0116,
                                a: 1.,
                            }
                        } else {
                            wgpu::Color::WHITE
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            for p in primitives {
                pass.set_pipeline(&self.pipelines[p.pipeline]);
                pass.set_bind_group(0, &p.bind, &[]);
                pass.set_vertex_buffer(0, p.vertices.slice(..));
                pass.set_index_buffer(p.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..p.count, 0, 0..1);
            }
        }
        encoder.copy_texture_to_buffer(
            target.color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &target.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(target.stride),
                    rows_per_image: Some(height),
                },
            },
            target.color.size(),
        );
        let wait_started = Instant::now();
        let submission = self.queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::sync_channel(1);
        target
            .readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .map_err(|_| anyhow!("GPU frame timed out"))?;
        rx.recv_timeout(std::time::Duration::from_secs(1))
            .map_err(|_| anyhow!("GPU readback timed out"))?
            .map_err(|_| anyhow!("GPU readback failed"))?;
        self.metrics.gpu_wait_ms = wait_started.elapsed().as_secs_f64() * 1000.;
        let pack_started = Instant::now();
        let mapped = target.readback.slice(..).get_mapped_range();
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for row in mapped.chunks_exact(target.stride as usize) {
            pixels.extend_from_slice(&row[..width as usize * 4]);
        }
        drop(mapped);
        target.readback.unmap();
        self.metrics.pack_ms = pack_started.elapsed().as_secs_f64() * 1000.;
        self.metrics.render_ms = started.elapsed().as_secs_f64() * 1000.;
        Ok(pixels)
    }
}

#[cfg(test)]
#[path = "../../../../tests/terra-map/unit/maps_renderer_object_tests.rs"]
mod object_tests;

impl Drop for Renderer {
    fn drop(&mut self) {
        self.surface.lock().unwrap().clear();
    }
}
