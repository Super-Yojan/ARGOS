use super::tiles::resolve_url;
use anyhow::{Result, anyhow, ensure};
use base64::Engine;
use bytemuck::{Pod, Zeroable};
use draco_core::{DecodeLimits, DecoderBuffer, Mesh, MeshDecoder};
use glam::{DMat4, DQuat, DVec3};
use image::RgbaImage;
use std::{io::Cursor, sync::Arc};
use url::Url;

const MAX_VERTICES: usize = 1_000_000;
const MAX_DECODED: usize = 128 * 1024 * 1024;
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}
#[derive(Clone, Copy)]
pub struct Sampler {
    pub wrap_s: gltf::texture::WrappingMode,
    pub wrap_t: gltf::texture::WrappingMode,
    pub nearest: bool,
}
impl Default for Sampler {
    fn default() -> Self {
        Self {
            wrap_s: gltf::texture::WrappingMode::Repeat,
            wrap_t: gltf::texture::WrappingMode::Repeat,
            nearest: false,
        }
    }
}
#[derive(Clone)]
pub struct Primitive {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub transform: DMat4,
    pub texture: Arc<RgbaImage>,
    pub sampler: Sampler,
    pub double_sided: bool,
    pub alpha_cutoff: f32,
    pub blend: bool,
}
pub struct TileContent {
    pub primitives: Vec<Primitive>,
    pub copyright: String,
}
impl TileContent {
    pub fn bytes(&self) -> usize {
        self.primitives
            .iter()
            .map(|p| {
                p.vertices.len() * size_of::<Vertex>()
                    + p.indices.len() * 4
                    + p.texture.as_raw().len()
            })
            .sum()
    }
}

fn resource(
    uri: &str,
    source: &Url,
    fetch: &mut impl FnMut(&Url) -> Result<Vec<u8>>,
) -> Result<Vec<u8>> {
    if let Some(data) = uri.strip_prefix("data:") {
        let (meta, data) = data
            .split_once(',')
            .ok_or_else(|| anyhow!("Invalid data URI"))?;
        ensure!(
            meta.ends_with(";base64") && data.len() < 48 * 1024 * 1024,
            "Unsupported or oversized data URI"
        );
        return base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| anyhow!("Invalid base64 resource"));
    }
    if source.scheme() == "file" {
        let url = source.join(uri)?;
        ensure!(
            url.scheme() == "file",
            "Local model resource must use file URLs"
        );
        return fetch(&url);
    }
    let key = source
        .query_pairs()
        .find(|(k, _)| k == "key")
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default();
    fetch(&resolve_url(source, uri, &key)?)
}

pub fn decode(
    bytes: &[u8],
    source: &Url,
    mut fetch: impl FnMut(&Url) -> Result<Vec<u8>>,
) -> Result<TileContent> {
    decode_with_limit(bytes, source, &mut fetch, MAX_DECODED)
}

pub(super) fn decode_with_limit(
    bytes: &[u8],
    source: &Url,
    mut fetch: impl FnMut(&Url) -> Result<Vec<u8>>,
    limit: usize,
) -> Result<TileContent> {
    let limit = limit.min(MAX_DECODED);
    ensure!(bytes.len() <= 32 * 1024 * 1024, "Content exceeds 32 MiB");
    let raw_document: serde_json::Value = if bytes.starts_with(b"glTF") {
        serde_json::from_slice(
            &gltf::binary::Glb::from_slice(bytes)
                .map_err(|_| anyhow!("Invalid GLB header"))?
                .json,
        )?
    } else {
        serde_json::from_slice(bytes).map_err(|_| anyhow!("Invalid glTF JSON"))?
    };
    // gltf-json does not know Draco; enforce required extensions ourselves,
    // then retain its structural validation for the rest of the document.
    let gltf = gltf::Gltf::from_slice_without_validation(bytes)
        .map_err(|_| anyhow!("Invalid glTF/GLB content"))?;
    for ext in gltf.document.extensions_required() {
        ensure!(
            supports_extension(ext),
            "Unsupported required glTF extension: {ext}"
        );
    }
    let mut json = gltf.document.into_json();
    json.extensions_required.clear();
    use gltf::json::validation::{Error, Validate};
    let value = serde_json::to_value(&json)?;
    let mut draco_accessors = std::collections::HashSet::new();
    if let Some(meshes) = value.get("meshes").and_then(|v| v.as_array()) {
        for mesh in meshes {
            if let Some(primitives) = mesh.get("primitives").and_then(|v| v.as_array()) {
                for p in primitives {
                    if p.pointer("/extensions/KHR_draco_mesh_compression")
                        .is_some()
                    {
                        if let Some(attributes) = p.get("attributes").and_then(|v| v.as_object()) {
                            for a in attributes.values() {
                                if let Some(i) = a.as_u64() {
                                    draco_accessors.insert(format!("accessors[{i}].bufferView"));
                                }
                            }
                        }
                        if let Some(i) = p.get("indices").and_then(|v| v.as_u64()) {
                            draco_accessors.insert(format!("accessors[{i}].bufferView"));
                        }
                    }
                }
            }
        }
    }
    let mut invalid = false;
    json.validate(&json, gltf::json::Path::new, &mut |path, error| {
        if !(error == Error::Missing && draco_accessors.contains(path().as_str())) {
            invalid = true;
        }
    });
    ensure!(!invalid, "Invalid glTF structure");
    let doc = gltf::Document::from_json_without_validation(json);
    let mut buffers = Vec::new();
    let mut allocated = 0usize;
    for buffer in doc.buffers() {
        ensure!(buffer.length() <= 32 * 1024 * 1024, "glTF buffer too large");
        ensure!(
            buffer.length() <= limit.saturating_sub(allocated),
            "Model buffer allocation limit exceeded"
        );
        let data = match buffer.source() {
            gltf::buffer::Source::Bin => gltf
                .blob
                .clone()
                .ok_or_else(|| anyhow!("Missing GLB binary chunk"))?,
            gltf::buffer::Source::Uri(uri) => resource(uri, source, &mut fetch)?,
        };
        ensure!(data.len() >= buffer.length(), "Truncated glTF buffer");
        allocated += data.len();
        ensure!(allocated <= limit, "Tile allocation limit exceeded");
        buffers.push(data);
    }
    // Validate byte ranges before gltf's typed readers borrow them.
    for view in doc.views() {
        ensure!(
            view.offset()
                .checked_add(view.length())
                .is_some_and(|end| end <= buffers[view.buffer().index()].len()),
            "Invalid buffer view range"
        );
    }
    for a in doc.accessors() {
        ensure!(
            a.count() <= MAX_VERTICES * 3,
            "Accessor count limit exceeded"
        );
        if let Some(v) = a.view() {
            let stride = v.stride().unwrap_or(a.size());
            let span = a
                .count()
                .saturating_sub(1)
                .checked_mul(stride)
                .and_then(|n| n.checked_add(a.size()))
                .and_then(|n| n.checked_add(a.offset()));
            ensure!(
                a.count() == 0 || span.is_some_and(|n| n <= v.length()),
                "Accessor extends beyond its buffer view"
            );
        }
        if let Some(sparse) = a.sparse() {
            let indices = sparse.indices();
            let values = sparse.values();
            ensure!(sparse.count() <= a.count(), "Invalid sparse accessor count");
            ensure!(
                indices
                    .offset()
                    .saturating_add(sparse.count().saturating_mul(indices.index_type().size()))
                    <= indices.view().length(),
                "Invalid sparse index range"
            );
            ensure!(
                values
                    .offset()
                    .saturating_add(sparse.count().saturating_mul(a.size()))
                    <= values.view().length(),
                "Invalid sparse value range"
            );
        }
    }
    let mut images = Vec::new();
    for image in doc.images() {
        let bytes = match image.source() {
            gltf::image::Source::View { view, .. } => buffers[view.buffer().index()]
                [view.offset()..view.offset() + view.length()]
                .to_vec(),
            gltf::image::Source::Uri { uri, .. } => resource(uri, source, &mut fetch)?,
        };
        let (width, height) = image::ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()?
            .into_dimensions()?;
        let decoded_size = u64::from(width)
            .saturating_mul(u64::from(height))
            .saturating_mul(4);
        ensure!(
            decoded_size <= limit.saturating_sub(allocated) as u64,
            "Model image allocation limit exceeded"
        );
        let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(limit.saturating_sub(allocated).min(64 * 1024 * 1024) as u64);
        reader.limits(limits);
        let rgba = reader
            .decode()
            .map_err(|_| anyhow!("Invalid or oversized tile image"))?
            .into_rgba8();
        allocated += rgba.as_raw().len();
        ensure!(allocated <= limit, "Tile image allocation limit exceeded");
        images.push(Arc::new(rgba));
    }
    let rtc = doc
        .extension_value("CESIUM_RTC")
        .and_then(|v| v.get("center"))
        .and_then(|v| v.as_array())
        .filter(|a| a.len() == 3)
        .map(|a| {
            DVec3::new(
                a[0].as_f64().unwrap_or(0.),
                a[1].as_f64().unwrap_or(0.),
                a[2].as_f64().unwrap_or(0.),
            )
        })
        .unwrap_or(DVec3::ZERO);
    let base = DMat4::from_translation(rtc) * DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2);
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .ok_or_else(|| anyhow!("glTF has no scene"))?;
    let mut stack: Vec<_> = scene.nodes().map(|node| (node, base, 0usize)).collect();
    let mut primitives = Vec::new();
    let mut visits = 0;
    while let Some((node, parent, depth)) = stack.pop() {
        visits += 1;
        ensure!(
            visits < 10_000 && depth < 128,
            "glTF scene hierarchy limit exceeded"
        );
        let transform = parent * raw_node_transform(&raw_document["nodes"][node.index()])?;
        ensure!(transform.is_finite(), "Non-finite glTF transform");
        for child in node.children() {
            stack.push((child, transform, depth + 1));
        }
        if let Some(mesh) = node.mesh() {
            for p in mesh.primitives() {
                ensure!(
                    p.mode() == gltf::mesh::Mode::Triangles,
                    "Only triangle mesh tiles are supported"
                );
                let reader = p.reader(|b| Some(buffers[b.index()].as_slice()));
                let material = p.material();
                let pbr = material.pbr_metallic_roughness();
                let factor = pbr.base_color_factor();
                let texture_info = pbr.base_color_texture();
                let uv_extension = texture_info
                    .as_ref()
                    .and_then(|t| t.extension_value("KHR_texture_transform"));
                let uv_set = uv_extension
                    .and_then(|v| v.get("texCoord"))
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32)
                    .or_else(|| texture_info.as_ref().map(|t| t.tex_coord()))
                    .unwrap_or(0);
                let vertex_count = p
                    .get(&gltf::Semantic::Positions)
                    .map(|a| a.count())
                    .ok_or_else(|| anyhow!("Model POSITION missing"))?;
                let index_count = p.indices().map(|a| a.count()).unwrap_or(vertex_count);
                let geometry_bytes = vertex_count
                    .saturating_mul(size_of::<Vertex>())
                    .saturating_add(index_count.saturating_mul(4));
                ensure!(
                    geometry_bytes <= limit.saturating_sub(allocated),
                    "Model geometry allocation limit exceeded"
                );
                let mut positions: Vec<[f32; 3]>;
                let mut uvs: Vec<[f32; 2]>;
                let colors: Vec<[f32; 4]>;
                let indices: Vec<u32>;
                if let Some(ext) = p.extension_value("KHR_draco_mesh_compression") {
                    let view_id = ext
                        .get("bufferView")
                        .and_then(|v| v.as_u64())
                        .ok_or_else(|| anyhow!("Missing Draco buffer view"))?
                        as usize;
                    let view = doc
                        .views()
                        .nth(view_id)
                        .ok_or_else(|| anyhow!("Invalid Draco buffer view"))?;
                    let data = &buffers[view.buffer().index()]
                        [view.offset()..view.offset() + view.length()];
                    let limits = DecodeLimits::default()
                        .with_max_points(
                            MAX_VERTICES.min(limit.saturating_sub(allocated) / size_of::<Vertex>())
                                as u64,
                        )
                        .with_max_faces(2_000_000)
                        .with_max_decoded_bytes(
                            limit.saturating_sub(allocated).min(64 * 1024 * 1024) as u64,
                        );
                    let mut input = DecoderBuffer::new(data).with_limits(limits);
                    let mut mesh = Mesh::new();
                    MeshDecoder::new()
                        .decode(&mut input, &mut mesh)
                        .map_err(|_| anyhow!("Draco mesh decoding failed"))?;
                    let attributes = ext
                        .get("attributes")
                        .ok_or_else(|| anyhow!("Missing Draco attributes"))?;
                    let attribute = |name: &str, components: usize| -> Result<Option<Vec<f32>>> {
                        let Some(id) = attributes.get(name).and_then(|v| v.as_u64()) else {
                            return Ok(None);
                        };
                        let a = mesh
                            .attribute_by_unique_id(id as u32)
                            .ok_or_else(|| anyhow!("Missing Draco attribute {name}"))?;
                        ensure!(
                            a.num_components() as usize >= components,
                            "Invalid Draco attribute dimensions"
                        );
                        let mut values = a.read_f32s(mesh.num_points(), components);
                        let semantic = match name {
                            "POSITION" => gltf::Semantic::Positions,
                            "COLOR_0" => gltf::Semantic::Colors(0),
                            _ => gltf::Semantic::TexCoords(uv_set),
                        };
                        let normalized = p
                            .get(&semantic)
                            .is_some_and(|accessor| accessor.normalized());
                        if normalized || a.normalized() {
                            let divisor = match a.data_type() {
                                draco_core::DataType::Uint8 => 255.,
                                draco_core::DataType::Uint16 => 65535.,
                                _ => 1.,
                            };
                            for v in &mut values {
                                *v /= divisor;
                            }
                        }
                        Ok(Some(values))
                    };
                    positions = attribute("POSITION", 3)?
                        .ok_or_else(|| anyhow!("Draco POSITION missing"))?
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .map(|p| [p[0], p[1], p[2]])
                        .collect();
                    uvs = attribute(&format!("TEXCOORD_{uv_set}"), 2)?
                        .unwrap_or_default()
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|p| [p[0], p[1]])
                        .collect();
                    let color_components = p
                        .get(&gltf::Semantic::Colors(0))
                        .map(|a| a.dimensions().multiplicity())
                        .unwrap_or(4);
                    ensure!(
                        color_components == 3 || color_components == 4,
                        "Invalid color dimensions"
                    );
                    colors = attribute("COLOR_0", color_components)?
                        .unwrap_or_default()
                        .chunks_exact(color_components)
                        .map(|p| {
                            [
                                p[0],
                                p[1],
                                p[2],
                                if color_components == 4 { p[3] } else { 1. },
                            ]
                        })
                        .collect();
                    indices = mesh
                        .faces()
                        .iter()
                        .flat_map(|face| face.iter().map(|p| p.0))
                        .collect();
                } else {
                    positions = reader
                        .read_positions()
                        .ok_or_else(|| anyhow!("Tile POSITION missing"))?
                        .collect();
                    uvs = reader
                        .read_tex_coords(uv_set)
                        .map(|v| v.into_f32().collect())
                        .unwrap_or_default();
                    colors = reader
                        .read_colors(0)
                        .map(|v| v.into_rgba_f32().collect())
                        .unwrap_or_default();
                    indices = reader
                        .read_indices()
                        .map(|i| i.into_u32().collect())
                        .unwrap_or_else(|| (0..positions.len() as u32).collect());
                }
                ensure!(
                    !positions.is_empty()
                        && positions.len() <= MAX_VERTICES
                        && indices.len() <= 6_000_000
                        && indices.len().is_multiple_of(3),
                    "Tile geometry limit exceeded"
                );
                ensure!(
                    indices.iter().all(|&i| (i as usize) < positions.len()),
                    "Tile index out of range"
                );
                ensure!(
                    uvs.is_empty() || uvs.len() == positions.len(),
                    "Tile UV count mismatch"
                );
                ensure!(
                    colors.is_empty() || colors.len() == positions.len(),
                    "Tile color count mismatch"
                );
                ensure!(
                    texture_info.is_none() || !uvs.is_empty(),
                    "Textured tile has no matching UV set"
                );
                if let Some(ext) = uv_extension {
                    let vec2 = |name: &str, default: [f64; 2]| {
                        ext.get(name)
                            .and_then(|v| v.as_array())
                            .filter(|v| v.len() == 2)
                            .map(|a| {
                                [
                                    a[0].as_f64().unwrap_or(default[0]),
                                    a[1].as_f64().unwrap_or(default[1]),
                                ]
                            })
                            .unwrap_or(default)
                    };
                    let offset = vec2("offset", [0., 0.]);
                    let scale = vec2("scale", [1., 1.]);
                    let r = ext.get("rotation").and_then(|v| v.as_f64()).unwrap_or(0.);
                    for uv in &mut uvs {
                        let (x, y) = (uv[0] as f64 * scale[0], uv[1] as f64 * scale[1]);
                        *uv = [
                            (r.cos() * x - r.sin() * y + offset[0]) as f32,
                            (r.sin() * x + r.cos() * y + offset[1]) as f32,
                        ];
                    }
                }
                ensure!(
                    positions
                        .len()
                        .saturating_mul(size_of::<Vertex>())
                        .saturating_add(indices.len().saturating_mul(4))
                        <= limit.saturating_sub(allocated),
                    "Model geometry allocation limit exceeded"
                );
                let vertices: Vec<_> = positions
                    .drain(..)
                    .enumerate()
                    .map(|(i, position)| {
                        let c = colors.get(i).copied().unwrap_or([1.; 4]);
                        Vertex {
                            position,
                            uv: uvs.get(i).copied().unwrap_or([0.; 2]),
                            color: std::array::from_fn(|j| c[j] * factor[j]),
                        }
                    })
                    .collect();
                ensure!(
                    vertices.iter().all(|v| v
                        .position
                        .iter()
                        .chain(&v.uv)
                        .chain(&v.color)
                        .all(|x| x.is_finite())),
                    "Non-finite vertex data"
                );
                let (texture, sampler) = if let Some(info) = texture_info {
                    let t = info.texture();
                    let s = t.sampler();
                    (
                        images[t.source().index()].clone(),
                        Sampler {
                            wrap_s: s.wrap_s(),
                            wrap_t: s.wrap_t(),
                            nearest: s.mag_filter() == Some(gltf::texture::MagFilter::Nearest),
                        },
                    )
                } else {
                    (
                        Arc::new(RgbaImage::from_pixel(1, 1, image::Rgba([255; 4]))),
                        Sampler::default(),
                    )
                };
                allocated += vertices.len() * size_of::<Vertex>() + indices.len() * 4;
                ensure!(
                    allocated <= limit && primitives.len() < 2048,
                    "Tile decoded allocation limit exceeded"
                );
                let alpha_cutoff = if material.alpha_mode() == gltf::material::AlphaMode::Mask {
                    material.alpha_cutoff().unwrap_or(0.5)
                } else {
                    -1.
                };
                primitives.push(Primitive {
                    vertices,
                    indices,
                    transform,
                    texture,
                    sampler,
                    double_sided: material.double_sided(),
                    alpha_cutoff,
                    blend: material.alpha_mode() == gltf::material::AlphaMode::Blend,
                });
            }
        }
    }
    ensure!(
        !primitives.is_empty(),
        "Tile contains no triangle primitives"
    );
    let copyright = doc.as_json().asset.copyright.clone().unwrap_or_default();
    Ok(TileContent {
        primitives,
        copyright,
    })
}

/// gltf's node API uses f32; read the original JSON before precision is lost.
fn raw_node_transform(node: &serde_json::Value) -> Result<DMat4> {
    fn array<const N: usize>(
        value: Option<&serde_json::Value>,
        fallback: [f64; N],
    ) -> Result<[f64; N]> {
        let Some(value) = value else {
            return Ok(fallback);
        };
        let values = value
            .as_array()
            .filter(|a| a.len() == N)
            .ok_or_else(|| anyhow!("Invalid node transform dimensions"))?;
        let mut output = fallback;
        for (i, value) in values.iter().enumerate() {
            output[i] = value
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| anyhow!("Non-finite node transform"))?;
        }
        Ok(output)
    }
    if let Some(matrix) = node.get("matrix") {
        return Ok(DMat4::from_cols_array(&array(
            Some(matrix),
            DMat4::IDENTITY.to_cols_array(),
        )?));
    }
    let translation = DVec3::from_array(array(node.get("translation"), [0.; 3])?);
    let scale = DVec3::from_array(array(node.get("scale"), [1.; 3])?);
    let rotation = DQuat::from_array(array(node.get("rotation"), [0., 0., 0., 1.])?);
    ensure!(
        (rotation.length_squared() - 1.).abs() < 0.01,
        "Invalid node quaternion"
    );
    Ok(DMat4::from_scale_rotation_translation(
        scale,
        rotation.normalize(),
        translation,
    ))
}

pub(super) fn supports_extension(name: &str) -> bool {
    matches!(
        name,
        "KHR_draco_mesh_compression"
            | "KHR_materials_unlit"
            | "KHR_texture_transform"
            | "CESIUM_RTC"
    )
}
