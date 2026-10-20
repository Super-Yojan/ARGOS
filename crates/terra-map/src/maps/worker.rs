use super::{
    MapConfig,
    camera::Camera,
    content::{self, TileContent},
    renderer::Renderer,
    tiles::{self, CacheEntry, HttpClient, TileTree},
};
use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, bounded};
use glam::{DMat4, DVec3};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use url::Url;

#[derive(Clone)]
pub struct Update {
    pub camera: Camera,
    pub width: u32,
    pub height: u32,
    pub generation: u64,
    pub epoch: u64,
    pub survey: bool,
}
pub struct Frame {
    pub generation: u64,
    pub epoch: u64,
    pub width: u32,
    pub height: u32,
    pub pixels: Option<Vec<u8>>,
    pub credits: String,
    pub status: String,
    pub tiles: usize,
    pub metrics: super::metrics::MapMetrics,
    pub completed_at: Instant,
}
#[derive(Default)]
struct Mailbox {
    update: Option<Update>,
    frame: Option<Frame>,
    generation: u64,
    epoch: u64,
    delivered_generation: u64,
}
pub struct MapWorker {
    mailbox: Arc<Mutex<Mailbox>>,
    stop: Arc<AtomicBool>,
}
impl MapWorker {
    #[cfg(test)]
    pub fn start(config: MapConfig) -> Self {
        Self::start_with_map(config, None)
    }
    pub fn start_with_map(config: MapConfig, map: Option<super::api::Map>) -> Self {
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (out, cancel) = (mailbox.clone(), stop.clone());
        let worker_generation = map.as_ref().map(|map| map.worker_started());
        thread::spawn(move || {
            if let Err(error) = run(config, &out, &cancel, map.clone()) {
                if let (Some(map), Some(generation)) = (&map, worker_generation)
                    && !cancel.load(Ordering::Relaxed)
                {
                    map.worker_failed(generation, format!("Map worker failed: {error:#}"));
                }
                let mut mailbox = out.lock().unwrap();
                let generation = mailbox.generation;
                mailbox.frame = Some(Frame {
                    generation,
                    epoch: mailbox.epoch,
                    width: 0,
                    height: 0,
                    pixels: None,
                    credits: String::new(),
                    status: format!("{error:#}"),
                    tiles: 0,
                    metrics: Default::default(),
                    completed_at: Instant::now(),
                });
            }
        });
        Self { mailbox, stop }
    }
    pub fn update(&self, update: Update) {
        let mut mailbox = self.mailbox.lock().unwrap();
        mailbox.generation = update.generation;
        mailbox.epoch = update.epoch;
        mailbox.update = Some(update);
    }
    pub fn take_frame(&self) -> Option<Frame> {
        let mut mailbox = self.mailbox.lock().unwrap();
        let frame = mailbox.frame.take()?;
        if frame.epoch != mailbox.epoch
            || frame.generation < mailbox.delivered_generation
            || frame.generation > mailbox.generation
        {
            return None;
        }
        mailbox.delivered_generation = frame.generation;
        Some(frame)
    }
}
impl Drop for MapWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn render_size(width: f64, height: f64, scale: f64, budget: u64) -> Option<(u32, u32)> {
    if !width.is_finite()
        || !height.is_finite()
        || !scale.is_finite()
        || width <= 0.
        || height <= 0.
        || scale <= 0.
        || budget == 0
    {
        return None;
    }
    let (w, h) = (width * scale, height * scale);
    if !(w * h).is_finite() {
        return None;
    }
    let ratio = (budget as f64 / (w * h))
        .sqrt()
        .min(1.)
        .min(4096. / w)
        .min(4096. / h);
    let (w, h) = (
        (w * ratio).floor().max(1.) as u32,
        (h * ratio).floor().max(1.) as u32,
    );
    if u64::from(w) * u64::from(h) > budget {
        return None;
    }
    Some((w, h))
}

struct Job {
    id: usize,
    epoch: u64,
    url: Url,
}
enum Payload {
    Hierarchy(Vec<u8>),
    Mesh(TileContent, Duration),
}
struct Loaded {
    id: usize,
    epoch: u64,
    result: Result<Payload>,
    download_ms: f64,
    decode_ms: f64,
}
struct Pool {
    jobs: Sender<Job>,
    results: Receiver<Loaded>,
}
impl Pool {
    fn new(http: HttpClient, stop: Arc<AtomicBool>) -> Self {
        let (jobs, requests) = bounded::<Job>(4);
        let (tx, results) = bounded(4);
        for _ in 0..4 {
            let (http, requests, tx, stop) =
                (http.clone(), requests.clone(), tx.clone(), stop.clone());
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let job = match requests.recv_timeout(Duration::from_millis(100)) {
                        Ok(j) => j,
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                        Err(_) => break,
                    };
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let mut download_ms = 0.;
                    let mut decode_ms = 0.;
                    let result = (|| {
                        let started = Instant::now();
                        let response = http.get(&job.url)?;
                        download_ms = started.elapsed().as_secs_f64() * 1000.;
                        if response.bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{')
                            && serde_json::from_slice::<serde_json::Value>(&response.bytes)
                                .is_ok_and(|v| v.get("root").is_some())
                        {
                            Ok(Payload::Hierarchy(response.bytes))
                        } else {
                            let mut ttl = response.cache_for;
                            let decode_started = Instant::now();
                            let mut external_ms = 0.;
                            let content = content::decode(&response.bytes, &job.url, |url| {
                                let external_started = Instant::now();
                                let external = http.get(url)?;
                                external_ms += external_started.elapsed().as_secs_f64() * 1000.;
                                ttl = ttl.min(external.cache_for);
                                Ok(external.bytes)
                            })?;
                            decode_ms = (decode_started.elapsed().as_secs_f64() * 1000.
                                - external_ms)
                                .max(0.);
                            download_ms += external_ms;
                            Ok(Payload::Mesh(content, ttl))
                        }
                    })();
                    let mut loaded = Loaded {
                        id: job.id,
                        epoch: job.epoch,
                        result,
                        download_ms,
                        decode_ms,
                    };
                    loop {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        match tx.send_timeout(loaded, Duration::from_millis(100)) {
                            Ok(()) => break,
                            Err(crossbeam_channel::SendTimeoutError::Timeout(value)) => {
                                loaded = value
                            }
                            Err(_) => return,
                        }
                    }
                }
            });
        }
        Self { jobs, results }
    }
}
struct Resident {
    meta: CacheEntry,
    credits: String,
}

fn root(http: &HttpClient, key: &str) -> Result<TileTree> {
    let base = Url::parse(tiles::ROOT_URL)?;
    let url = tiles::resolve_url(&base, tiles::ROOT_URL, key)?;
    TileTree::parse(&http.get(&url)?.bytes, url)
}
fn publish(mailbox: &Mutex<Mailbox>, frame: Frame) {
    let mut mailbox = mailbox.lock().unwrap();
    if frame.pixels.is_none()
        && let Some(pending) = mailbox.frame.as_mut()
        && pending.epoch == frame.epoch
        && pending.generation <= frame.generation
        && pending.pixels.is_some()
    {
        pending.status = frame.status;
    } else {
        mailbox.frame = Some(frame);
    }
}
fn status(
    mailbox: &Mutex<Mailbox>,
    update: &Update,
    message: String,
    tiles: usize,
    metrics: super::metrics::MapMetrics,
) {
    publish(
        mailbox,
        Frame {
            generation: update.generation,
            epoch: update.epoch,
            width: update.width,
            height: update.height,
            pixels: None,
            credits: String::new(),
            status: message,
            tiles,
            metrics,
            completed_at: Instant::now(),
        },
    );
}
fn wait_update(mailbox: &Mutex<Mailbox>, stop: &AtomicBool) -> Option<Update> {
    while !stop.load(Ordering::Relaxed) {
        if let Some(update) = mailbox.lock().unwrap().update.take() {
            return Some(update);
        }
        thread::sleep(Duration::from_millis(16));
    }
    None
}

fn run(
    config: MapConfig,
    mailbox: &Mutex<Mailbox>,
    stop: &Arc<AtomicBool>,
    map: Option<super::api::Map>,
) -> Result<()> {
    let Some(mut update) = wait_update(mailbox, stop) else {
        return Ok(());
    };
    if config.demo {
        return demo(config, update, mailbox, stop, map);
    }
    let Some(key) = config
        .api_key
        .as_deref()
        .filter(|key| !key.trim().is_empty())
    else {
        return objects_only(
            &config,
            update,
            mailbox,
            stop,
            map,
            "Set GOOGLE_MAPS_API_KEY to load Google tiles".into(),
        );
    };
    status(
        mailbox,
        &update,
        "Connecting to Google Map Tiles API".into(),
        0,
        Default::default(),
    );
    let http = HttpClient::new()?;
    let mut tree = match root(&http, key) {
        Ok(tree) => tree,
        Err(error) => return objects_only(&config, update, mailbox, stop, map, error.to_string()),
    };
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    let mut renderer = Renderer::new()?;
    if let Some(map) = &map { renderer.surface = map.surface.clone(); }
    let pool = Pool::new(http.clone(), stop.clone());
    let mut objects = super::objects::Objects::new(map, config.max_model_bytes);
    let mut resident: HashMap<usize, Resident> = HashMap::new();
    let mut pending = HashSet::new();
    let mut failed: HashMap<usize, (String, Option<Instant>)> = HashMap::new();
    let mut total_bytes = 0usize;
    let mut tick = 0u64;
    let mut epoch = 0u64;
    let mut refresh = Instant::now() + Duration::from_secs(10_500);
    let mut dirty = true;
    let mut previous_draw = Vec::new();
    let mut last_status = Instant::now() - Duration::from_secs(1);
    let mut session_notice = String::new();
    let mut metrics = super::metrics::MapMetrics::default();
    while !stop.load(Ordering::Relaxed) {
        dirty |= objects.poll(&mut renderer)?;
        let mut notice = session_notice.clone();
        if let Some(latest) = mailbox.lock().unwrap().update.take() {
            update = latest;
            dirty = true;
        }
        if Instant::now() >= refresh {
            status(
                mailbox,
                &update,
                "Refreshing map session".into(),
                resident.len(),
                metrics,
            );
            match root(&http, key) {
                Ok(next) => {
                    for id in resident.keys() {
                        renderer.remove(*id);
                    }
                    resident.clear();
                    pending.clear();
                    failed.clear();
                    total_bytes = 0;
                    tree = next;
                    epoch += 1;
                    dirty = true;
                    previous_draw.clear();
                    notice.clear();
                    session_notice.clear();
                    refresh = Instant::now() + Duration::from_secs(10_500);
                }
                Err(error) => {
                    notice = error.to_string();
                    session_notice = notice.clone();
                    refresh = Instant::now() + Duration::from_secs(180);
                }
            }
        }
        tick += 1;
        let view = update.camera.view(update.width, update.height);
        let ready = resident.keys().copied().collect();
        let select_started = Instant::now();
        let selection = tree.select(&view, &ready, config.screen_space_error);
        metrics.select_ms = select_started.elapsed().as_secs_f64() * 1000.;
        let protected = selection.keep.clone();
        let mut changed = false;
        // Evict stale responses only offscreen; an actively displayed response
        // remains the current presentation until replacement content is ready.
        let expired: Vec<_> = resident
            .iter()
            .filter(|(id, r)| !protected.contains(id) && r.meta.expires <= Instant::now())
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            evict(id, &mut resident, &mut renderer, &mut total_bytes);
        }
        while let Ok(loaded) = pool.results.try_recv() {
            if loaded.epoch != epoch {
                continue;
            }
            pending.remove(&loaded.id);
            metrics.download_ms = loaded.download_ms;
            if loaded.decode_ms > 0. {
                metrics.decode_ms = loaded.decode_ms;
            }
            match loaded.result {
                Ok(Payload::Hierarchy(bytes)) => {
                    if let Err(error) = tree.expand(loaded.id, &bytes) {
                        failed.insert(loaded.id, (error.to_string(), None));
                    }
                    changed = true;
                }
                Ok(Payload::Mesh(content, ttl)) => {
                    // Include the CPU surface acceleration data in the resident budget.
                    let bytes = content.bytes() + content.primitives.iter()
                        .map(|p| p.indices.len() / 3 * 128).sum::<usize>();
                    while total_bytes.saturating_add(bytes) > config.cache_bytes
                        || resident.len() >= config.max_resident_tiles
                    {
                        let oldest = resident
                            .iter()
                            .filter(|(id, _)| !protected.contains(id))
                            .min_by_key(|(_, r)| r.meta.used)
                            .map(|(id, _)| *id);
                        if let Some(id) = oldest {
                            evict(id, &mut resident, &mut renderer, &mut total_bytes);
                        } else {
                            break;
                        }
                    }
                    if total_bytes.saturating_add(bytes) > config.cache_bytes
                        || resident.len() >= config.max_resident_tiles
                    {
                        notice =
                            "Map memory budget reached; zoom out or increase MapConfig.cache_bytes"
                                .into();
                        failed.insert(
                            loaded.id,
                            (
                                notice.clone(),
                                Some(Instant::now() + Duration::from_secs(10)),
                            ),
                        );
                        continue;
                    }
                    let upload_started = Instant::now();
                    let uploaded =
                        renderer.upload(loaded.id, &content, tree.nodes[loaded.id].transform);
                    metrics.upload_ms = upload_started.elapsed().as_secs_f64() * 1000.;
                    match uploaded {
                        Ok(()) => {
                            total_bytes += bytes;
                            resident.insert(
                                loaded.id,
                                Resident {
                                    meta: CacheEntry {
                                        bytes,
                                        used: tick,
                                        expires: Instant::now() + ttl,
                                    },
                                    credits: content.copyright,
                                },
                            );
                            changed = true;
                        }
                        Err(error) => {
                            failed.insert(loaded.id, (error.to_string(), None));
                        }
                    }
                }
                Err(error) => {
                    let error = error.to_string();
                    let transient = error.contains("network")
                        || error.contains("timeout")
                        || error.contains("429")
                        || (500..600).any(|n| error.contains(&format!("HTTP {n}")));
                    failed.insert(
                        loaded.id,
                        (
                            error,
                            transient.then(|| Instant::now() + Duration::from_secs(30)),
                        ),
                    );
                }
            }
        }
        if changed {
            dirty = true;
            continue;
        }
        for id in &selection.draw {
            if let Some(r) = resident.get_mut(id) {
                r.meta.used = tick;
            }
        }
        for &id in &selection.requests {
            if pending.len() >= 4 {
                break;
            }
            if pending.contains(&id) {
                continue;
            }
            if let Some((error, retry)) = failed.get(&id) {
                if retry.is_none_or(|at| at > Instant::now()) {
                    notice = error.clone();
                    continue;
                }
                failed.remove(&id);
            }
            if let Some(url) = tree.nodes[id].content.clone()
                && pool.jobs.try_send(Job { id, epoch, url }).is_ok()
            {
                pending.insert(id);
            }
        }
        if let Some(error) = request_notice(&selection.requests, &failed, Instant::now()) {
            notice = error;
        }
        metrics.pending = pending.len();
        metrics.resident = resident.len();
        let message = if !notice.is_empty() {
            notice.clone()
        } else if selection.draw.is_empty() && pending.is_empty() {
            "No map tiles in this view · reset or move the camera".into()
        } else if selection.draw.is_empty() {
            format!("Loading map tiles · {} requests", pending.len())
        } else if !pending.is_empty() {
            format!("Refining map · {} requests", pending.len())
        } else {
            String::new()
        };
        if should_render(
            dirty,
            previous_draw != selection.draw,
            !selection.draw.is_empty() || renderer.has_objects(),
            pending.len(),
        ) {
            renderer.set_survey(update.survey);
            let pixels = renderer.render(&view, &selection.draw, update.width, update.height)?;
            metrics.render_ms = renderer.metrics.render_ms;
            metrics.gpu_wait_ms = renderer.metrics.gpu_wait_ms;
            metrics.pack_ms = renderer.metrics.pack_ms;
            metrics.pending = pending.len();
            metrics.resident = resident.len();
            let credits = tiles::copyrights(
                selection
                    .draw
                    .iter()
                    .filter_map(|id| resident.get(id).map(|r| r.credits.as_str())),
            );
            publish(
                mailbox,
                Frame {
                    generation: update.generation,
                    epoch: update.epoch,
                    width: update.width,
                    height: update.height,
                    pixels: Some(pixels),
                    credits,
                    status: message,
                    tiles: selection.draw.len(),
                    metrics,
                    completed_at: Instant::now(),
                },
            );
            previous_draw = selection.draw;
            dirty = false;
            last_status = Instant::now();
        } else if last_status.elapsed() > Duration::from_millis(250) {
            status(mailbox, &update, message, previous_draw.len(), metrics);
            last_status = Instant::now();
        }
        thread::sleep(Duration::from_millis(12));
    }
    Ok(())
}
/// Keep caller-owned objects usable even when the map service is unavailable.
fn objects_only(
    config: &MapConfig,
    mut update: Update,
    mailbox: &Mutex<Mailbox>,
    stop: &AtomicBool,
    map: Option<super::api::Map>,
    message: String,
) -> Result<()> {
    let mut renderer = Renderer::new()?;
    if let Some(map) = &map { renderer.surface = map.surface.clone(); }
    let mut objects = super::objects::Objects::new(map, config.max_model_bytes);
    let mut dirty = true;
    while !stop.load(Ordering::Relaxed) {
        dirty |= objects.poll(&mut renderer)?;
        if let Some(latest) = mailbox.lock().unwrap().update.take() {
            update = latest;
            dirty = true;
        }
        if dirty {
            renderer.set_survey(update.survey);
            let pixels = renderer.render(
                &update.camera.view(update.width, update.height),
                &[],
                update.width,
                update.height,
            )?;
            publish(
                mailbox,
                Frame {
                    generation: update.generation,
                    epoch: update.epoch,
                    width: update.width,
                    height: update.height,
                    pixels: Some(pixels),
                    credits: String::new(),
                    status: format!("Map tiles unavailable · {message}"),
                    tiles: 0,
                    metrics: renderer.metrics,
                    completed_at: Instant::now(),
                },
            );
            dirty = false;
        }
        thread::sleep(Duration::from_millis(12));
    }
    Ok(())
}

fn evict(
    id: usize,
    resident: &mut HashMap<usize, Resident>,
    renderer: &mut Renderer,
    total: &mut usize,
) {
    if let Some(r) = resident.remove(&id) {
        *total = total.saturating_sub(r.meta.bytes);
        renderer.remove(id);
    }
}
fn demo(
    config: MapConfig,
    mut update: Update,
    mailbox: &Mutex<Mailbox>,
    stop: &AtomicBool,
    map: Option<super::api::Map>,
) -> Result<()> {
    let source = Url::parse("https://tile.googleapis.com/demo.glb")?;
    let mut content = content::decode(
        include_bytes!("fixtures/textured-quad.glb"),
        &source,
        |_| unreachable!(),
    )?;
    let lat = config.latitude.to_radians();
    let lon = config.longitude.to_radians();
    let up = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
    let east = DVec3::new(-lon.sin(), lon.cos(), 0.);
    let north = up.cross(east);
    let world = DMat4::from_cols(
        (east * 500.).extend(0.),
        (north * 500.).extend(0.),
        up.extend(0.),
        update.camera.target.extend(1.),
    );
    content.primitives[0].transform = DMat4::IDENTITY;
    let mut renderer = Renderer::new()?;
    if let Some(map) = &map { renderer.surface = map.surface.clone(); }
    renderer.upload(0, &content, world)?;
    let mut objects = super::objects::Objects::new(map, config.max_model_bytes);
    let mut dirty = true;
    while !stop.load(Ordering::Relaxed) {
        dirty |= objects.poll(&mut renderer)?;
        if let Some(latest) = mailbox.lock().unwrap().update.take() {
            update = latest;
            dirty = true;
        }
        if !dirty {
            thread::sleep(Duration::from_millis(12));
            continue;
        }
        dirty = false;
        renderer.set_survey(update.survey);
        let pixels = renderer.render(
            &update.camera.view(update.width, update.height),
            &[0],
            update.width,
            update.height,
        )?;
        publish(
            mailbox,
            Frame {
                generation: update.generation,
                epoch: update.epoch,
                width: update.width,
                height: update.height,
                pixels: Some(pixels),
                credits: String::new(),
                status: "Offline GPU fixture · no Google map data".into(),
                tiles: 1,
                metrics: renderer.metrics,
                completed_at: Instant::now(),
            },
        );
    }
    Ok(())
}

/// Keep the last usable scene only while replacement requests are running.
fn request_notice(
    requests: &[usize],
    failed: &HashMap<usize, (String, Option<Instant>)>,
    now: Instant,
) -> Option<String> {
    requests
        .iter()
        .filter_map(|id| failed.get(id))
        .find(|(_, retry)| retry.is_none_or(|at| at > now))
        .map(|(error, _)| error.clone())
}

pub fn should_render(dirty: bool, changed: bool, has_draw: bool, pending: usize) -> bool {
    (dirty || changed) && (has_draw || pending == 0)
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    fn frame(generation: u64, pixels: Option<Vec<u8>>) -> Frame {
        Frame {
            generation,
            epoch: 0,
            width: 1,
            height: 1,
            pixels,
            credits: "source".into(),
            status: String::new(),
            tiles: 1,
            metrics: Default::default(),
            completed_at: Instant::now(),
        }
    }
    #[test]
    fn mailbox_preserves_pending_pixels_when_status_arrives() {
        let mailbox = Mutex::new(Mailbox::default());
        publish(&mailbox, frame(1, Some(vec![1, 2, 3, 255])));
        let mut status_frame = frame(1, None);
        status_frame.status = "Refining".into();
        publish(&mailbox, status_frame);
        let mailbox = mailbox.lock().unwrap();
        let frame = mailbox.frame.as_ref().unwrap();
        assert!(frame.pixels.is_some());
        assert_eq!(frame.status, "Refining");
    }
    #[test]
    fn mailbox_delivers_completed_frame_while_camera_keeps_moving() {
        let worker = MapWorker {
            mailbox: Arc::new(Mutex::new(Mailbox {
                generation: 2,
                frame: Some(frame(1, Some(vec![0; 4]))),
                ..Default::default()
            })),
            stop: Arc::new(AtomicBool::new(false)),
        };
        assert_eq!(worker.take_frame().map(|f| f.generation), Some(1));
    }
    #[test]
    fn error_notice_only_uses_current_failed_requests() {
        let now = Instant::now();
        let failed = HashMap::from([(
            1,
            (
                "temporary error".into(),
                Some(now + Duration::from_secs(30)),
            ),
        )]);
        assert!(request_notice(&[2], &failed, now).is_none());
        assert_eq!(
            request_notice(&[1], &failed, now),
            Some("temporary error".into())
        );
        assert!(request_notice(&[1], &failed, now + Duration::from_secs(31)).is_none());
        assert!(request_notice(&[1], &HashMap::new(), now).is_none());
    }
}

#[cfg(test)]
mod frame_order_tests {
    use super::*;
    fn worker_with(frame_epoch: u64, generation: u64, delivered: u64) -> MapWorker {
        MapWorker {
            mailbox: Arc::new(Mutex::new(Mailbox {
                epoch: 2,
                generation: 10,
                delivered_generation: delivered,
                frame: Some(Frame {
                    epoch: frame_epoch,
                    generation,
                    width: 1,
                    height: 1,
                    pixels: Some(vec![0; 4]),
                    credits: String::new(),
                    status: String::new(),
                    tiles: 1,
                    metrics: Default::default(),
                    completed_at: Instant::now(),
                }),
                ..Default::default()
            })),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }
    #[test]
    fn resize_or_mode_change_rejects_old_epoch() {
        assert!(worker_with(1, 9, 0).take_frame().is_none());
    }
    #[test]
    fn completed_frames_never_regress() {
        assert!(worker_with(2, 8, 9).take_frame().is_none());
    }
    #[test]
    fn camera_updates_do_not_starve_completed_frames() {
        assert_eq!(worker_with(2, 9, 8).take_frame().unwrap().generation, 9);
    }
}

#[cfg(test)]
mod object_fallback_tests {
    use super::*;
    #[test]
    #[ignore = "Requires a native GPU"]
    fn gpu_objects_load_and_update_without_google_credentials() {
        use super::super::api::{GeoPosition, Map, ObjectStatus};
        let config = MapConfig {
            api_key: None,
            demo: false,
            ..Default::default()
        };
        let map = Map::new(config.clone()).unwrap();
        let worker = MapWorker::start_with_map(config.clone(), Some(map.clone()));
        worker.update(Update {
            camera: config.camera(),
            width: 320,
            height: 240,
            generation: 1,
            epoch: 0,
            survey: true,
        });
        // Add after the worker has entered its missing-key fallback.
        thread::sleep(Duration::from_millis(50));
        let id = map
            .add_3d_obj(
                GeoPosition::new(config.latitude, config.longitude, 80.),
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/assets/robot.glb"),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut rendered = false;
        while Instant::now() < deadline {
            if let Some(frame) = worker.take_frame() {
                assert!(frame.status.contains("Map tiles unavailable"));
                rendered |= frame.pixels.is_some();
            }
            if rendered && map.object_status(id).unwrap() == ObjectStatus::Ready {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(rendered);
        assert_eq!(map.object_status(id).unwrap(), ObjectStatus::Ready);
    }
}
