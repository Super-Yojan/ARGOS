//! Native Google Photorealistic 3D Tiles rendered with wgpu inside GPUI.
//!
//! Set `GOOGLE_MAPS_API_KEY` in the launching process. `MAPS_DEMO=1` runs a
//! synthetic textured GPU fixture without making any network requests.
pub(crate) mod api;
pub(crate) mod camera;
mod content;
mod hud;
mod metrics;
mod navigation;
mod objects;
mod renderer;
#[cfg(test)]
mod tests;
mod tiles;
mod worker;

use camera::{Camera, MapMode};
use gpui_kit::{prelude::*, *};
use std::{sync::Arc, time::Duration};
use worker::{MapWorker, Update};

/// Credentials are intentionally excluded from Debug output.
#[derive(Clone)]
pub struct MapConfig {
    pub api_key: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f64,
    pub distance: f64,
    pub heading: f64,
    pub pitch: f64,
    pub screen_space_error: f64,
    pub cache_bytes: usize,
    pub max_resident_tiles: usize,
    pub max_frame_pixels: u64,
    pub demo: bool,
    pub survey: bool,
    /// Maximum live object instances (default 4096).
    pub max_objects: usize,
    /// Aggregate decoded model cache budget; decoding uses its remaining capacity.
    /// GPU resources, instance uniforms, source bytes and decoder scratch are additional.
    pub max_model_bytes: usize,
    /// Duration for show_lat_lon/show_position; zero jumps immediately.
    pub fly_duration: Duration,
    /// Show explorer controls. Attribution remains visible when false.
    pub show_controls: bool,
}
impl Default for MapConfig {
    fn default() -> Self {
        Self {
            api_key: std::env::var("GOOGLE_MAPS_API_KEY").ok(),
            latitude: 37.7749,
            longitude: -122.4194,
            altitude: 40.,
            distance: 1800.,
            heading: 0.,
            pitch: 50.,
            screen_space_error: 16.,
            cache_bytes: 384 * 1024 * 1024,
            max_resident_tiles: 2048,
            max_frame_pixels: 1_440_000,
            demo: std::env::var("MAPS_DEMO").is_ok_and(|s| s == "1"),
            survey: true,
            max_objects: 4096,
            max_model_bytes: 128 * 1024 * 1024,
            fly_duration: Duration::from_millis(500),
            show_controls: true,
        }
    }
}
impl MapConfig {
    fn camera(&self) -> Camera {
        Camera::new(
            self.latitude,
            self.longitude,
            self.altitude,
            self.distance,
            self.heading,
            self.pitch,
        )
    }
}

pub struct MapView {
    config: MapConfig,
    controller: api::Map,
    observed_revision: u64,
    navigation_revision: u64,
    mode_revision: u64,
    flight: Option<navigation::Flight>,
    camera: Camera,
    worker: MapWorker,
    image: Option<Arc<RenderImage>>,
    retired: Vec<Arc<RenderImage>>,
    status: String,
    credits: String,
    tile_count: usize,
    metrics: metrics::MapMetrics,
    viewport: Option<(u32, u32)>,
    generation: u64,
    epoch: u64,
    drag: Option<Point<Pixels>>,
    focus: FocusHandle,
    _poll: Task<()>,
}
impl MapView {
    pub fn new(controller: api::Map, cx: &mut Context<Self>) -> Self {
        let config = controller.config().clone();
        let worker = MapWorker::start_with_map(config.clone(), Some(controller.clone()));
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        this.apply_commands(cx);
                        // Backpressure: do not create images faster than GPUI can paint.
                        if this.retired.len() >= 2 {
                            return;
                        }
                        if let Some(frame) = this.worker.take_frame() {
                            let has_new_pixels = frame.pixels.is_some();
                            if has_new_pixels {
                                this.metrics = frame.metrics;
                                this.metrics.delivery_ms =
                                    frame.completed_at.elapsed().as_secs_f64() * 1000.;
                            } else {
                                this.metrics.pending = frame.metrics.pending;
                                this.metrics.resident = frame.metrics.resident;
                                this.metrics.download_ms = frame.metrics.download_ms;
                                this.metrics.decode_ms = frame.metrics.decode_ms;
                                this.metrics.upload_ms = frame.metrics.upload_ms;
                            }
                            let image_started = std::time::Instant::now();
                            if frame.epoch != this.epoch {
                                return;
                            }
                            if let Some(pixels) = frame.pixels
                                && let Some(buffer) =
                                    image::RgbaImage::from_raw(frame.width, frame.height, pixels)
                            {
                                if let Some(old) =
                                    this.image.replace(Arc::new(RenderImage::new(vec![
                                        image::Frame::new(buffer),
                                    ])))
                                {
                                    this.retired.push(old);
                                }
                                this.credits = frame.credits;
                                this.metrics.ui_image_ms =
                                    image_started.elapsed().as_secs_f64() * 1000.;
                            }
                            if this.status != frame.status
                                || this.tile_count != frame.tiles
                                || !this.retired.is_empty()
                                || has_new_pixels
                            {
                                this.status = frame.status;
                                this.tile_count = frame.tiles;
                                cx.notify();
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            controller,
            observed_revision: 0,
            navigation_revision: 0,
            mode_revision: 0,
            flight: None,
            camera: config.camera(),
            config,
            worker,
            image: None,
            retired: Vec::new(),
            status: "Loading native map".into(),
            credits: String::new(),
            tile_count: 0,
            metrics: Default::default(),
            viewport: None,
            generation: 0,
            epoch: 0,
            drag: None,
            focus: cx.focus_handle(),
            _poll: poll,
        }
    }
    pub fn controller(&self) -> api::Map {
        self.controller.clone()
    }
    fn apply_commands(&mut self, cx: &mut Context<Self>) {
        if self.controller.revision() != self.observed_revision {
            let snapshot = self.controller.snapshot();
            self.observed_revision = snapshot.revision;
            if let Some((revision, mode)) = snapshot.mode
                && revision != self.mode_revision
            {
                self.mode_revision = revision;
                self.set_mode(mode, cx);
            }
            if let Some((revision, position)) = snapshot.navigation
                && revision != self.navigation_revision
            {
                self.navigation_revision = revision;
                self.flight = Some(navigation::Flight::new(
                    &self.camera,
                    position,
                    self.config.fly_duration,
                ));
            }
        }
        if let Some(flight) = &self.flight {
            let finished = flight.advance(&mut self.camera);
            if finished {
                self.flight = None;
            }
            self.send();
            cx.notify();
        }
    }
    fn send(&mut self) {
        if let Some((width, height)) = self.viewport {
            self.generation = self.generation.wrapping_add(1);
            self.worker.update(Update {
                camera: self.camera.clone(),
                width,
                height,
                generation: self.generation,
                epoch: self.epoch,
                survey: self.config.survey,
            });
        }
    }
    fn resize(&mut self, bounds: Bounds<Pixels>, scale: f32) {
        let size = worker::render_size(
            f32::from(bounds.size.width) as f64,
            f32::from(bounds.size.height) as f64,
            scale as f64,
            self.config.max_frame_pixels,
        );
        if self.viewport != size {
            self.viewport = size;
            self.epoch = self.epoch.wrapping_add(1);
            self.send();
        }
    }
    fn reset(&mut self, cx: &mut Context<Self>) {
        self.flight = None;
        let mode = self.camera.mode;
        self.camera = self.config.camera();
        self.camera.set_mode(mode);
        self.epoch = self.epoch.wrapping_add(1);
        self.send();
        cx.notify();
    }
    fn set_mode(&mut self, mode: MapMode, cx: &mut Context<Self>) {
        if self.camera.mode == mode {
            return;
        }
        self.camera.set_mode(mode);
        self.drag = None;
        self.epoch = self.epoch.wrapping_add(1);
        self.send();
        cx.notify();
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "tab" {
            if event.keystroke.modifiers.shift {
                window.focus_prev(cx);
            } else {
                window.focus_next(cx);
            }
            cx.stop_propagation();
            return;
        }
        self.flight = None;
        match event.keystroke.key.as_str() {
            "up" | "w" => self.camera.pan(0., -40.),
            "down" | "s" => self.camera.pan(0., 40.),
            "left" | "a" => self.camera.pan(40., 0.),
            "right" | "d" => self.camera.pan(-40., 0.),
            "+" | "=" => self.camera.zoom(100.),
            "-" => self.camera.zoom(-100.),
            "r" | "home" => {
                self.reset(cx);
                cx.stop_propagation();
                return;
            }
            "2" => {
                self.set_mode(MapMode::TwoD, cx);
                cx.stop_propagation();
                return;
            }
            "3" => {
                self.set_mode(MapMode::ThreeD, cx);
                cx.stop_propagation();
                return;
            }
            "escape" => {
                self.drag = None;
                return;
            }
            _ => return,
        }
        self.focus.focus(window, cx);
        self.send();
        cx.notify();
        cx.stop_propagation();
    }
}
impl Focusable for MapView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for MapView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_map(window, cx)
    }
}
