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
#[path = "../../../../tests/terra-map/unit/maps_objects_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../tests/terra-map/unit/maps_objects_lifecycle_tests.rs"]
mod lifecycle_tests;

#[cfg(test)]
#[path = "../../../../tests/terra-map/unit/maps_objects_budget_tests.rs"]
mod budget_tests;
