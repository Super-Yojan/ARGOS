//! A top-down map of the fleet in each rover's world frame (metres, X to
//! the right, Y up).
//!
//! Rovers that need the operator are drawn large, with a ring and their
//! place in the help queue. The others stay small. A dashed line runs from
//! each rover to its goal. Underneath lies each rover's occupancy grid:
//! the cells it counts as obstacles.
//!
//! - drag to pan, scroll to zoom, click a rover to pick it
//!   ([`MapEvent::RoverPicked`]);
//! - after [`FleetMap::pick_goal_for`], the next click sends that rover
//!   there.
//!
//! [`FleetMap::inset`] makes a small, fixed map that follows one rover.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::help_queue::tone;
use super::widgets::{ButtonLook, button, mono};
use crate::fleet::assist::MAX_STATE_AGE_S;
use crate::fleet::{Fleet, Rover, help_queue};
use crate::theme;

/// Pixels per metre.
const DEFAULT_ZOOM: f32 = 40.0;
const MIN_ZOOM: f32 = 4.0;
const MAX_ZOOM: f32 = 400.0;
/// The closest the automatic fit zooms in, so one rover alone isn't huge.
const MAX_FIT_ZOOM: f32 = 60.0;
/// Free space kept around the rovers by the automatic fit, metres.
const FIT_MARGIN: f64 = 3.0;
/// A click this close to a rover picks it, pixels.
const PICK_RADIUS: f32 = 22.0;
/// The mouse must travel this far before a press becomes a drag, pixels.
const DRAG_THRESHOLD: f32 = 4.0;
/// The cell size assumed when a rover reports obstacles without one, metres.
const DEFAULT_CELL_M: f32 = 0.1;

pub enum MapEvent {
    /// The operator clicked this rover.
    RoverPicked(u64),
}

pub struct FleetMap {
    fleet: Fleet,
    /// The world point in the middle of the view, metres.
    center: [f64; 2],
    /// Pixels per metre.
    zoom: f32,
    /// Keep every rover in view. Turned off by the first pan or zoom.
    auto_fit: bool,
    /// Inset only: keep this rover in the middle.
    follow: Option<u64>,
    interactive: bool,
    /// The rover the next click sends somewhere.
    goal_for: Option<u64>,
    drag: Option<Drag>,
    /// Where the map was last painted. Mouse events arrive in window
    /// coordinates; this turns them into map coordinates.
    bounds: Rc<Cell<Bounds<Pixels>>>,
}

struct Drag {
    last: Point<Pixels>,
    /// Total distance so far, to tell a drag from a click.
    travelled: f32,
}

impl EventEmitter<MapEvent> for FleetMap {}

impl FleetMap {
    pub fn new(fleet: Fleet) -> Self {
        Self {
            fleet,
            center: [0.0, 0.0],
            zoom: DEFAULT_ZOOM,
            auto_fit: true,
            follow: None,
            interactive: true,
            goal_for: None,
            drag: None,
            bounds: Rc::default(),
        }
    }

    /// A small map without controls. Call [`Self::follow`] to say which
    /// rover it keeps in the middle.
    pub fn inset(fleet: Fleet) -> Self {
        Self {
            interactive: false,
            auto_fit: false,
            zoom: 30.0,
            ..Self::new(fleet)
        }
    }

    pub fn follow(&mut self, rover_id: Option<u64>, cx: &mut Context<Self>) {
        self.follow = rover_id;
        cx.notify();
    }

    /// The next click on the map sends this rover there.
    pub fn pick_goal_for(&mut self, rover_id: u64, cx: &mut Context<Self>) {
        self.goal_for = Some(rover_id);
        cx.notify();
    }

    fn cancel_goal(&mut self, cx: &mut Context<Self>) {
        self.goal_for = None;
        cx.notify();
    }

    /// Each rover's obstacle map. A rover that does not send its
    /// occupancy grid (older software) still shows the obstacles it reports
    /// while the obstacle assist runs.
    fn obstacle_grids(&self, rovers: &[Rover]) -> Vec<ObstacleGrid> {
        rovers
            .iter()
            .filter_map(|rover| {
                if let Some(occupancy) = self.fleet.occupancy(rover.id) {
                    let cells = occupancy
                        .cells
                        .iter()
                        .map(|cell| [cell[0] as f64, cell[1] as f64])
                        .collect();
                    let free = occupancy
                        .free
                        .iter()
                        .map(|cell| [cell[0] as f64, cell[1] as f64])
                        .collect();
                    return Some(ObstacleGrid {
                        cell_size: occupancy.resolution as f32,
                        cells,
                        free,
                    });
                }
                let (state, age) = self.fleet.remote_state(rover.id)?;
                (age <= MAX_STATE_AGE_S).then_some(ObstacleGrid {
                    cell_size: DEFAULT_CELL_M,
                    cells: state.obstacles,
                    free: Vec::new(),
                })
            })
            .collect()
    }

    fn projection(&self) -> Projection {
        Projection {
            center: self.center,
            zoom: self.zoom,
            bounds: self.bounds.get(),
        }
    }

    /// Move the view so it shows what it should: the followed rover, or
    /// every rover and goal.
    fn aim(&mut self, rovers: &[Rover]) {
        if let Some(followed) = rovers.iter().find(|rover| Some(rover.id) == self.follow) {
            if let Some(pose) = &followed.pose {
                self.center = [pose.x, pose.y];
            }
            return;
        }
        if !self.auto_fit {
            return;
        }

        let points: Vec<[f64; 2]> = rovers
            .iter()
            .flat_map(|rover| {
                rover
                    .pose
                    .as_ref()
                    .map(|pose| [pose.x, pose.y])
                    .into_iter()
                    .chain(rover.goal)
            })
            .collect();
        let Some(first) = points.first() else { return };
        let (mut min, mut max) = (*first, *first);
        for point in &points {
            for axis in 0..2 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
        self.center = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];

        let size = self.bounds.get().size;
        let (width, height) = (f32::from(size.width), f32::from(size.height));
        if width > 0.0 && height > 0.0 {
            let fit_x = width / (max[0] - min[0] + 2.0 * FIT_MARGIN) as f32;
            let fit_y = height / (max[1] - min[1] + 2.0 * FIT_MARGIN) as f32;
            self.zoom = fit_x.min(fit_y).clamp(MIN_ZOOM, MAX_FIT_ZOOM);
        }
    }

    // ----------------------------------------------------------- the mouse

    fn mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, _: &mut Context<Self>) {
        self.drag = Some(Drag {
            last: event.position,
            travelled: 0.0,
        });
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return;
        }
        let Some(drag) = &mut self.drag else { return };
        let dx = f32::from(event.position.x - drag.last.x);
        let dy = f32::from(event.position.y - drag.last.y);
        drag.last = event.position;
        drag.travelled += dx.abs() + dy.abs();
        if drag.travelled < DRAG_THRESHOLD {
            return;
        }

        // Screen Y points down, world Y points up.
        self.center[0] -= (dx / self.zoom) as f64;
        self.center[1] += (dy / self.zoom) as f64;
        self.auto_fit = false;
        cx.notify();
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let was_click = self
            .drag
            .take()
            .is_some_and(|drag| drag.travelled < DRAG_THRESHOLD);
        if was_click {
            self.click(event.position, cx);
        }
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => delta.y,
            ScrollDelta::Pixels(delta) => f32::from(delta.y) / 16.0,
        };
        self.zoom = (self.zoom * (lines * 0.08).exp()).clamp(MIN_ZOOM, MAX_ZOOM);
        self.auto_fit = false;
        cx.notify();
    }

    fn click(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let projection = self.projection();

        if let Some(rover_id) = self.goal_for.take() {
            let [x, y] = projection.to_world(position);
            self.fleet.go_to(rover_id, x, y);
            cx.notify();
            return;
        }

        let distance_to = |rover: &Rover| {
            let pose = rover.pose.as_ref()?;
            let at = projection.to_screen([pose.x, pose.y]);
            let (dx, dy) = (f32::from(at.x - position.x), f32::from(at.y - position.y));
            Some((dx * dx + dy * dy).sqrt())
        };
        let nearest = self
            .fleet
            .rovers()
            .iter()
            .filter_map(|rover| Some((rover.id, distance_to(rover)?)))
            .filter(|(_, distance)| *distance <= PICK_RADIUS)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((rover_id, _)) = nearest {
            cx.emit(MapEvent::RoverPicked(rover_id));
        }
    }
}

impl Render for FleetMap {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rovers = self.fleet.rovers();
        self.aim(&rovers);

        let markers = markers(&rovers, &self.fleet);
        let grids = self.obstacle_grids(&rovers);
        let (center, zoom, interactive) = (self.center, self.zoom, self.interactive);
        let remember_bounds = self.bounds.clone();
        let drawing = canvas(
            move |bounds, _, _| remember_bounds.set(bounds),
            move |bounds, _, window, cx| {
                let projection = Projection {
                    center,
                    zoom,
                    bounds,
                };
                for grid in &grids {
                    paint_free_grid(grid, &projection, window);
                }
                paint_grid(&projection, window);
                for grid in &grids {
                    paint_obstacle_grid(grid, &projection, window);
                }
                for marker in &markers {
                    paint_marker(marker, &projection, window, cx);
                }
                if interactive {
                    paint_scale_bar(&projection, window, cx);
                }
            },
        )
        .size_full();

        let map = div()
            .id("fleet-map")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(theme::MAP_BG))
            .child(drawing);
        if !self.interactive {
            return map;
        }

        let hint = match self.goal_for {
            Some(rover_id) => format!("Click the map to send Rover {rover_id}"),
            None => "Local metres · true-to-scale rovers · lighter floor = explored".into(),
        };

        map.cursor(if self.goal_for.is_some() {
            CursorStyle::Crosshair
        } else {
            CursorStyle::OpenHand
        })
        .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
        .on_mouse_move(cx.listener(Self::mouse_move))
        .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
        .on_scroll_wheel(cx.listener(Self::scroll))
        .child(
            h_flex()
                .absolute()
                .left_3()
                .top_3()
                .gap_2()
                .when(self.goal_for.is_some(), |bar| {
                    bar.child(
                        overlay_button("cancel-goal", "Cancel goal")
                            .border_color(rgb(theme::BLUE))
                            .text_color(rgb(theme::BLUE_TEXT))
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_goal(cx))),
                    )
                })
                .when(!self.auto_fit, |bar| {
                    bar.child(overlay_button("show-all", "Show all").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.auto_fit = true;
                            cx.notify();
                        },
                    )))
                }),
        )
        .child(
            mono(hint)
                .absolute()
                .left_3()
                .bottom_3()
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .bg(rgb(theme::PANEL))
                .text_color(rgb(theme::TEXT_MUTED)),
        )
    }
}

/// A button that floats on the map. `occlude` keeps a click on it from
/// also starting a drag on the map underneath.
fn overlay_button(id: &'static str, label: &'static str) -> Stateful<Div> {
    button(id, label, ButtonLook::Outline)
        .bg(rgb(theme::PANEL))
        .occlude()
}

// ---------------------------------------------------------------- geometry

/// Converts between world metres and window pixels.
#[derive(Clone, Copy)]
struct Projection {
    center: [f64; 2],
    zoom: f32,
    bounds: Bounds<Pixels>,
}

impl Projection {
    fn to_screen(&self, world: [f64; 2]) -> Point<Pixels> {
        let middle = self.bounds.center();
        point(
            middle.x + px((world[0] - self.center[0]) as f32 * self.zoom),
            middle.y - px((world[1] - self.center[1]) as f32 * self.zoom),
        )
    }

    fn to_world(&self, screen: Point<Pixels>) -> [f64; 2] {
        let middle = self.bounds.center();
        [
            self.center[0] + (f32::from(screen.x - middle.x) / self.zoom) as f64,
            self.center[1] - (f32::from(screen.y - middle.y) / self.zoom) as f64,
        ]
    }
}

// ----------------------------------------------------------------- drawing

/// One rover, ready to draw.
struct Marker {
    position: [f64; 2],
    /// Radians, counter-clockwise from +X.
    yaw: f64,
    /// Physical rover `[length, width]` in metres.
    footprint: [f64; 2],
    goal: Option<[f64; 2]>,
    /// Global-planner waypoints in the world frame, metres.
    planned_path: Vec<[f64; 2]>,
    label: String,
    color: u32,
    label_color: u32,
    route_color: u32,
    path_color: u32,
    /// `Some` for a rover in the help queue: its place, from 1.
    place: Option<usize>,
}

fn markers(rovers: &[Rover], fleet: &Fleet) -> Vec<Marker> {
    let queue = help_queue(rovers);
    rovers
        .iter()
        .filter_map(|rover| {
            let pose = rover.pose.as_ref()?;
            let place = queue
                .iter()
                .position(|queued| queued.id == rover.id)
                .map(|index| index + 1);
            let alert = rover.urgency().map(tone);
            let planned_path = fleet
                .planned_path(rover.id)
                .map(|path| path.poses.iter().map(|p| [p.x, p.y]).collect())
                .unwrap_or_default();
            Some(Marker {
                position: [pose.x, pose.y],
                yaw: pose.yaw,
                footprint: rover.footprint,
                goal: rover.goal,
                planned_path,
                label: match alert {
                    Some(_) => format!("{} · {}", rover.name(), rover.headline().to_lowercase()),
                    None => rover.short_name(),
                },
                color: alert.map(|tone| tone.accent).unwrap_or(theme::QUIET_ROVER),
                label_color: alert.map(|tone| tone.text).unwrap_or(theme::QUIET_LABEL),
                route_color: alert.map(|tone| tone.accent).unwrap_or(theme::QUIET_ROUTE),
                path_color: alert.map(|tone| tone.accent).unwrap_or(theme::BLUE),
                place,
            })
        })
        .collect()
}

/// Lines one metre apart; five metres when zoomed far out.
fn paint_grid(projection: &Projection, window: &mut Window) {
    let spacing = if projection.zoom >= 16.0 { 1.0 } else { 5.0 };
    let bounds = projection.bounds;
    let [left, top] = projection.to_world(bounds.origin);
    let [right, bottom] = projection.to_world(bounds.bottom_right());
    let line = rgb(theme::MAP_GRID);

    let mut x = (left / spacing).floor() * spacing;
    while x <= right {
        let at = projection.to_screen([x, 0.0]).x;
        window.paint_quad(fill(
            Bounds::new(
                point(at, bounds.origin.y),
                size(px(1.0), bounds.size.height),
            ),
            line,
        ));
        x += spacing;
    }
    let mut y = (bottom / spacing).floor() * spacing;
    while y <= top {
        let at = projection.to_screen([0.0, y]).y;
        window.paint_quad(fill(
            Bounds::new(point(bounds.origin.x, at), size(bounds.size.width, px(1.0))),
            line,
        ));
        y += spacing;
    }
}

/// The occupied and observed-free cells of one rover's map, world frame.
struct ObstacleGrid {
    /// Side of one cell, metres.
    cell_size: f32,
    /// Occupied cell centres, metres.
    cells: Vec<[f64; 2]>,
    /// Observed-free cell centres, metres.
    free: Vec<[f64; 2]>,
}

/// One square per observed-free cell so explored floor stands out from unknown space.
fn paint_free_grid(grid: &ObstacleGrid, projection: &Projection, window: &mut Window) {
    if grid.free.is_empty() {
        return;
    }
    let side = px((grid.cell_size * projection.zoom).max(2.0));
    for cell in &grid.free {
        let center = projection.to_screen(*cell);
        let corner = point(center.x - side / 2.0, center.y - side / 2.0);
        window.paint_quad(fill(
            Bounds::new(corner, size(side, side)),
            rgb(theme::MAP_FREE),
        ));
    }
}

/// One square per occupied cell, never smaller than 2 px.
fn paint_obstacle_grid(grid: &ObstacleGrid, projection: &Projection, window: &mut Window) {
    let side = px((grid.cell_size * projection.zoom).max(2.0));
    for cell in &grid.cells {
        let center = projection.to_screen(*cell);
        let corner = point(center.x - side / 2.0, center.y - side / 2.0);
        window.paint_quad(fill(
            Bounds::new(corner, size(side, side)),
            rgb(theme::MAP_OBSTACLE),
        ));
    }
}

/// A real-world scale bar in the bottom-right corner of the map.
fn paint_scale_bar(projection: &Projection, window: &mut Window, cx: &mut App) {
    // Pick a clean metric length that lands between ~50 px and ~140 px.
    let candidates = [0.2_f32, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0];
    let metres = candidates
        .iter()
        .copied()
        .find(|&m| m * projection.zoom >= 55.0)
        .unwrap_or(20.0);
    let bar_px = metres * projection.zoom;
    let label = if metres < 1.0 {
        format!("{} cm", (metres * 100.0).round() as i32)
    } else {
        format!("{metres:.0} m")
    };

    let right = projection.bounds.bottom_right().x - px(16.0);
    let bottom = projection.bounds.bottom_right().y - px(20.0);
    let left = right - px(bar_px);

    // Horizontal bar with end ticks.
    paint_line(point(left, bottom), point(right, bottom), &[], theme::TEXT_MUTED, window);
    paint_line(point(left, bottom - px(5.0)), point(left, bottom + px(2.0)), &[], theme::TEXT_MUTED, window);
    paint_line(point(right, bottom - px(5.0)), point(right, bottom + px(2.0)), &[], theme::TEXT_MUTED, window);
    paint_text(
        &label,
        point((left + right) / 2.0, bottom - px(12.0)),
        TextAnchor::Middle,
        11.0,
        theme::TEXT_MUTED,
        window,
        cx,
    );
}

fn paint_marker(marker: &Marker, projection: &Projection, window: &mut Window, cx: &mut App) {
    let at = projection.to_screen(marker.position);

    let has_path = marker.planned_path.len() >= 2;
    if has_path {
        let points: Vec<Point<Pixels>> = marker
            .planned_path
            .iter()
            .map(|p| projection.to_screen(*p))
            .collect();
        paint_polyline(&points, 2.5, marker.path_color, window);
    }

    if let Some(goal) = marker.goal {
        let goal = projection.to_screen(goal);
        if !has_path {
            paint_line(at, goal, &[px(6.0), px(6.0)], marker.route_color, window);
        }
        paint_ring(goal, 9.0, 2.0, marker.route_color, window);
    }

    // Scale the rover body from its real physical length × width (metres)
    // with a tiny screen-space floor so it stays visible when zoomed far out.
    let half_length = ((marker.footprint[0] as f32 * 0.5) * projection.zoom).max(4.0);
    let half_width = ((marker.footprint[1] as f32 * 0.5) * projection.zoom).max(3.0);
    paint_rover_body(
        at,
        marker.yaw,
        half_length,
        half_width,
        marker.color,
        window,
    );

    let mut label_offset = half_length + 8.0;
    if let Some(place) = marker.place {
        let ring_radius = (half_length.hypot(half_width) + 10.0).max(20.0);
        paint_ring(at, ring_radius, 2.5, marker.color, window);
        // The badge sits on the ring, up and to the right.
        let badge = point(at.x + px(ring_radius * 0.7), at.y - px(ring_radius * 0.7));
        paint_disc(badge, 12.0, marker.color, window);
        paint_text(
            &place.to_string(),
            badge,
            TextAnchor::Middle,
            13.0,
            theme::INK,
            window,
            cx,
        );
        label_offset = ring_radius + 8.0;
    }
    let label_at = point(at.x + px(label_offset), at.y);
    paint_text(
        &marker.label,
        label_at,
        TextAnchor::Left,
        12.0,
        marker.label_color,
        window,
        cx,
    );
}

/// A rounded box with a short line out of its front.
fn paint_rover_body(
    at: Point<Pixels>,
    yaw: f64,
    half_length: f32,
    half_width: f32,
    color: u32,
    window: &mut Window,
) {
    // Screen Y points down, so a counter-clockwise yaw turns the other way.
    let (sin, cos) = (yaw.sin() as f32, yaw.cos() as f32);
    let corner = |forward: f32, sideways: f32| {
        point(
            at.x + px(forward * cos + sideways * sin),
            at.y + px(-forward * sin + sideways * cos),
        )
    };

    let mut body = PathBuilder::fill();
    body.move_to(corner(half_length, half_width));
    body.line_to(corner(half_length, -half_width));
    body.line_to(corner(-half_length, -half_width));
    body.line_to(corner(-half_length, half_width));
    body.close();
    if let Ok(body) = body.build() {
        window.paint_path(body, rgb(color));
    }
    paint_line(
        corner(half_length, 0.0),
        corner(half_length + 8.0, 0.0),
        &[],
        color,
        window,
    );
}

/// A solid polyline, `width` px wide.
fn paint_polyline(points: &[Point<Pixels>], width: f32, color: u32, window: &mut Window) {
    let Some((&first, rest)) = points.split_first() else {
        return;
    };
    if rest.is_empty() {
        return;
    }
    let mut line = PathBuilder::stroke(px(width));
    line.move_to(first);
    for &pt in rest {
        line.line_to(pt);
    }
    if let Ok(line) = line.build() {
        window.paint_path(line, rgb(color));
    }
}

/// A straight line, 2 px wide. `dashes` is empty for a solid line.
fn paint_line(
    from: Point<Pixels>,
    to: Point<Pixels>,
    dashes: &[Pixels],
    color: u32,
    window: &mut Window,
) {
    let mut line = PathBuilder::stroke(px(2.0));
    if !dashes.is_empty() {
        line = line.dash_array(dashes);
    }
    line.move_to(from);
    line.line_to(to);
    if let Ok(line) = line.build() {
        window.paint_path(line, rgb(color));
    }
}

fn paint_ring(center: Point<Pixels>, radius: f32, width: f32, color: u32, window: &mut Window) {
    window.paint_quad(quad(
        circle_bounds(center, radius),
        px(radius),
        transparent_black(),
        px(width),
        rgb(color),
        BorderStyle::default(),
    ));
}

fn paint_disc(center: Point<Pixels>, radius: f32, color: u32, window: &mut Window) {
    window.paint_quad(fill(circle_bounds(center, radius), rgb(color)).corner_radii(px(radius)));
}

fn circle_bounds(center: Point<Pixels>, radius: f32) -> Bounds<Pixels> {
    Bounds::new(
        point(center.x - px(radius), center.y - px(radius)),
        size(px(radius * 2.0), px(radius * 2.0)),
    )
}

enum TextAnchor {
    /// `at` is the left end of the text, vertically centred.
    Left,
    /// `at` is the middle of the text.
    Middle,
}

fn paint_text(
    text: &str,
    at: Point<Pixels>,
    anchor: TextAnchor,
    font_size: f32,
    color: u32,
    window: &mut Window,
    cx: &mut App,
) {
    let run = TextRun {
        len: text.len(),
        font: font(theme::FONT_MONO),
        color: rgb(color).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(
        SharedString::from(text.to_string()),
        px(font_size),
        &[run],
        None,
    );
    let line_height = px(font_size * 1.3);
    let left = match anchor {
        TextAnchor::Left => at.x,
        TextAnchor::Middle => at.x - line.width / 2.0,
    };
    let origin = point(left, at.y - line_height / 2.0);
    // Text that fails to draw is not worth stopping the frame for.
    let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
}
