//! Geographic (WGS84) outdoor fleet map powered by `terra-map`.
//!
//! Renders Google Photorealistic 3D Tiles (or a clean offline WGS84 canvas when
//! `GOOGLE_MAPS_API_KEY` is unset) with projected GPUI overlays for:
//! - GPS-anchored rovers (position, compass heading, urgency/selection rings)
//! - Active `Goal::Gps` markers, arrival-tolerance circles, and dashed lines
//! - Optional anchored occupancy grid obstacles
//! - Interactive click-to-place WGS84 goal picking (`Goal::Gps`) with live
//!   distance and tolerance preview.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use terra_map::{Map, MapConfig, MapMode, MapView};

use crate::components::help_queue::tone;
use crate::components::widgets::{ButtonLook, button, mono, segment, segmented};
use crate::fleet::{Fleet, Rover, haversine_distance_m, world_to_lat_lon};
use crate::theme;

/// Events emitted by [`GeoMap`] to its parent page (`SupervisePage`).
pub enum GeoMapEvent {
    /// The operator clicked a rover on the geographic map.
    RoverPicked(u64),
}

impl EventEmitter<GeoMapEvent> for GeoMap {}

pub struct GeoMap {
    fleet: Fleet,
    map: Map,
    map_view: Entity<MapView>,
    mode: MapMode,
    survey: bool,
    show_obstacles: bool,
    has_tiles_key: bool,
    demo_mode: bool,
    /// True once we have flown to the first GPS-anchored rover.
    auto_centered: bool,
    /// Currently selected rover id (highlighted on the map).
    focused_rover: Option<u64>,
    /// Rover id we are currently picking a WGS84 goal for.
    picking_for: Option<u64>,
    /// Last hovered window position while picking a GPS goal.
    hover_pos: Option<Point<Pixels>>,
    /// Brief feedback message if a GPS goal attempt failed.
    notice: Option<String>,
    focus: FocusHandle,
}

impl GeoMap {
    pub fn new(fleet: Fleet, cx: &mut Context<Self>) -> Self {
        let mut config = MapConfig::default();
        let has_tiles_key = config.api_key.as_ref().is_some_and(|k| !k.trim().is_empty());
        let demo_mode = config.demo;
        config.show_controls = false;
        config.distance = 140.0;
        config.pitch = 55.0;
        // Degrade cleanly when no Google API key is configured (unless MAPS_DEMO=1):
        // disable tile fetches so the map stays a clean interactive WGS84 canvas.
        if !has_tiles_key && !demo_mode {
            config.tiles = false;
        }
        let survey = config.survey;
        let map = Map::new(config).expect("default terra-map config is valid");
        let map_view = cx.new(|cx| MapView::new(map.clone(), cx));
        Self {
            fleet,
            map,
            map_view,
            mode: MapMode::ThreeD,
            survey,
            show_obstacles: true,
            has_tiles_key,
            demo_mode,
            auto_centered: false,
            focused_rover: None,
            picking_for: None,
            hover_pos: None,
            notice: None,
            focus: cx.focus_handle(),
        }
    }

    pub fn set_focused_rover(&mut self, rover_id: Option<u64>) {
        self.focused_rover = rover_id;
    }

    pub fn fly_to(&self, latitude: f64, longitude: f64) {
        let _ = self.map.show_lat_lon(latitude, longitude);
    }

    pub fn pick_goal_for(&mut self, rover_id: u64, cx: &mut Context<Self>) {
        self.picking_for = Some(rover_id);
        self.hover_pos = None;
        self.notice = None;
        cx.notify();
    }

    fn cancel_picking(&mut self, cx: &mut Context<Self>) {
        self.picking_for = None;
        self.hover_pos = None;
        cx.notify();
    }

    fn set_mode(&mut self, mode: MapMode, cx: &mut Context<Self>) {
        self.mode = mode;
        self.map.set_mode(mode);
        cx.notify();
    }

    fn set_survey(&mut self, survey: bool, cx: &mut Context<Self>) {
        self.survey = survey;
        self.map.set_survey(survey);
        cx.notify();
    }

    fn center_on_fleet(&mut self, rovers: &[Rover]) {
        // Prefer the focused rover if it has a geo pose; otherwise the first anchored rover.
        let target = self
            .focused_rover
            .and_then(|id| rovers.iter().find(|r| r.id == id))
            .and_then(|r| r.geo_pose)
            .or_else(|| rovers.iter().find_map(|r| r.geo_pose));
        if let Some(geo) = target {
            let _ = self.map.show_lat_lon(geo.latitude, geo.longitude);
        }
    }
}

/// Snapshot of projected screen-space geometry painted over `MapView`.
struct OverlayData {
    rovers: Vec<ProjectedRover>,
    obstacles: Vec<Point<Pixels>>,
    preview: Option<GoalPreview>,
}

struct ProjectedRover {
    id: u64,
    label: String,
    center: Point<Pixels>,
    /// Screen-space unit vector pointing in the rover's forward heading.
    forward: (f32, f32),
    accent: u32,
    urgent: bool,
    selected: bool,
    goal: Option<ProjectedGoal>,
}

struct ProjectedGoal {
    pos: Point<Pixels>,
    tolerance_px: f32,
}

struct GoalPreview {
    rover_pos: Option<Point<Pixels>>,
    target_pos: Point<Pixels>,
    tolerance_px: f32,
    latitude: f64,
    longitude: f64,
    distance_m: Option<f64>,
    tolerance_m: f64,
}

impl Render for GeoMap {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rovers = self.fleet.rovers();
        let anchored_count = rovers.iter().filter(|r| r.geo_pose.is_some()).count();

        if !self.auto_centered && anchored_count > 0 {
            self.center_on_fleet(&rovers);
            self.auto_centered = true;
        }

        // Check whether goal picking is currently allowed for the target rover.
        let picking_state: Option<(u64, String, Result<f64, &'static str>)> =
            self.picking_for.map(|id| {
                if let Some(rover) = rovers.iter().find(|r| r.id == id) {
                    let check = rover
                        .supports_gps_goal()
                        .map(|()| rover.gps_goal_tolerance());
                    (id, rover.name(), check)
                } else {
                    (id, format!("Rover {id}"), Err("Rover is not connected"))
                }
            });
        let can_pick = picking_state
            .as_ref()
            .is_some_and(|(_, _, check)| check.is_ok());

        // Project rovers, goals, obstacles, and live goal preview using MapView's camera.
        let overlay = {
            let mv = self.map_view.read(cx);
            let mpp = mv.metres_per_pixel().unwrap_or(0.5).max(1e-3);

            let mut projected_rovers = Vec::new();
            let mut projected_obstacles = Vec::new();

            for rover in &rovers {
                let Some(geo) = rover.geo_pose else { continue };
                let Some(center) = mv.project_local(geo.latitude, geo.longitude) else {
                    continue;
                };

                // Compute exact screen-space heading by projecting a point 2 m ahead.
                let (fwd_x, fwd_y) = {
                    let h_rad = geo.heading_deg.to_radians();
                    let d_north = 2.0 * h_rad.cos();
                    let d_east = 2.0 * h_rad.sin();
                    let lat2 = geo.latitude + (d_north / 6_371_000.0).to_degrees();
                    let lon2 = geo.longitude
                        + (d_east / (6_371_000.0 * geo.latitude.to_radians().cos().abs().max(1e-6)))
                            .to_degrees();
                    if let Some(ahead) = mv.project_local(lat2, lon2) {
                        let dx = f32::from(ahead.x - center.x);
                        let dy = f32::from(ahead.y - center.y);
                        let len = dx.hypot(dy);
                        if len > 1e-3 {
                            (dx / len, dy / len)
                        } else {
                            (0.0, -1.0)
                        }
                    } else {
                        (0.0, -1.0)
                    }
                };

                // Project active GPS goal (or local goal via anchor).
                let goal = if let Some(g) = rover.gps_goal {
                    mv.project_local(g.latitude, g.longitude).map(|pos| ProjectedGoal {
                        pos,
                        tolerance_px: ((g.tolerance / mpp) as f32).clamp(6.0, 240.0),
                    })
                } else if let (Some([gx, gy]), Some(anchor)) =
                    (rover.goal, rover.gps_anchor.as_ref())
                {
                    let (glat, glon) = world_to_lat_lon(anchor, gx, gy);
                    mv.project_local(glat, glon).map(|pos| ProjectedGoal {
                        pos,
                        tolerance_px: ((0.35 / mpp) as f32).clamp(6.0, 240.0),
                    })
                } else {
                    None
                };

                let accent = rover
                    .urgency()
                    .map(|u| tone(u).accent)
                    .unwrap_or(theme::BLUE);

                projected_rovers.push(ProjectedRover {
                    id: rover.id,
                    label: rover.short_name(),
                    center,
                    forward: (fwd_x, fwd_y),
                    accent,
                    urgent: rover.urgency().is_some(),
                    selected: self.focused_rover == Some(rover.id),
                    goal,
                });

                // Optional anchored occupancy grid obstacles.
                if self.show_obstacles
                    && let Some(anchor) = rover.gps_anchor.as_ref()
                    && let Some(occ) = self.fleet.occupancy(rover.id)
                {
                    for cell in occ.cells.iter().take(1200) {
                        let (clat, clon) =
                            world_to_lat_lon(anchor, cell[0] as f64, cell[1] as f64);
                        if let Some(pt) = mv.project_local(clat, clon) {
                            projected_obstacles.push(pt);
                        }
                    }
                }
            }

            // Live goal preview while hovering in picking mode.
            let preview = if can_pick
                && let (Some(rover_id), Some(hover_window_pos)) = (self.picking_for, self.hover_pos)
                && let Some((lat, lon)) = mv.ground_at(hover_window_pos)
                && let Some(target_pos) = mv.project_local(lat, lon)
            {
                let rover = rovers.iter().find(|r| r.id == rover_id);
                let tolerance_m = rover.map_or(0.35, |r| r.gps_goal_tolerance());
                let rover_pos = projected_rovers
                    .iter()
                    .find(|r| r.id == rover_id)
                    .map(|r| r.center);
                let distance_m = rover
                    .and_then(|r| r.geo_pose)
                    .map(|g| haversine_distance_m(g.latitude, g.longitude, lat, lon));
                Some(GoalPreview {
                    rover_pos,
                    target_pos,
                    tolerance_px: ((tolerance_m / mpp) as f32).clamp(6.0, 240.0),
                    latitude: lat,
                    longitude: lon,
                    distance_m,
                    tolerance_m,
                })
            } else {
                None
            };

            OverlayData {
                rovers: projected_rovers,
                obstacles: projected_obstacles,
                preview,
            }
        };

        let preview_readout = overlay.preview.as_ref().map(|p| {
            let dist = p
                .distance_m
                .map(|d| format!(" · {d:.1} m"))
                .unwrap_or_default();
            format!(
                "{:.5}, {:.5}{dist} · ±{:.1} m",
                p.latitude, p.longitude, p.tolerance_m
            )
        });

        let rovers_for_center = rovers.clone();
        let clickable_rovers: Vec<(u64, String, Point<Pixels>, bool)> = overlay
            .rovers
            .iter()
            .map(|r| (r.id, r.label.clone(), r.center, r.selected))
            .collect();

        // Top-right compact controls.
        let controls = h_flex()
            .gap(px(6.0))
            .items_center()
            .child(
                segmented()
                    .child(
                        segment("geo-mode-2d", "2D", self.mode == MapMode::TwoD)
                            .on_click(cx.listener(|this, _, _, cx| this.set_mode(MapMode::TwoD, cx))),
                    )
                    .child(
                        segment("geo-mode-3d", "3D", self.mode == MapMode::ThreeD)
                            .on_click(cx.listener(|this, _, _, cx| this.set_mode(MapMode::ThreeD, cx))),
                    ),
            )
            .when(self.has_tiles_key || self.demo_mode, |row| {
                row.child(
                    segmented()
                        .child(
                            segment("geo-style-survey", "Survey", self.survey)
                                .on_click(cx.listener(|this, _, _, cx| this.set_survey(true, cx))),
                        )
                        .child(
                            segment("geo-style-imagery", "Imagery", !self.survey)
                                .on_click(cx.listener(|this, _, _, cx| this.set_survey(false, cx))),
                        ),
                )
            })
            .child(
                button(
                    "geo-toggle-obstacles",
                    if self.show_obstacles { "Grid: On" } else { "Grid: Off" },
                    ButtonLook::Outline,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_obstacles = !this.show_obstacles;
                    cx.notify();
                })),
            )
            .child(
                button("geo-center-fleet", "Center", ButtonLook::Outline).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.center_on_fleet(&rovers_for_center);
                        cx.notify();
                    },
                )),
            );

        // Status pill in top-left.
        let status_pill = if !self.has_tiles_key && !self.demo_mode {
            Some("Offline WGS84 (set GOOGLE_MAPS_API_KEY for 3D tiles)")
        } else if anchored_count == 0 {
            Some("No GPS-anchored rovers yet")
        } else {
            None
        };

        div()
            .id("geo-map-root")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(theme::MAP_BG))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && this.picking_for.is_some() {
                    this.cancel_picking(cx);
                    cx.stop_propagation();
                }
            }))
            // Base terra-map viewport.
            .child(div().size_full().child(self.map_view.clone()))
            // Non-interactive vector overlay canvas (obstacles, goals, rover chevrons, preview).
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| paint_overlay(bounds, &overlay, window),
                    )
                    .size_full()),
            )
            // Clickable rover badges (when not picking a goal).
            .when(self.picking_for.is_none(), |root| {
                root.children(clickable_rovers.into_iter().map(|(id, label, center, selected)| {
                    div()
                        .id(("geo-rover-badge", id))
                        .absolute()
                        .left(center.x + px(11.0))
                        .top(center.y - px(10.0))
                        .px(px(5.0))
                        .py(px(1.0))
                        .rounded(px(4.0))
                        .bg(rgb(if selected { theme::BLUE } else { theme::PANEL }))
                        .border_1()
                        .border_color(rgb(if selected { theme::BLUE } else { theme::BORDER }))
                        .text_color(rgb(if selected {
                            theme::INK
                        } else {
                            theme::TEXT
                        }))
                        .text_size(px(11.0))
                        .font_family("Menlo")
                        .cursor_pointer()
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(GeoMapEvent::RoverPicked(id));
                        }))
                        .child(label)
                }))
            })
            // Interactive goal-picking hit layer (only active when picking is allowed).
            .when(can_pick, |root| {
                let rover_id = self.picking_for.unwrap();
                root.child(
                    div()
                        .id("geo-pick-layer")
                        .absolute()
                        .inset_0()
                        .cursor_crosshair()
                        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                            this.hover_pos = Some(event.position);
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                let coord = this.map_view.read(cx).ground_at(event.position);
                                if let Some((lat, lon)) = coord {
                                    match this.fleet.go_to_gps(rover_id, lat, lon) {
                                        Ok(()) => {
                                            this.picking_for = None;
                                            this.hover_pos = None;
                                            this.notice = None;
                                        }
                                        Err(err) => {
                                            this.notice = Some(err.into());
                                        }
                                    }
                                    cx.notify();
                                }
                            }),
                        ),
                )
            })
            // Top-left status / picking banner.
            .child(
                v_flex()
                    .absolute()
                    .top(px(10.0))
                    .left(px(10.0))
                    .gap(px(6.0))
                    .when_some(status_pill, |col, msg| {
                        col.child(
                            div()
                                .px(px(10.0))
                                .py(px(5.0))
                                .rounded(px(6.0))
                                .bg(rgb(theme::PANEL))
                                .border_1()
                                .border_color(rgb(theme::BORDER))
                                .child(
                                    mono(msg)
                                        .text_size(px(11.0))
                                        .text_color(rgb(theme::TEXT_MUTED)),
                                ),
                        )
                    })
                    .when_some(picking_state, |col, (_, name, check)| {
                        let (bg, border, text_col, msg) = match check {
                            Ok(tol) => (
                                theme::PANEL,
                                theme::BLUE,
                                theme::TEXT,
                                format!("Click map to send {name} to GPS goal (±{tol:.1} m) · Esc to cancel"),
                            ),
                            Err(reason) => (
                                theme::RED.bg,
                                theme::RED.accent,
                                theme::RED.text,
                                format!("Cannot send GPS goal to {name}: {reason}"),
                            ),
                        };
                        col.child(
                            h_flex()
                                .px(px(10.0))
                                .py(px(6.0))
                                .gap(px(8.0))
                                .items_center()
                                .rounded(px(6.0))
                                .bg(rgb(bg))
                                .border_1()
                                .border_color(rgb(border))
                                .child(mono(msg).text_size(px(11.0)).text_color(rgb(text_col)))
                                .child(
                                    button("geo-cancel-pick", "Cancel", ButtonLook::Outline)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.cancel_picking(cx);
                                        })),
                                ),
                        )
                    })
                    .when_some(preview_readout, |col, readout| {
                        col.child(
                            div()
                                .px(px(10.0))
                                .py(px(4.0))
                                .rounded(px(6.0))
                                .bg(rgb(theme::PANEL))
                                .border_1()
                                .border_color(rgb(theme::BLUE))
                                .child(
                                    mono(readout)
                                        .text_size(px(11.0))
                                        .text_color(rgb(theme::BLUE_TEXT)),
                                ),
                        )
                    })
                    .when_some(self.notice.clone(), |col, msg| {
                        col.child(
                            div()
                                .px(px(10.0))
                                .py(px(5.0))
                                .rounded(px(6.0))
                                .bg(rgb(theme::RED.bg))
                                .border_1()
                                .border_color(rgb(theme::RED.accent))
                                .child(
                                    mono(msg)
                                        .text_size(px(11.0))
                                        .text_color(rgb(theme::RED.text)),
                                ),
                        )
                    }),
            )
            // Top-right map controls.
            .child(
                div()
                    .absolute()
                    .top(px(10.0))
                    .right(px(10.0))
                    .child(controls),
            )
    }
}

fn paint_overlay(bounds: Bounds<Pixels>, overlay: &OverlayData, window: &mut Window) {
    let to_win = |p: Point<Pixels>| point(bounds.origin.x + p.x, bounds.origin.y + p.y);

    // 1. Anchored occupancy obstacles.
    for &cell in &overlay.obstacles {
        let p = to_win(cell);
        window.paint_quad(fill(
            Bounds::centered_at(p, size(px(4.0), px(4.0))),
            rgba((theme::MAP_OBSTACLE << 8) | 0xcc),
        ));
    }

    // 2. Active goals (dashed line + tolerance circle + pin).
    for rover in &overlay.rovers {
        if let Some(goal) = &rover.goal {
            let from = to_win(rover.center);
            let to = to_win(goal.pos);
            paint_dashed_line(from, to, rgb(theme::BLUE), window);
            paint_circle_stroke(
                to,
                goal.tolerance_px,
                px(1.5),
                rgba((theme::BLUE << 8) | 0x99),
                window,
            );
            paint_circle_fill(to, 5.0, rgb(theme::BLUE), window);
        }
    }

    // 3. Live goal-picking preview.
    if let Some(preview) = &overlay.preview {
        let to = to_win(preview.target_pos);
        if let Some(from_local) = preview.rover_pos {
            paint_dashed_line(to_win(from_local), to, rgb(theme::ORANGE.accent), window);
        }
        paint_circle_stroke(
            to,
            preview.tolerance_px,
            px(1.5),
            rgba((theme::ORANGE.accent << 8) | 0xcc),
            window,
        );
        paint_circle_fill(to, 4.5, rgb(theme::ORANGE.accent), window);
    }

    // 4. Rover markers (selection/urgency ring + directional chevron).
    for rover in &overlay.rovers {
        let c = to_win(rover.center);
        if rover.urgent || rover.selected {
            let ring_color = if rover.urgent {
                rover.accent
            } else {
                theme::BLUE
            };
            paint_circle_stroke(c, 15.0, px(2.0), rgb(ring_color), window);
        }
        let (fx, fy) = rover.forward;
        let (rx, ry) = (-fy, fx); // right vector
        let nose = point(c.x + px(fx * 11.0), c.y + px(fy * 11.0));
        let back_l = point(
            c.x + px(-fx * 7.0 - rx * 7.0),
            c.y + px(-fy * 7.0 - ry * 7.0),
        );
        let back_r = point(
            c.x + px(-fx * 7.0 + rx * 7.0),
            c.y + px(-fy * 7.0 + ry * 7.0),
        );
        let mut poly = PathBuilder::fill();
        poly.move_to(nose);
        poly.line_to(back_r);
        poly.line_to(back_l);
        poly.close();
        if let Ok(path) = poly.build() {
            window.paint_path(path, rgb(rover.accent));
        }
    }
}

fn paint_dashed_line(
    from: Point<Pixels>,
    to: Point<Pixels>,
    color: impl Into<Background>,
    window: &mut Window,
) {
    let dx = f32::from(to.x - from.x);
    let dy = f32::from(to.y - from.y);
    let len = dx.hypot(dy);
    if len < 1.0 {
        return;
    }
    let ux = dx / len;
    let uy = dy / len;
    let dash = 8.0_f32;
    let gap = 5.0_f32;
    let mut t = 0.0_f32;
    let mut pb = PathBuilder::stroke(px(1.5));
    while t < len {
        let t_end = (t + dash).min(len);
        pb.move_to(point(from.x + px(ux * t), from.y + px(uy * t)));
        pb.line_to(point(from.x + px(ux * t_end), from.y + px(uy * t_end)));
        t += dash + gap;
    }
    if let Ok(path) = pb.build() {
        window.paint_path(path, color);
    }
}

fn paint_circle_stroke(
    center: Point<Pixels>,
    radius: f32,
    width: Pixels,
    color: impl Into<Background>,
    window: &mut Window,
) {
    let segments = 48usize;
    let mut pb = PathBuilder::stroke(width);
    for i in 0..=segments {
        let a = i as f32 / segments as f32 * std::f32::consts::TAU;
        let p = point(
            center.x + px(a.cos() * radius),
            center.y + px(a.sin() * radius),
        );
        if i == 0 {
            pb.move_to(p);
        } else {
            pb.line_to(p);
        }
    }
    if let Ok(path) = pb.build() {
        window.paint_path(path, color);
    }
}

fn paint_circle_fill(
    center: Point<Pixels>,
    radius: f32,
    color: impl Into<Background>,
    window: &mut Window,
) {
    let segments = 24usize;
    let mut pb = PathBuilder::fill();
    for i in 0..segments {
        let a = i as f32 / segments as f32 * std::f32::consts::TAU;
        let p = point(
            center.x + px(a.cos() * radius),
            center.y + px(a.sin() * radius),
        );
        if i == 0 {
            pb.move_to(p);
        } else {
            pb.line_to(p);
        }
    }
    pb.close();
    if let Ok(path) = pb.build() {
        window.paint_path(path, color);
    }
}
