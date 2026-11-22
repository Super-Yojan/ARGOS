use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::*;
use terra_comms::{BaseLink, CommsConfig, MeshChunk, Subscription as CommsSub};

use super::mesh_renderer::{MeshRenderer, OrbitCamera};

#[derive(Clone, Copy)]
enum DragMode {
    Orbit,
    Pan,
}

pub struct RoverMap {
    endpoint: String,
    base: Option<BaseLink>,
    connect_error: Option<String>,
    _mesh_sub: Option<CommsSub>,

    /// Incoming mesh chunks queued by the background Zenoh callback.
    pending_meshes: Arc<Mutex<Vec<(u64, MeshChunk)>>>,
    /// Latest copy of every received chunk, keyed by `(rover_id, chunk_id)`.
    meshes: BTreeMap<(u64, String), MeshChunk>,

    /// Hardware `wgpu` 3D renderer + current rendered frame.
    renderer: Option<MeshRenderer>,
    renderer_error: Option<String>,
    frame: Option<Arc<RenderImage>>,
    needs_render: bool,

    camera: OrbitCamera,
    hide_ceiling: bool,
    dragging: Option<(DragMode, Point<Pixels>)>,

    _poll_task: Task<()>,
}

impl RoverMap {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let endpoint =
            std::env::var("TERRA_ENDPOINT").unwrap_or_else(|_| "tcp/127.0.0.1:7447".into());
        let prefix = std::env::var("TERRA_PREFIX").unwrap_or_else(|_| "terra".into());
        let user = std::env::var("TERRA_USER").ok().filter(|s| !s.is_empty());
        let password = std::env::var("TERRA_PASSWORD").ok().filter(|s| !s.is_empty());

        let mut toml = format!("[comms]\nendpoint = \"{endpoint}\"\nprefix = \"{prefix}\"\n");
        if let Some(u) = &user {
            toml.push_str(&format!("user = \"{u}\"\n"));
        }

        let pending_meshes: Arc<Mutex<Vec<(u64, MeshChunk)>>> = Arc::new(Mutex::new(Vec::new()));

        let (base, mesh_sub, connect_error) = match CommsConfig::from_toml(&toml)
            .map(|cfg| cfg.with_password(password))
            .and_then(|cfg| terra_comms::connect(&cfg))
        {
            Ok(link) => {
                let base = link.base();
                let queue = pending_meshes.clone();
                let sub = base.subscribe::<MeshChunk>(move |rover_id, chunk| {
                    if let Ok(mut q) = queue.lock() {
                        q.push((rover_id, chunk));
                    }
                });
                (Some(base), Some(sub), None)
            }
            Err(err) => (None, None, Some(err.to_string())),
        };

        let (renderer, renderer_error) = match MeshRenderer::new(1024, 720) {
            Ok(r) => (Some(r), None),
            Err(err) => (None, Some(err.to_string())),
        };

        let poll_task = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            let ok = this
                .update(cx, |this, cx| {
                    this.drain_meshes(cx);
                })
                .is_ok();
            if !ok {
                break;
            }
        });

        Self {
            endpoint,
            base,
            connect_error,
            _mesh_sub: mesh_sub,
            pending_meshes,
            meshes: BTreeMap::new(),
            renderer,
            renderer_error,
            frame: None,
            needs_render: true,
            camera: OrbitCamera::default(),
            hide_ceiling: true,
            dragging: None,
            _poll_task: poll_task,
        }
    }

    fn drain_meshes(&mut self, cx: &mut Context<Self>) {
        let updates = self
            .pending_meshes
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default();

        if updates.is_empty() {
            return;
        }

        for (rover_id, chunk) in updates {
            let key = (rover_id, chunk.id.clone());
            if let Some(renderer) = &mut self.renderer {
                renderer.upsert_chunk(key.clone(), &chunk, self.hide_ceiling);
            }
            if chunk.vertices.is_empty() || chunk.indices.is_empty() {
                self.meshes.remove(&key);
            } else {
                self.meshes.insert(key, chunk);
            }
        }
        self.needs_render = true;
        cx.notify();
    }

    fn toggle_ceilings(&mut self, cx: &mut Context<Self>) {
        self.hide_ceiling = !self.hide_ceiling;
        if let Some(renderer) = &mut self.renderer {
            for (key, chunk) in &self.meshes {
                renderer.upsert_chunk(key.clone(), chunk, self.hide_ceiling);
            }
        }
        self.needs_render = true;
        cx.notify();
    }

    fn reset_view(&mut self, cx: &mut Context<Self>) {
        self.camera = OrbitCamera::default();
        self.needs_render = true;
        cx.notify();
    }

    fn clear_meshes(&mut self, cx: &mut Context<Self>) {
        self.meshes.clear();
        if let Some(renderer) = &mut self.renderer {
            renderer.clear();
        }
        self.needs_render = true;
        cx.notify();
    }

    fn print_meshes(&self) {
        println!("=== Stored Meshes ({} chunks) ===", self.meshes.len());
        for ((rover_id, id), chunk) in &self.meshes {
            println!(
                "  R{rover_id} {id}: {} verts, {} tris, {} classes",
                chunk.vertices.len(),
                chunk.indices.len(),
                chunk.classes.len(),
            );
        }
    }
}

impl Render for RoverMap {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Re-render on the GPU only when chunks changed or the camera moved.
        if self.needs_render
            && let Some(renderer) = &self.renderer
        {
            let next = renderer.render(&self.camera);
            if let Some(old) = self.frame.replace(next) {
                let _ = window.drop_image(old);
            }
            self.needs_render = false;
        }

        let connected = self.base.as_ref().is_some_and(BaseLink::connected);
        let chunk_count = self.meshes.len();
        let tri_count: usize = self.meshes.values().map(|c| c.indices.len()).sum();

        let status = if let Some(err) = &self.connect_error {
            format!("Config error: {err}")
        } else if let Some(err) = &self.renderer_error {
            format!("GPU error: {err}")
        } else if connected {
            format!("Connected · {}", self.endpoint)
        } else {
            format!("Connecting to {}…", self.endpoint)
        };

        let hud = h_flex()
            .w_full()
            .px_3()
            .py_2()
            .gap_3()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_xs()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .w(px(8.0))
                            .h(px(8.0))
                            .rounded_full()
                            .bg(if connected {
                                rgb(0x22c55e)
                            } else {
                                rgb(0xf59e0b)
                            }),
                    )
                    .child(div().text_color(cx.theme().foreground).child(status))
                    .child(
                        div()
                            .text_color(cx.theme().foreground)
                            .child(format!("Mesh: {chunk_count} chunks · {tri_count} tris")),
                    )
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child("Left-drag: orbit · Right-drag: pan · Scroll: zoom"),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .id("toggle-ceilings")
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(cx.theme().border)
                            .cursor_pointer()
                            .text_color(cx.theme().foreground)
                            .child(if self.hide_ceiling {
                                "Ceilings: Off"
                            } else {
                                "Ceilings: On"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_ceilings(cx))),
                    )
                    .child(
                        div()
                            .id("reset-view")
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(cx.theme().border)
                            .cursor_pointer()
                            .text_color(cx.theme().foreground)
                            .child("Reset View")
                            .on_click(cx.listener(|this, _, _, cx| this.reset_view(cx))),
                    )
                    .child(
                        div()
                            .id("clear-meshes")
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(cx.theme().border)
                            .cursor_pointer()
                            .text_color(cx.theme().foreground)
                            .child("Clear")
                            .on_click(cx.listener(|this, _, _, cx| this.clear_meshes(cx))),
                    )
                    .child(
                        div()
                            .id("print-meshes")
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(cx.theme().border)
                            .cursor_pointer()
                            .text_color(cx.theme().foreground)
                            .child("Print")
                            .on_click(cx.listener(|this, _, _, _| this.print_meshes())),
                    ),
            );

        let viewport = div()
            .id("wgpu-mesh-viewport")
            .flex_1()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .bg(rgb(0x0b0f14))
            .cursor_grab()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, _| {
                    let mode = if ev.modifiers.shift {
                        DragMode::Pan
                    } else {
                        DragMode::Orbit
                    };
                    this.dragging = Some((mode, ev.position));
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseDownEvent, _, _| {
                    this.dragging = Some((DragMode::Pan, ev.position));
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.dragging = None),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.dragging = None),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if let Some((mode, last)) = this.dragging {
                    let dx = f32::from(ev.position.x - last.x);
                    let dy = f32::from(ev.position.y - last.y);
                    match mode {
                        DragMode::Orbit => this.camera.orbit(dx, dy),
                        DragMode::Pan => this.camera.pan(dx, dy),
                    }
                    this.dragging = Some((mode, ev.position));
                    this.needs_render = true;
                    cx.notify();
                }
            }))
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                let dy = match ev.delta {
                    ScrollDelta::Lines(p) => p.y * 16.0,
                    ScrollDelta::Pixels(p) => f32::from(p.y),
                };
                this.camera.zoom(dy);
                this.needs_render = true;
                cx.notify();
            }))
            .children(
                self.frame
                    .clone()
                    .map(|frame| img(frame).size_full().object_fit(ObjectFit::Contain)),
            );

        v_flex().size_full().child(hud).child(viewport)
    }
}
