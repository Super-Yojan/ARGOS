//! A live 3D view of what the rovers have scanned: the room mesh each
//! rover's phone streams, drawn with `wgpu` ([`MeshRenderer`]), with every
//! rover standing in it at its reported position and heading.
//!
//! Terra has no video topic, so this stands where the design shows a
//! camera.
//!
//! Left-drag orbits, right-drag (or shift-drag) pans, scroll zooms.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
use terra_comms::{MeshChunk, Subscription as MeshSubscription};

use super::help_queue::tone;
use super::mesh_renderer::{MeshColorMode, MeshRenderer, OrbitCamera, PathOverlay, RoverMarker};
use super::widgets::mono;
use crate::fleet::Fleet;
use crate::snapshot;
use crate::theme;

/// How often chunks that arrived from the network are handed to the GPU.
const DRAIN_PERIOD: Duration = Duration::from_millis(50);
/// How long the "Saved …" notice stays visible on the 3D view.
const NOTICE_DURATION: Duration = Duration::from_secs(6);
/// Size of the rendered image, pixels. It is scaled to fill the view.
const RENDER_SIZE: (u32, u32) = (1024, 720);

/// A mesh chunk is identified by the rover that sent it and the chunk's id.
type ChunkKey = (u64, String);

#[derive(Clone, Copy)]
enum DragMode {
    Orbit,
    Pan,
}

pub struct SceneView {
    fleet: Fleet,
    /// The rover markers last given to the renderer.
    markers: Vec<RoverMarker>,
    /// The planned paths last given to the renderer.
    paths: Vec<PathOverlay>,

    /// Chunks from the network, waiting for the next drain. Filled on the
    /// link's thread, emptied on the UI thread.
    incoming: Arc<Mutex<Vec<(u64, MeshChunk)>>>,
    /// The latest copy of every chunk, kept so the ceiling can be toggled.
    chunks: BTreeMap<ChunkKey, MeshChunk>,

    renderer: Option<MeshRenderer>,
    renderer_error: Option<String>,
    /// The last rendered image.
    frame: Option<Arc<RenderImage>>,
    /// The mesh or the camera changed since `frame` was rendered.
    stale: bool,

    camera: OrbitCamera,
    hide_ceiling: bool,
    color_mode: MeshColorMode,
    drag: Option<(DragMode, Point<Pixels>)>,
    /// Short-lived banner after saving a snapshot or clearing meshes:
    /// `(message, is_error, created_at)`.
    notice: Option<(String, bool, Instant)>,

    _mesh_subscription: Option<MeshSubscription>,
    _drain_task: Task<()>,
}

impl SceneView {
    pub fn new(fleet: &Fleet, cx: &mut Context<Self>) -> Self {
        let incoming: Arc<Mutex<Vec<(u64, MeshChunk)>>> = Arc::default();
        let mesh_subscription = fleet.base().map(|base| {
            let incoming = incoming.clone();
            base.subscribe::<MeshChunk>(move |rover_id, chunk| {
                if let Ok(mut incoming) = incoming.lock() {
                    incoming.push((rover_id, chunk));
                }
            })
        });

        let (renderer, renderer_error) = match MeshRenderer::new(RENDER_SIZE.0, RENDER_SIZE.1) {
            Ok(renderer) => (Some(renderer), None),
            Err(error) => (None, Some(error.to_string())),
        };

        let drain_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(DRAIN_PERIOD).await;
                let refreshed = this.update(cx, |this, cx| {
                    this.drain_incoming(cx);
                    this.place_rovers(cx);
                });
                if refreshed.is_err() {
                    break; // The view is gone.
                }
            }
        });

        Self {
            fleet: fleet.clone(),
            markers: Vec::new(),
            paths: Vec::new(),
            incoming,
            chunks: BTreeMap::new(),
            renderer,
            renderer_error,
            frame: None,
            stale: true,
            camera: OrbitCamera::default(),
            hide_ceiling: true,
            color_mode: MeshColorMode::VertexColor,
            drag: None,
            notice: None,
            _mesh_subscription: mesh_subscription,
            _drain_task: drain_task,
        }
    }

    /// Move the chunks that arrived since the last call to the GPU.
    fn drain_incoming(&mut self, cx: &mut Context<Self>) {
        let arrived = self
            .incoming
            .lock()
            .map(|mut incoming| std::mem::take(&mut *incoming))
            .unwrap_or_default();
        if arrived.is_empty() {
            return;
        }

        for (rover_id, chunk) in arrived {
            let key = (rover_id, chunk.id.clone());
            if let Some(renderer) = &mut self.renderer {
                renderer.upsert_chunk(key.clone(), &chunk, self.hide_ceiling, self.color_mode);
            }
            // An empty chunk means the rover dropped that piece of mesh.
            if chunk.vertices.is_empty() || chunk.indices.is_empty() {
                self.chunks.remove(&key);
            } else {
                self.chunks.insert(key, chunk);
            }
        }
        self.redraw(cx);
    }

    /// Move the rover markers and planned paths to where the rovers are now.
    /// Poses arrive 20 times a second with sensor jitter, so coordinates are
    /// rounded first: a rover that stands still costs no redraw.
    fn place_rovers(&mut self, cx: &mut Context<Self>) {
        let round_to = |value: f64, step: f64| ((value / step).round() * step) as f32;
        let rovers = self.fleet.rovers();
        let markers: Vec<RoverMarker> = rovers
            .iter()
            .filter_map(|rover| {
                let pose = rover.pose.as_ref()?;
                let color = rover
                    .urgency()
                    .map(|urgency| tone(urgency).accent)
                    .unwrap_or(theme::BLUE);
                Some(RoverMarker {
                    position: [round_to(pose.x, 0.02), round_to(pose.y, 0.02)],
                    yaw: round_to(pose.yaw, 0.02),
                    length: rover.footprint[0] as f32,
                    width: rover.footprint[1] as f32,
                    color: linear_rgb(color),
                })
            })
            .collect();
        let paths: Vec<PathOverlay> = rovers
            .iter()
            .filter_map(|rover| {
                let path = self.fleet.planned_path(rover.id)?;
                let points: Vec<[f32; 2]> = path
                    .poses
                    .iter()
                    .map(|p| [round_to(p.x, 0.02), round_to(p.y, 0.02)])
                    .collect();
                if points.len() < 2 {
                    return None;
                }
                let color = rover
                    .urgency()
                    .map(|urgency| tone(urgency).accent)
                    .unwrap_or(theme::BLUE);
                Some(PathOverlay {
                    points,
                    color: linear_rgb(color),
                })
            })
            .collect();

        let mut changed = false;
        if markers != self.markers {
            if let Some(renderer) = &mut self.renderer {
                renderer.set_rovers(&markers);
            }
            self.markers = markers;
            changed = true;
        }
        if paths != self.paths {
            if let Some(renderer) = &mut self.renderer {
                renderer.set_paths(&paths);
            }
            self.paths = paths;
            changed = true;
        }
        if changed {
            self.redraw(cx);
        }
    }

    fn toggle_ceiling(&mut self, cx: &mut Context<Self>) {
        self.hide_ceiling = !self.hide_ceiling;
        self.reupload_all_chunks();
        self.redraw(cx);
    }

    fn toggle_color_mode(&mut self, cx: &mut Context<Self>) {
        self.color_mode = match self.color_mode {
            MeshColorMode::VertexColor => MeshColorMode::Classification,
            MeshColorMode::Classification => MeshColorMode::VertexColor,
        };
        self.reupload_all_chunks();
        self.redraw(cx);
    }

    fn reupload_all_chunks(&mut self) {
        if let Some(renderer) = &mut self.renderer {
            for (key, chunk) in &self.chunks {
                renderer.upsert_chunk(key.clone(), chunk, self.hide_ceiling, self.color_mode);
            }
        }
    }

    fn reset_camera(&mut self, cx: &mut Context<Self>) {
        self.camera = OrbitCamera::default();
        self.redraw(cx);
    }

    /// Drop all accumulated 3D mesh chunks (both cached and queued) so stale
    /// geometry disappears until rovers stream fresh chunks.
    pub fn clear_meshes(&mut self, cx: &mut Context<Self>) {
        if let Ok(mut incoming) = self.incoming.lock() {
            incoming.clear();
        }
        let cleared = self.chunks.len();
        self.chunks.clear();
        if let Some(renderer) = &mut self.renderer {
            renderer.clear();
        }
        self.notice = Some((
            format!("Cleared {cleared} mesh chunks"),
            false,
            Instant::now(),
        ));
        self.redraw(cx);
    }

    /// Save the current fleet points, occupancy maps, 3D mesh chunks, and
    /// rendered 3D view into a timestamped snapshot folder.
    pub fn save_snapshot(&mut self, cx: &mut Context<Self>) {
        let view_3d = self
            .renderer
            .as_ref()
            .map(|renderer| renderer.render_rgba(&self.camera));
        let input = snapshot::collect(
            &self.fleet,
            &self.chunks,
            &self.camera,
            self.hide_ceiling,
            view_3d,
        );
        let root = snapshot::default_root();
        self.notice = match snapshot::write_to_root(&root, &input) {
            Ok(saved) => Some((
                format!(
                    "Saved {} ({} occ, {} pts, {} chunks)",
                    saved.dir.display(),
                    saved.occupancy_files,
                    saved.point_files,
                    saved.mesh_chunks,
                ),
                false,
                Instant::now(),
            )),
            Err(error) => Some((format!("Save failed: {error}"), true, Instant::now())),
        };
        cx.notify();
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        self.stale = true;
        cx.notify();
    }

    // ----------------------------------------------------------- the mouse

    fn start_drag(&mut self, event: &MouseDownEvent, _: &mut Window, _: &mut Context<Self>) {
        let pan = event.button == MouseButton::Right || event.modifiers.shift;
        let mode = if pan { DragMode::Pan } else { DragMode::Orbit };
        self.drag = Some((mode, event.position));
    }

    fn end_drag(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.drag = None;
    }

    fn drag_to(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.pressed_button.is_none() {
            self.drag = None;
        }
        let Some((mode, last)) = self.drag else {
            return;
        };
        let dx = f32::from(event.position.x - last.x);
        let dy = f32::from(event.position.y - last.y);
        match mode {
            DragMode::Orbit => self.camera.orbit(dx, dy),
            DragMode::Pan => self.camera.pan(dx, dy),
        }
        self.drag = Some((mode, event.position));
        self.redraw(cx);
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let pixels = match event.delta {
            ScrollDelta::Lines(delta) => delta.y * 16.0,
            ScrollDelta::Pixels(delta) => f32::from(delta.y),
        };
        self.camera.zoom(pixels);
        self.redraw(cx);
    }
}

impl Render for SceneView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Render on the GPU only when something changed.
        if self.stale
            && let Some(renderer) = &self.renderer
        {
            let new_frame = renderer.render(&self.camera);
            if let Some(old_frame) = self.frame.replace(new_frame) {
                let _ = window.drop_image(old_frame);
            }
            self.stale = false;
        }

        if let Some((_, _, at)) = &self.notice
            && at.elapsed() > NOTICE_DURATION
        {
            self.notice = None;
        }

        let rovers = self.markers.len();
        let (status, status_color) = match (&self.renderer_error, self.chunks.len()) {
            (Some(error), _) => (format!("GPU error: {error}"), theme::RED.fill),
            (None, 0) => (format!("3D · {rovers}R"), theme::TEXT_MUTED),
            (None, chunks) => (format!("3D · {rovers}R · {chunks} chunks"), theme::GREEN),
        };

        let notice = self.notice.as_ref().map(|(msg, is_error, _)| {
            let color = if *is_error {
                theme::RED.text
            } else {
                theme::GREEN
            };
            mono(msg.clone())
                .absolute()
                .left_2()
                .top_2()
                .px_2()
                .py_1()
                .rounded(px(6.0))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .bg(rgb(theme::PANEL))
                .text_size(px(11.0))
                .text_color(rgb(color))
        });

        let header_button = |id: &'static str, label: &'static str| {
            mono(label)
                .id(id)
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .bg(rgb(theme::PANEL_MUTED))
                .text_size(px(10.5))
                .text_color(rgb(theme::TEXT_MUTED))
                .cursor_pointer()
                .hover(|style| style.bg(rgb(theme::BORDER)).text_color(rgb(theme::TEXT)))
                .occlude()
        };

        let header = h_flex()
            .flex_none()
            .justify_between()
            .items_center()
            .gap_1()
            .px_2()
            .py(px(5.0))
            .bg(rgb(theme::PANEL))
            .border_b_1()
            .border_color(rgb(theme::BORDER_SUBTLE))
            .child(
                mono(status)
                    .text_size(px(10.5))
                    .text_color(rgb(status_color)),
            )
            .child(
                h_flex()
                    .gap(px(3.0))
                    .child(
                        header_button(
                            "toggle-color-mode",
                            match self.color_mode {
                                MeshColorMode::VertexColor => "RGB",
                                MeshColorMode::Classification => "Class",
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_color_mode(cx))),
                    )
                    .child(
                        header_button(
                            "toggle-ceiling",
                            if self.hide_ceiling { "Ceil" } else { "Ceil ✓" },
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_ceiling(cx))),
                    )
                    .child(
                        header_button("clear-meshes", "Clear")
                            .on_click(cx.listener(|this, _, _, cx| this.clear_meshes(cx))),
                    )
                    .child(
                        header_button("reset-camera", "Reset")
                            .on_click(cx.listener(|this, _, _, cx| this.reset_camera(cx))),
                    )
                    .child(
                        header_button("save-snapshot", "Save")
                            .on_click(cx.listener(|this, _, _, cx| this.save_snapshot(cx))),
                    ),
            );

        let canvas = div()
            .id("scene-view")
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .bg(rgb(theme::SCENE_BG))
            .cursor_grab()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::start_drag))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::start_drag))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::end_drag))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::end_drag))
            .on_mouse_move(cx.listener(Self::drag_to))
            .on_scroll_wheel(cx.listener(Self::scroll))
            .children(
                self.frame
                    .clone()
                    .map(|frame| img(frame).size_full().object_fit(ObjectFit::Cover)),
            )
            .children(notice);

        v_flex().size_full().child(header).child(canvas)
    }
}

/// A `0xRRGGBB` theme colour as the renderer wants it: three floats, 0..1.
fn linear_rgb(color: u32) -> [f32; 3] {
    let channel = |shift: u32| ((color >> shift) & 0xff) as f32 / 255.0;
    [channel(16), channel(8), channel(0)]
}
