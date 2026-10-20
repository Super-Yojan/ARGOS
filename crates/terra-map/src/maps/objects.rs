use super::{
    api::{Map, ObjectRecord, ObjectStatus},
    camera::ecef,
    content::{self, TileContent},
    renderer::Renderer,
};
use anyhow::{Result, anyhow, ensure};
use crossbeam_channel::{Receiver, Sender, bounded};
use glam::{DMat4, DVec3};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use url::Url;

pub(super) fn transform(object: &ObjectRecord) -> DMat4 {
    let p = object.position;
    let (lat, lon) = (p.latitude.to_radians(), p.longitude.to_radians());
    let up = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
    let east = DVec3::new(-lon.sin(), lon.cos(), 0.);
    let north = up.cross(east);
    DMat4::from_cols(
        east.extend(0.),
        north.extend(0.),
        up.extend(0.),
        ecef(p.latitude, p.longitude, p.altitude).extend(1.),
    ) * DMat4::from_rotation_z(-object.heading.to_radians())
        * DMat4::from_scale(DVec3::from_array(object.axis_scale) * object.scale)
}
fn read_limited(path: &Path, budget: &mut usize) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)
        .map_err(|_| anyhow!("Cannot open model resource: {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take((*budget as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= *budget, "Model resources exceed byte budget");
    *budget -= bytes.len();
    Ok(bytes)
}
pub(super) fn load(path: &Path, limit: usize) -> Result<Arc<TileContent>> {
    let path = path
        .canonicalize()
        .map_err(|_| anyhow!("Model file does not exist: {}", path.display()))?;
    let root = path
        .parent()
        .ok_or_else(|| anyhow!("Model needs a parent directory"))?;
    let mut budget = limit.min(32 * 1024 * 1024);
    let bytes = read_limited(&path, &mut budget)?;
    let source = Url::from_file_path(&path).map_err(|_| anyhow!("Invalid local model path"))?;
    let content = content::decode_with_limit(
        &bytes,
        &source,
        |url| {
            ensure!(
                url.scheme() == "file",
                "Local models may only reference local resources"
            );
            let path = url
                .to_file_path()
                .map_err(|_| anyhow!("Invalid model resource path"))?
                .canonicalize()?;
            ensure!(
                path.starts_with(root),
                "Model resource escapes its directory"
            );
            read_limited(&path, &mut budget)
        },
        limit,
    )?;
    ensure!(
        content.bytes() <= limit,
        "Decoded model exceeds MapConfig.max_model_bytes"
    );
    ensure!(
        !content.primitives.is_empty(),
        "Model contains no renderable triangles"
    );
    Ok(Arc::new(content))
}
struct Job {
    path: PathBuf,
    limit: usize,
}
struct Loaded {
    requested: PathBuf,
    result: Result<(PathBuf, Arc<TileContent>)>,
}
/// One bounded decoder, shared assets, no filesystem work on the UI/render thread.
pub(super) struct Objects {
    map: Option<Map>,
    revision: u64,
    scene_revision: u64,
    records: Arc<Vec<ObjectRecord>>,
    models: HashMap<PathBuf, Arc<TileContent>>,
    aliases: HashMap<PathBuf, PathBuf>,
    errors: HashMap<PathBuf, String>,
    pending: HashSet<PathBuf>,
    jobs: Sender<Job>,
    results: Receiver<Loaded>,
    stop: Arc<AtomicBool>,
    limit: usize,
}
impl Objects {
    pub fn new(map: Option<Map>, limit: usize) -> Self {
        let (jobs, input) = bounded::<Job>(1);
        let (output, results) = bounded::<Loaded>(1);
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        thread::spawn(move || {
            let mut cache: HashMap<PathBuf, std::sync::Weak<TileContent>> = HashMap::new();
            while !cancelled.load(Ordering::Relaxed) {
                let job = match input.recv_timeout(Duration::from_millis(50)) {
                    Ok(job) => job,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                let result = (|| {
                    let canonical = job.path.canonicalize().map_err(|_| {
                        anyhow!("Model file does not exist: {}", job.path.display())
                    })?;
                    let content =
                        if let Some(content) = cache.get(&canonical).and_then(|v| v.upgrade()) {
                            content
                        } else {
                            let content = load(&canonical, job.limit)?;
                            cache.insert(canonical.clone(), Arc::downgrade(&content));
                            content
                        };
                    cache.retain(|_, v| v.strong_count() > 0);
                    Ok((canonical, content))
                })();
                let mut result = Loaded {
                    result,
                    requested: job.path,
                };
                loop {
                    if cancelled.load(Ordering::Relaxed) {
                        return;
                    }
                    match output.send_timeout(result, Duration::from_millis(50)) {
                        Ok(()) => break,
                        Err(crossbeam_channel::SendTimeoutError::Timeout(value)) => result = value,
                        Err(_) => return,
                    }
                }
            }
        });
        Self {
            map,
            revision: 0,
            scene_revision: 0,
            records: Arc::new(Vec::new()),
            models: HashMap::new(),
            aliases: HashMap::new(),
            errors: HashMap::new(),
            pending: HashSet::new(),
            jobs,
            results,
            stop,
            limit,
        }
    }
    pub fn poll(&mut self, renderer: &mut Renderer) -> Result<bool> {
        let Some(map) = &self.map else {
            return Ok(false);
        };
        let mut changed = false;
        if map.revision() != self.revision {
            let snapshot = map.snapshot();
            self.revision = snapshot.revision;
            if snapshot.scene_revision != self.scene_revision {
                self.scene_revision = snapshot.scene_revision;
                self.records = snapshot.objects;
                changed = true;
            }
        }
        if !changed && self.pending.is_empty() {
            return Ok(false);
        }
        let wanted: HashSet<_> = self.records.iter().map(|o| o.path.clone()).collect();
        self.aliases.retain(|path, _| wanted.contains(path));
        let canonical_wanted: HashSet<_> = self.aliases.values().cloned().collect();
        self.models
            .retain(|path, _| canonical_wanted.contains(path));
        self.errors.retain(|path, _| wanted.contains(path));
        while let Ok(loaded) = self.results.try_recv() {
            self.pending.remove(&loaded.requested);
            if wanted.contains(&loaded.requested) {
                match loaded.result {
                    Ok((canonical, content)) => {
                        self.aliases.insert(loaded.requested, canonical.clone());
                        self.models.insert(canonical, content);
                    }
                    Err(error) => {
                        self.errors.insert(loaded.requested, error.to_string());
                    }
                }
            }
            changed = true;
        }
        // Reserve all remaining decoded capacity for a single outstanding decode.
        // No second result can allocate against a stale copy of the budget.
        if self.pending.is_empty()
            && let Some(object) = self
                .records
                .iter()
                .find(|o| !self.aliases.contains_key(&o.path) && !self.errors.contains_key(&o.path))
        {
            let used: usize = self.models.values().map(|c| c.bytes()).sum();
            let job = Job {
                path: object.path.clone(),
                limit: self.limit.saturating_sub(used),
            };
            if self.jobs.try_send(job).is_ok() {
                self.pending.insert(object.path.clone());
            }
        }
        if changed {
            let records: Vec<_> = self
                .records
                .iter()
                .filter_map(|o| {
                    let path = self.aliases.get(&o.path)?;
                    let mut record = o.clone();
                    record.path = path.clone();
                    Some(record)
                })
                .collect();
            for (path, error) in renderer.sync_objects(&records, &self.models) {
                self.models.remove(&path);
                for (alias, canonical) in &self.aliases {
                    if *canonical == path {
                        self.errors.insert(alias.clone(), error.clone());
                    }
                }
            }
            for object in self.records.iter() {
                if let Some(error) = self.errors.get(&object.path) {
                    map.report(object.id, ObjectStatus::Failed(error.clone()));
                } else if self
                    .aliases
                    .get(&object.path)
                    .is_some_and(|p| self.models.contains_key(p))
                {
                    map.report(object.id, ObjectStatus::Ready);
                } else {
                    map.report(object.id, ObjectStatus::Loading);
                }
            }
        }
        Ok(changed)
    }
}
impl Drop for Objects {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{MapConfig, api::GeoPosition};
    use super::*;
    #[test]
    fn static_local_model_and_enu_placement() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/maps/fixtures/textured-quad.glb");
        let content = load(&path, 8 * 1024 * 1024).unwrap();
        assert!(!content.primitives.is_empty());
        assert!(load(&path, 1).is_err());
        let map = Map::new(MapConfig::default()).unwrap();
        let id = map
            .add_3d_obj(GeoPosition::new(0., 0., 100.), path)
            .unwrap();
        let snapshot = map.snapshot();
        let world = transform(&snapshot.objects[0]);
        assert_eq!(world.w_axis.truncate(), ecef(0., 0., 100.));
        assert!(world.transform_vector3(DVec3::X).distance(DVec3::Y) < 1e-9);
        map.set_object_heading(id, 90.).unwrap();
        let world = transform(&map.snapshot().objects[0]);
        assert!(world.transform_vector3(DVec3::X).distance(-DVec3::Z) < 1e-9);
    }
    #[test]
    fn gltf_external_resources_resolve_locally() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/maps/fixtures/rgb-quad.gltf");
        assert!(load(&path, 8 * 1024 * 1024).is_ok());
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::super::{
        MapConfig,
        api::{Color, GeoPosition},
    };
    use super::*;
    #[test]
    #[ignore = "Requires a native GPU"]
    fn gpu_async_models_report_failures_and_do_not_resurrect_removed_ids() {
        let map = Map::new(MapConfig::default()).unwrap();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/assets/robot.glb");
        let position = GeoPosition::new(37.7749, -122.4194, 80.);
        let removed = map.add_3d_obj(position, &path).unwrap();
        let ready = map.add_3d_obj(position, &path).unwrap();
        let alias = map
            .add_3d_obj(position, path.parent().unwrap().join("../assets/robot.glb"))
            .unwrap();
        let failed = map
            .add_3d_obj(position, "/private/tmp/terra-map-missing-model.glb")
            .unwrap();
        map.illuminate_3d_object(ready, Color::rgb(255, 100, 0))
            .unwrap();
        let mut objects = Objects::new(Some(map.clone()), 1024 * 1024);
        let mut renderer = Renderer::new().unwrap();
        objects.poll(&mut renderer).unwrap();
        map.remove_object(removed).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            objects.poll(&mut renderer).unwrap();
            if map.object_status(ready).unwrap() == ObjectStatus::Ready
                && map.object_status(alias).unwrap() == ObjectStatus::Ready
                && matches!(map.object_status(failed).unwrap(), ObjectStatus::Failed(_))
            {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(map.object_status(ready).unwrap(), ObjectStatus::Ready);
        assert!(matches!(
            map.object_status(failed).unwrap(),
            ObjectStatus::Failed(_)
        ));
        assert!(map.object_status(removed).is_err());
        assert_eq!(map.object_status(alias).unwrap(), ObjectStatus::Ready);
        assert_eq!(objects.models.len(), 1);
        assert!(renderer.has_objects());
        map.remove_object(ready).unwrap();
        map.remove_object(alias).unwrap();
        objects.poll(&mut renderer).unwrap();
        assert!(objects.models.is_empty());
        assert!(!renderer.has_objects());
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn compressed_texture_is_rejected_before_large_image_allocation() {
        use base64::Engine;
        let mut png = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(64, 64, image::Rgba([40, 80, 120, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let doc = serde_json::json!({
            "asset": {"version":"2.0"},
            "images": [{"uri": format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png.into_inner()))}],
            "scenes": [{"nodes":[]}], "scene":0
        });
        let error = content::decode_with_limit(
            &serde_json::to_vec(&doc).unwrap(),
            &Url::parse("file:///test.gltf").unwrap(),
            |_| unreachable!(),
            1024,
        )
        .err()
        .unwrap();
        assert!(
            error.to_string().contains("image allocation limit"),
            "{error}"
        );
    }
    use std::io::Cursor;
}
