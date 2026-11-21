use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use terra_comms::{BaseLink, CommsConfig, MeshChunk, Pose, Subscription as CommsSub, WorldMap};

const MAX_TRAIL_POINTS: usize = 400;

#[derive(Clone)]
struct WorldMapSnapshot {
    rover_id: u64,
    sequence: u64,
    points: Vec<[f32; 3]>,
    bytes: usize,
}

#[derive(Default)]
struct SharedInbox {
    latest_world_map: Option<WorldMapSnapshot>,
    mesh_updates: Vec<(u64, MeshChunk)>,
}

#[derive(Clone)]
struct RenderSnapshot {
    meshes: Vec<MeshChunk>,
    points: Vec<[f32; 3]>,
    rovers: BTreeMap<u64, Pose>,
    trails: BTreeMap<u64, Vec<(f64, f64)>>,
    center: (f64, f64),
    px_per_meter: f32,
}

pub struct RoverMap {
    endpoint: String,
    base: Option<BaseLink>,
    connect_error: Option<String>,
    _world_map_sub: Option<CommsSub>,
    _mesh_sub: Option<CommsSub>,
    inbox: Arc<Mutex<SharedInbox>>,

    meshes: BTreeMap<(u64, String), MeshChunk>,
    world_map: Option<WorldMapSnapshot>,
    rovers: BTreeMap<u64, Pose>,
    trails: BTreeMap<u64, VecDeque<(f64, f64)>>,
    connected: bool,

    // Camera view in world metres (center) and zoom (px / m)
    center: (f64, f64),
    px_per_meter: f32,
    dragging: Option<(Point<Pixels>, (f64, f64))>,

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

        let inbox = Arc::new(Mutex::new(SharedInbox::default()));
        let (base, world_map_sub, mesh_sub, connect_error) = match CommsConfig::from_toml(&toml)
            .map(|cfg| cfg.with_password(password))
            .and_then(|cfg| terra_comms::connect(&cfg))
        {
            Ok(link) => {
                let base = link.base();
                let inbox_w = inbox.clone();
                let w_sub = base.subscribe::<WorldMap>(move |rover_id, map| {
                    if let Ok(mut guard) = inbox_w.lock() {
                        guard.latest_world_map = Some(WorldMapSnapshot {
                            rover_id,
                            sequence: map.sequence,
                            points: map.points,
                            bytes: map.data.len(),
                        });
                    }
                });
                let inbox_m = inbox.clone();
                let m_sub = base.subscribe::<MeshChunk>(move |rover_id, chunk| {
                    if let Ok(mut guard) = inbox_m.lock() {
                        guard.mesh_updates.push((rover_id, chunk));
                    }
                });
                (Some(base), Some(w_sub), Some(m_sub), None)
            }
            Err(err) => (None, None, None, Some(err.to_string())),
        };

        let poll_task = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            let keep_going = this
                .update(cx, |map, cx| {
                    map.poll_comms(cx);
                })
                .is_ok();
            if !keep_going {
                break;
            }
        });

        Self {
            endpoint,
            base,
            connect_error,
            _world_map_sub: world_map_sub,
            _mesh_sub: mesh_sub,
            inbox,
            meshes: BTreeMap::new(),
            world_map: None,
            rovers: BTreeMap::new(),
            trails: BTreeMap::new(),
            connected: false,
            center: (0.0, 0.0),
            px_per_meter: 90.0,
            dragging: None,
            _poll_task: poll_task,
        }
    }

    fn poll_comms(&mut self, cx: &mut Context<Self>) {
        let Some(base) = &self.base else { return };
        let mut dirty = false;

        let now_connected = base.connected();
        if now_connected != self.connected {
            self.connected = now_connected;
            dirty = true;
        }

        // Update poses & trails for all online rovers (plus rover 0 fallback).
        let mut ids = base.rovers();
        if ids.is_empty() && base.latest::<Pose>(0).is_some() {
            ids.push(0);
        }
        for id in ids {
            if let Some((pose, _age)) = base.latest::<Pose>(id) {
                let changed = self.rovers.get(&id).is_none_or(|old| old != &pose);
                if changed {
                    self.rovers.insert(id, pose.clone());
                    if pose.tracked {
                        let trail = self.trails.entry(id).or_default();
                        let should_push = trail.back().is_none_or(|&(lx, ly)| {
                            (pose.x - lx).hypot(pose.y - ly) >= 0.03
                        });
                        if should_push {
                            if trail.len() >= MAX_TRAIL_POINTS {
                                trail.pop_front();
                            }
                            trail.push_back((pose.x, pose.y));
                        }
                    }
                    dirty = true;
                }
            }
        }

        // Drain incoming incremental ARMeshAnchor chunks and ARWorldMap.
        let (incoming_map, mesh_updates) = self
            .inbox
            .lock()
            .map(|mut guard| {
                (
                    guard.latest_world_map.take(),
                    std::mem::take(&mut guard.mesh_updates),
                )
            })
            .unwrap_or((None, Vec::new()));

        if !mesh_updates.is_empty() {
            for (rover_id, chunk) in mesh_updates {
                let key = (rover_id, chunk.id.clone());
                if chunk.vertices.is_empty() || chunk.indices.is_empty() {
                    self.meshes.remove(&key);
                } else {
                    self.meshes.insert(key, chunk);
                }
            }
            dirty = true;
        }

        if let Some(map) = incoming_map {
            let first_map = self.world_map.is_none() && self.meshes.is_empty();
            self.world_map = Some(map);
            if first_map {
                self.recenter_to_map();
            }
            dirty = true;
        }

        if dirty {
            cx.notify();
        }
    }

    fn recenter_to_map(&mut self) {
        if let Some(pose) = self.rovers.values().next() {
            self.center = (pose.x, pose.y);
            return;
        }
        if !self.meshes.is_empty() {
            let (mut sx, mut sy, mut n) = (0.0_f64, 0.0_f64, 0usize);
            for chunk in self.meshes.values() {
                for &[x, y, _] in &chunk.vertices {
                    sx += f64::from(x);
                    sy += f64::from(y);
                    n += 1;
                }
            }
            if n > 0 {
                self.center = (sx / n as f64, sy / n as f64);
                return;
            }
        }
        if let Some(map) = &self.world_map
            && !map.points.is_empty()
        {
            let (mut sx, mut sy) = (0.0_f64, 0.0_f64);
            for &[x, y, _] in &map.points {
                sx += f64::from(x);
                sy += f64::from(y);
            }
            let n = map.points.len() as f64;
            self.center = (sx / n, sy / n);
            return;
        }
        self.center = (0.0, 0.0);
    }

    fn snapshot(&self) -> RenderSnapshot {
        let meshes = self.meshes.values().cloned().collect();
        let points = self
            .world_map
            .as_ref()
            .map(|m| m.points.clone())
            .unwrap_or_default();
        let trails = self
            .trails
            .iter()
            .map(|(&id, q)| (id, q.iter().copied().collect()))
            .collect();
        RenderSnapshot {
            meshes,
            points,
            rovers: self.rovers.clone(),
            trails,
            center: self.center,
            px_per_meter: self.px_per_meter,
        }
    }

    fn clear_map(&mut self, cx: &mut Context<Self>) {
        self.meshes.clear();
        self.world_map = None;
        self.trails.clear();
        cx.notify();
    }

    fn recenter(&mut self, cx: &mut Context<Self>) {
        self.recenter_to_map();
        self.px_per_meter = 90.0;
        cx.notify();
    }
}

impl Render for RoverMap {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snap = self.snapshot();
        let rover_count = self.rovers.len();
        let mesh_chunks = self.meshes.len();
        let mesh_tris: usize = self.meshes.values().map(|m| m.indices.len()).sum();

        let status_label = if let Some(err) = &self.connect_error {
            format!("Config error: {err}")
        } else if self.connected {
            format!("Connected · {}", self.endpoint)
        } else {
            format!("Connecting to {}…", self.endpoint)
        };

        let pose_summary = self
            .rovers
            .iter()
            .next()
            .map(|(id, p)| {
                format!(
                    "R{id}: ({:.2}, {:.2})m  yaw {:+.0}°{}",
                    p.x,
                    p.y,
                    p.yaw.to_degrees(),
                    if p.tracked { "" } else { " [untracked]" }
                )
            })
            .unwrap_or_else(|| "No rover pose yet".into());

        let mesh_summary = format!("Mesh: {mesh_chunks} chunks · {mesh_tris} tris");

        let world_map_summary = self
            .world_map
            .as_ref()
            .map(|m| {
                format!(
                    "WorldMap R{} #{} · {} pts · {:.0} KB",
                    m.rover_id,
                    m.sequence,
                    m.points.len(),
                    m.bytes as f64 / 1024.0
                )
            })
            .unwrap_or_default();

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
                            .bg(if self.connected {
                                rgb(0x22c55e)
                            } else {
                                rgb(0xf59e0b)
                            }),
                    )
                    .child(div().text_color(cx.theme().foreground).child(status_label))
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("rovers: {rover_count}")),
                    )
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(pose_summary),
                    )
                    .child(
                        div()
                            .text_color(cx.theme().foreground)
                            .child(mesh_summary),
                    )
                    .when(!world_map_summary.is_empty(), |el| {
                        el.child(
                            div()
                                .text_color(cx.theme().muted_foreground)
                                .child(world_map_summary),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .id("map-recenter")
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(cx.theme().border)
                            .cursor_pointer()
                            .text_color(cx.theme().foreground)
                            .child("Recenter")
                            .on_click(cx.listener(|this, _, _, cx| this.recenter(cx))),
                    )
                    .child(
                        div()
                            .id("map-clear")
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(cx.theme().border)
                            .cursor_pointer()
                            .text_color(cx.theme().foreground)
                            .child("Clear")
                            .on_click(cx.listener(|this, _, _, cx| this.clear_map(cx))),
                    ),
            );

        let map_canvas = div()
            .id("rover-map-canvas")
            .flex_1()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .bg(rgb(0x0b0f14))
            .cursor_grab()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, _| {
                    this.dragging = Some((ev.position, this.center));
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    this.dragging = None;
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if let Some((start_px, start_center)) = this.dragging {
                    let dx = f32::from(ev.position.x - start_px.x) / this.px_per_meter;
                    let dy = f32::from(ev.position.y - start_px.y) / this.px_per_meter;
                    this.center = (
                        start_center.0 - f64::from(dx),
                        start_center.1 + f64::from(dy),
                    );
                    cx.notify();
                }
            }))
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                let dy = match ev.delta {
                    ScrollDelta::Lines(p) => p.y * 12.0,
                    ScrollDelta::Pixels(p) => f32::from(p.y),
                };
                let factor = (1.0 + dy * 0.005).clamp(0.8, 1.25);
                this.px_per_meter = (this.px_per_meter * factor).clamp(15.0, 400.0);
                cx.notify();
            }))
            .child(
                canvas(
                    move |_bounds, _window, _cx| snap.clone(),
                    |bounds, snap, window, _cx| {
                        paint_map(bounds, &snap, window);
                    },
                )
                .size_full(),
            );

        v_flex().size_full().child(hud).child(map_canvas)
    }
}

/// Colour for an `ARMeshClassification` byte:
/// 0=none, 1=wall, 2=floor, 3=ceiling (skipped), 4=table, 5=seat, 6=window, 7=door.
fn mesh_class_color(cls: u8) -> Option<Hsla> {
    match cls {
        2 => Some(Hsla::from(rgba(0x10b98144))), // floor: emerald
        1 => Some(Hsla::from(rgba(0xf43f5ebb))), // wall: coral
        4 => Some(Hsla::from(rgba(0xf59e0b88))), // table: amber
        5 => Some(Hsla::from(rgba(0xeab30888))), // seat: yellow
        7 => Some(Hsla::from(rgba(0x38bdf899))), // door: sky
        6 => Some(Hsla::from(rgba(0xa78bfa99))), // window: violet
        3 => None,                               // ceiling: hidden in top-down view
        _ => Some(Hsla::from(rgba(0x64748b44))), // none/unknown: subtle slate
    }
}

/// Height-based colour for ARWorldMap feature points (z in metres, Terra Z-up).
fn height_color(z: f32) -> Hsla {
    let t = ((z + 0.5) / 2.0).clamp(0.0, 1.0);
    let hue = (0.55 - t * 0.50).rem_euclid(1.0);
    hsla(hue, 0.85, 0.60, 0.85)
}

fn paint_map(bounds: Bounds<Pixels>, snap: &RenderSnapshot, window: &mut Window) {
    let ppm = snap.px_per_meter;
    let origin_x = f32::from(bounds.origin.x) + f32::from(bounds.size.width) * 0.5;
    let origin_y = f32::from(bounds.origin.y) + f32::from(bounds.size.height) * 0.5;
    let (cx, cy) = snap.center;

    // World (+X right, +Y up) -> screen pixels
    let to_screen = |wx: f64, wy: f64| -> Point<Pixels> {
        let sx = origin_x + ((wx - cx) as f32) * ppm;
        let sy = origin_y - ((wy - cy) as f32) * ppm;
        point(px(sx), px(sy))
    };

    // 1. 1-metre background grid
    let half_w_m = f32::from(bounds.size.width) * 0.5 / ppm;
    let half_h_m = f32::from(bounds.size.height) * 0.5 / ppm;
    let min_x = (cx - f64::from(half_w_m)).floor() as i32;
    let max_x = (cx + f64::from(half_w_m)).ceil() as i32;
    let min_y = (cy - f64::from(half_h_m)).floor() as i32;
    let max_y = (cy + f64::from(half_h_m)).ceil() as i32;

    let grid_color = Hsla::from(rgba(0x1f2937aa));
    let axis_color = Hsla::from(rgba(0x334155ff));

    for gx in min_x..=max_x {
        let mut path = PathBuilder::stroke(px(if gx == 0 { 1.5 } else { 1.0 }));
        path.move_to(to_screen(f64::from(gx), cy - f64::from(half_h_m)));
        path.line_to(to_screen(f64::from(gx), cy + f64::from(half_h_m)));
        if let Ok(built) = path.build() {
            window.paint_path(built, if gx == 0 { axis_color } else { grid_color });
        }
    }
    for gy in min_y..=max_y {
        let mut path = PathBuilder::stroke(px(if gy == 0 { 1.5 } else { 1.0 }));
        path.move_to(to_screen(cx - f64::from(half_w_m), f64::from(gy)));
        path.line_to(to_screen(cx + f64::from(half_w_m), f64::from(gy)));
        if let Ok(built) = path.build() {
            window.paint_path(built, if gy == 0 { axis_color } else { grid_color });
        }
    }

    // 2. Classified ARMeshAnchor triangles (floor underneath, walls/furniture on top)
    for pass_walls in [false, true] {
        for chunk in &snap.meshes {
            for (face_i, &[i0, i1, i2]) in chunk.indices.iter().enumerate() {
                let cls = chunk.classes.get(face_i).copied().unwrap_or(0);
                let is_obstacle = matches!(cls, 1 | 4 | 5 | 6 | 7);
                if is_obstacle != pass_walls {
                    continue;
                }
                let Some(color) = mesh_class_color(cls) else {
                    continue;
                };
                let (Some(&v0), Some(&v1), Some(&v2)) = (
                    chunk.vertices.get(i0 as usize),
                    chunk.vertices.get(i1 as usize),
                    chunk.vertices.get(i2 as usize),
                ) else {
                    continue;
                };
                let p0 = to_screen(f64::from(v0[0]), f64::from(v0[1]));
                let p1 = to_screen(f64::from(v1[0]), f64::from(v1[1]));
                let p2 = to_screen(f64::from(v2[0]), f64::from(v2[1]));

                let mut tri = PathBuilder::fill();
                tri.move_to(p0);
                tri.line_to(p1);
                tri.line_to(p2);
                tri.close();
                if let Ok(built) = tri.build() {
                    window.paint_path(built, color);
                }
            }
        }
    }

    // 3. ARWorldMap 3D feature point cloud (if sent)
    let dot = (ppm * 0.035).clamp(2.5, 6.0);
    let half = dot * 0.5;
    for &[wx, wy, wz] in &snap.points {
        let p = to_screen(f64::from(wx), f64::from(wy));
        window.paint_quad(
            fill(
                Bounds::new(point(p.x - px(half), p.y - px(half)), size(px(dot), px(dot))),
                height_color(wz),
            )
            .corner_radii(px(half)),
        );
    }

    // 4. Rover pose trails
    let trail_color = Hsla::from(rgba(0x38bdf8bb));
    for trail in snap.trails.values() {
        if trail.len() < 2 {
            continue;
        }
        let mut path = PathBuilder::stroke(px(2.0));
        path.move_to(to_screen(trail[0].0, trail[0].1));
        for &(wx, wy) in &trail[1..] {
            path.line_to(to_screen(wx, wy));
        }
        if let Ok(built) = path.build() {
            window.paint_path(built, trail_color);
        }
    }

    // 5. Rovers (body circle + heading arrow)
    for pose in snap.rovers.values() {
        let center = to_screen(pose.x, pose.y);
        let body_r = (0.25 * ppm).clamp(6.0, 28.0);
        let body_color = if pose.tracked {
            Hsla::from(rgba(0x38bdf8ff))
        } else {
            Hsla::from(rgba(0x94a3b8aa))
        };

        window.paint_quad(
            fill(
                Bounds::new(
                    point(center.x - px(body_r), center.y - px(body_r)),
                    size(px(body_r * 2.0), px(body_r * 2.0)),
                ),
                body_color,
            )
            .corner_radii(px(body_r)),
        );

        let tip = to_screen(
            pose.x + 0.45 * pose.yaw.cos(),
            pose.y + 0.45 * pose.yaw.sin(),
        );
        let mut heading = PathBuilder::stroke(px(2.5));
        heading.move_to(center);
        heading.line_to(tip);
        if let Ok(built) = heading.build() {
            window.paint_path(built, Hsla::from(rgba(0xf8fafcff)));
        }
    }
}
