use super::camera::{Sphere, View, region_sphere};
use anyhow::{Result, anyhow, bail, ensure};
use glam::{DMat4, DVec3};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    time::{Duration, Instant},
};
use url::Url;

pub const ROOT_URL: &str = "https://tile.googleapis.com/v1/3dtiles/root.json";
pub const MAX_RESPONSE: u64 = 32 * 1024 * 1024;

pub fn resolve_url(parent: &Url, link: &str, key: &str) -> Result<Url> {
    let mut url = parent.join(link).map_err(|_| anyhow!("Invalid tile URL"))?;
    ensure!(
        url.origin() == parent.origin(),
        "Cross-origin tile links are unsupported"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Credentials in tile URLs are unsupported"
    );
    let mut pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| k != "key")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    for (k, v) in parent.query_pairs() {
        if k != "key" && !pairs.iter().any(|(name, _)| name == &k) {
            pairs.push((k.into_owned(), v.into_owned()));
        }
    }
    if !key.is_empty() {
        pairs.push(("key".into(), key.into()));
    }
    url.set_query(None);
    if !pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(pairs);
    }
    Ok(url)
}

#[derive(Deserialize)]
struct Tileset {
    root: RawTile,
    #[serde(default, rename = "extensionsRequired")]
    required: Vec<String>,
    #[serde(default)]
    extensions: HashMap<String, serde_json::Value>,
}
#[derive(Deserialize)]
struct RawTile {
    #[serde(rename = "boundingVolume")]
    bounds: RawBounds,
    #[serde(default, rename = "geometricError")]
    error: f64,
    transform: Option<[f64; 16]>,
    refine: Option<String>,
    content: Option<RawContent>,
    #[serde(default)]
    children: Vec<RawTile>,
    #[serde(default, rename = "implicitTiling")]
    implicit: Option<serde_json::Value>,
    #[serde(default)]
    contents: Vec<serde_json::Value>,
}
#[derive(Deserialize)]
struct RawContent {
    uri: Option<String>,
    url: Option<String>,
}
#[derive(Deserialize)]
struct RawBounds {
    sphere: Option<[f64; 4]>,
    #[serde(rename = "box")]
    bbox: Option<[f64; 12]>,
    region: Option<[f64; 6]>,
}

pub struct Node {
    pub transform: DMat4,
    pub bounds: Sphere,
    pub error: f64,
    pub additive: bool,
    pub content: Option<Url>,
    pub children: Vec<usize>,
}
pub struct TileTree {
    pub nodes: Vec<Node>,
}
pub struct Selection {
    pub draw: Vec<usize>,
    pub requests: Vec<usize>,
    pub keep: HashSet<usize>,
}

pub fn replacement_ids(
    parent: Option<usize>,
    children: Vec<(bool, Vec<usize>)>,
    additive: bool,
) -> (bool, Vec<usize>) {
    let all_ready = children.iter().all(|c| c.0);
    let mut draw = Vec::new();
    if additive || !all_ready {
        draw.extend(parent);
    }
    // A partial REPLACE subtree would overlap the fallback and cause seams.
    if additive || all_ready || parent.is_none() {
        for (_, ids) in children {
            draw.extend(ids);
        }
    }
    (parent.is_some() || all_ready, draw)
}

impl TileTree {
    pub fn parse(bytes: &[u8], base: Url) -> Result<Self> {
        let mut tree = Self { nodes: Vec::new() };
        tree.append(bytes, &base, DMat4::IDENTITY, false)?;
        Ok(tree)
    }
    fn append(
        &mut self,
        bytes: &[u8],
        base: &Url,
        transform: DMat4,
        additive: bool,
    ) -> Result<usize> {
        let raw: Tileset =
            serde_json::from_slice(bytes).map_err(|_| anyhow!("Invalid tileset JSON"))?;
        ensure!(
            raw.required.iter().all(|ext| ext == "3DTILES_content_gltf"),
            "Unsupported required tileset extensions: {}",
            raw.required.join(", ")
        );
        if let Some(required) = raw
            .extensions
            .get("3DTILES_content_gltf")
            .and_then(|v| v.get("extensionsRequired"))
            .and_then(|v| v.as_array())
        {
            for extension in required {
                let name = extension
                    .as_str()
                    .ok_or_else(|| anyhow!("Invalid required glTF extension name"))?;
                ensure!(
                    super::content::supports_extension(name),
                    "Unsupported required glTF extension: {name}"
                );
            }
        }
        self.add(raw.root, base, transform, additive, 0)
    }
    fn add(
        &mut self,
        raw: RawTile,
        base: &Url,
        parent: DMat4,
        additive: bool,
        depth: usize,
    ) -> Result<usize> {
        ensure!(
            depth < 100 && self.nodes.len() < 100_000,
            "Tileset hierarchy limit exceeded"
        );
        ensure!(
            raw.implicit.is_none() && raw.contents.is_empty(),
            "Implicit tiling and multiple contents are unsupported"
        );
        let transform = parent
            * raw
                .transform
                .map(|a| DMat4::from_cols_array(&a))
                .unwrap_or(DMat4::IDENTITY);
        ensure!(
            transform.is_finite() && raw.error.is_finite() && raw.error >= 0.,
            "Invalid tile transform or geometric error"
        );
        let additive = match raw.refine.as_deref() {
            Some("ADD") => true,
            Some("REPLACE") => false,
            None => additive,
            _ => bail!("Unknown tile refinement"),
        };
        let axes = [
            transform.x_axis.truncate(),
            transform.y_axis.truncate(),
            transform.z_axis.truncate(),
        ];
        let lengths = axes.map(|v| v.length());
        // TRS transforms have orthogonal columns: their maximum scale is the
        // longest column. Use a conservative bound if composition adds shear.
        let orthogonal = [(0, 1), (0, 2), (1, 2)]
            .iter()
            .all(|&(i, j)| axes[i].dot(axes[j]).abs() <= 1e-12 * (lengths[i] * lengths[j]).max(1.));
        let scale = if orthogonal {
            lengths.into_iter().fold(0f64, f64::max)
        } else {
            lengths.iter().map(|v| v * v).sum::<f64>().sqrt()
        };
        let bounds = if let Some(r) = raw.bounds.region {
            region_sphere(r)
        } else {
            let sphere = if let Some(s) = raw.bounds.sphere {
                Sphere {
                    center: DVec3::new(s[0], s[1], s[2]),
                    radius: s[3],
                }
            } else if let Some(b) = raw.bounds.bbox {
                Sphere {
                    center: DVec3::from_slice(&b[..3]),
                    radius: (0..8)
                        .map(|corner| {
                            let axes = [
                                DVec3::from_slice(&b[3..6]),
                                DVec3::from_slice(&b[6..9]),
                                DVec3::from_slice(&b[9..12]),
                            ];
                            axes.iter()
                                .enumerate()
                                .map(|(i, a)| *a * if corner & (1 << i) == 0 { -1. } else { 1. })
                                .sum::<DVec3>()
                                .length()
                        })
                        .fold(0f64, f64::max),
                }
            } else {
                bail!("Missing tile bounding volume");
            };
            Sphere {
                center: transform.transform_point3(sphere.center),
                radius: sphere.radius * scale,
            }
        };
        ensure!(
            bounds.center.is_finite() && bounds.radius.is_finite() && bounds.radius >= 0.,
            "Invalid tile bounds"
        );
        let key = base
            .query_pairs()
            .find(|(k, _)| k == "key")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default();
        let content = raw
            .content
            .map(|c| {
                resolve_url(
                    base,
                    &c.uri
                        .or(c.url)
                        .ok_or_else(|| anyhow!("Missing content URI"))?,
                    &key,
                )
            })
            .transpose()?;
        let id = self.nodes.len();
        self.nodes.push(Node {
            transform,
            bounds,
            error: raw.error * scale,
            additive,
            content,
            children: Vec::new(),
        });
        let mut children = Vec::new();
        for child in raw.children {
            children.push(self.add(child, base, transform, additive, depth + 1)?);
        }
        self.nodes[id].children = children;
        Ok(id)
    }
    pub fn expand(&mut self, id: usize, bytes: &[u8]) -> Result<()> {
        let n = &self.nodes[id];
        let base = n
            .content
            .clone()
            .ok_or_else(|| anyhow!("Tile already expanded"))?;
        let (transform, additive) = (n.transform, n.additive);
        let start = self.nodes.len();
        match self.append(bytes, &base, transform, additive) {
            Ok(root) => {
                self.nodes[id].children.push(root);
                self.nodes[id].content = None;
                Ok(())
            }
            Err(e) => {
                self.nodes.truncate(start);
                Err(e)
            }
        }
    }
    pub fn select(&self, view: &View, ready: &HashSet<usize>, threshold: f64) -> Selection {
        let mut selection = Selection {
            draw: Vec::new(),
            requests: Vec::new(),
            keep: HashSet::new(),
        };
        let (_, draw) = self.visit(0, view, ready, threshold, &mut selection, 0);
        selection.draw = draw;
        selection
            .requests
            .sort_by(|a, b| self.priority(*b, view).total_cmp(&self.priority(*a, view)));
        selection.requests.dedup();
        selection
    }
    fn priority(&self, id: usize, view: &View) -> f64 {
        let n = &self.nodes[id];
        view.error(n.bounds, n.error.max(1.))
    }
    fn visit(
        &self,
        id: usize,
        view: &View,
        ready: &HashSet<usize>,
        threshold: f64,
        selection: &mut Selection,
        depth: usize,
    ) -> (bool, Vec<usize>) {
        let n = &self.nodes[id];
        if !view.visible(n.bounds) {
            return (true, Vec::new());
        }
        let parent = ready.contains(&id).then_some(id);
        if parent.is_some() {
            selection.keep.insert(id);
        }
        let refine = parent.is_none() || view.error(n.bounds, n.error) > threshold;
        if n.content.is_some() && parent.is_none() {
            selection.requests.push(id);
        }
        if refine && !n.children.is_empty() && depth < 128 {
            let children = n
                .children
                .iter()
                .map(|c| self.visit(*c, view, ready, threshold, selection, depth + 1))
                .collect();
            replacement_ids(parent, children, n.additive)
        } else {
            (
                parent.is_some() || n.content.is_none(),
                parent.into_iter().collect(),
            )
        }
    }
}

#[derive(Clone)]
pub struct HttpClient {
    client: reqwest::blocking::Client,
}
pub struct Response {
    pub bytes: Vec<u8>,
    pub cache_for: Duration,
}
impl HttpClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(25))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("learn-gpui-native-maps/0.1")
                .build()?,
        })
    }
    pub fn get(&self, url: &Url) -> Result<Response> {
        for attempt in 0..3 {
            let response = self.client.get(url.clone()).send().map_err(|e| {
                anyhow!(
                    "Tile request failed: {}",
                    if e.is_timeout() {
                        "timeout"
                    } else {
                        "network error"
                    }
                )
            })?;
            let status = response.status();
            if (status.as_u16() == 429 || status.is_server_error()) && attempt < 2 {
                let delay = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(1 << attempt)
                    .min(30);
                std::thread::sleep(Duration::from_secs(delay));
                continue;
            }
            ensure!(
                status.is_success(),
                "Map Tiles API returned HTTP {}{}",
                status.as_u16(),
                if status.as_u16() == 403 {
                    "; check that Map Tiles API and billing are enabled and the key allows this application"
                } else {
                    ""
                }
            );
            ensure!(
                response.content_length().unwrap_or(0) <= MAX_RESPONSE,
                "Tile response exceeds 32 MiB"
            );
            let control = response
                .headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let mut cache_for = Duration::from_secs(300);
            for part in control.split(',').map(str::trim) {
                if part == "no-store" || part == "no-cache" {
                    cache_for = Duration::ZERO;
                    break;
                }
                if let Some(age) = part
                    .strip_prefix("max-age=")
                    .and_then(|v| v.trim_matches('"').parse::<u64>().ok())
                {
                    cache_for = Duration::from_secs(age.min(10_800));
                }
            }
            let mut bytes = Vec::new();
            response
                .take(MAX_RESPONSE + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| anyhow!("Failed to read tile body"))?;
            ensure!(
                bytes.len() as u64 <= MAX_RESPONSE,
                "Tile response exceeds 32 MiB"
            );
            return Ok(Response { bytes, cache_for });
        }
        bail!("Tile request retry limit reached")
    }
}

pub fn copyrights<'a>(sources: impl Iterator<Item = &'a str>) -> String {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for source in sources {
        for item in source.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            *counts.entry(item).or_default() += 1;
        }
    }
    let mut items: Vec<_> = counts.into_iter().collect();
    items.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    items
        .into_iter()
        .map(|(s, _)| s)
        .collect::<Vec<_>>()
        .join("; ")
}

pub struct CacheEntry {
    pub bytes: usize,
    pub used: u64,
    pub expires: Instant,
}
