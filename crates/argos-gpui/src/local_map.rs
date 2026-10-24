//! Local occupancy grid and its pointer/coordinate conversions.
use super::Dashboard;
use argos_core::{
    CellClass, DragAnchor, HEADING_HOLD, HEADING_HOLD_LABEL, Travel, pose_warning_text,
};
use argos_gpui::RoverMark;
use gpui_kit::{prelude::*, *};
use std::sync::{Arc, Mutex};

pub(super) fn local_grid(
    mark: &RoverMark,
    drag: Option<(u64, f64, f64, f64, Option<f64>)>,
    grid_frame: Arc<Mutex<Option<Bounds<Pixels>>>>,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let Some(map) = mark.grid.clone() else {
        return div()
            .text_size(px(12.))
            .text_color(rgb(super::palette::MUTED))
            .child("No occupancy yet")
            .into_any_element();
    };
    let cols = map.cols;
    let rows = map.rows;
    let cell = 248. / cols.max(rows).max(1) as f32;
    let width = cell * cols as f32;
    let height = cell * rows as f32;
    let id = mark.id;
    let pose = (mark.local_x, mark.local_y);
    let route = mark.route.clone();
    let goal = mark.goal.as_ref().map(|goal| (goal.x, goal.y));
    let warn =
        pose_warning_text(mark.command_origin, mark.degraded, &mark.degrade_reason).is_some();
    let drag_here = drag
        .filter(|item| item.0 == id)
        .map(|item| (item.1, item.2, item.4));
    let rover_at = local_pixel(width, height, &map, pose.0, pose.1);
    let goal_at = goal.map(|(x, y)| local_pixel(width, height, &map, x, y));
    div()
        .relative()
        .w(px(width))
        .h(px(height))
        .bg(rgb(cell_color(CellClass::Unknown)))
        .child(grid_overlay(
            map, pose, route, goal, drag_here, warn, grid_frame,
        ))
        .child(drag_handle_at(id, DragAnchor::Rover, rover_at, false, cx))
        .when_some(goal_at, |grid, at| {
            grid.child(drag_handle_at(id, DragAnchor::Goal, at, false, cx))
        })
        .when(mark.hold.as_deref() == Some(HEADING_HOLD), |grid| {
            grid.child(
                div()
                    .absolute()
                    .bottom(px(2.))
                    .left(px(2.))
                    .text_size(px(10.))
                    .text_color(rgb(0xe7b15a))
                    .child(HEADING_HOLD_LABEL),
            )
        })
        .into_any_element()
}

pub(super) fn cell_color(class: CellClass) -> u32 {
    match class {
        CellClass::Unknown => 0x6e7c84,
        CellClass::Free => 0xd7efe0,
        CellClass::Occupied => 0x12161a,
    }
}

fn grid_overlay(
    map: argos_core::LocalMap,
    pose: (f64, f64),
    route: Vec<argos_core::RouteSample>,
    goal: Option<(f64, f64)>,
    drag: Option<(f64, f64, Option<f64>)>,
    warn: bool,
    frames: Arc<Mutex<Option<Bounds<Pixels>>>>,
) -> impl IntoElement {
    canvas(
        move |bounds, _, _| {
            *frames.lock().unwrap() = Some(bounds);
            bounds
        },
        move |bounds, _, window, _| {
            // Paint observed cells directly: a phone grid has 40,000 cells,
            // and does not need a retained layout element for every sample.
            let cell_width = bounds.size.width / map.cols.max(1) as f32;
            let cell_height = bounds.size.height / map.rows.max(1) as f32;
            for (index, class) in map.cells.iter().copied().enumerate() {
                if class == CellClass::Unknown {
                    continue;
                }
                let col = index as u32 % map.cols;
                let row = index as u32 / map.cols;
                window.paint_quad(quad(
                    Bounds {
                        origin: point(
                            bounds.origin.x + cell_width * col as f32,
                            bounds.origin.y + cell_height * (map.rows - 1 - row) as f32,
                        ),
                        size: size(cell_width, cell_height),
                    },
                    Corners::default(),
                    rgb(cell_color(class)),
                    Edges::default(),
                    transparent_black(),
                    BorderStyle::default(),
                ));
            }
            for pair in route.windows(2) {
                let travel = pair[1].travel;
                let color = if travel == Travel::Reverse {
                    rgb(0xf0c36a)
                } else {
                    rgb(0x7eb6ff)
                };
                stroke(
                    window,
                    local_to_bounds(&bounds, &map, pair[0].x, pair[0].y),
                    local_to_bounds(&bounds, &map, pair[1].x, pair[1].y),
                    color,
                    travel == Travel::Reverse,
                );
            }
            if let Some((x, y)) = goal {
                paint_dot(
                    window,
                    local_to_bounds(&bounds, &map, x, y),
                    8.,
                    rgb(0x3dcc9a),
                );
            }
            if let Some((x, y, radius)) = drag {
                stroke(
                    window,
                    local_to_bounds(&bounds, &map, pose.0, pose.1),
                    local_to_bounds(&bounds, &map, x, y),
                    rgb(0xf0c36a),
                    false,
                );
                if let Some(radius) = radius {
                    for step in 0..16 {
                        let angle = step as f64 / 16. * std::f64::consts::TAU;
                        paint_dot(
                            window,
                            local_to_bounds(
                                &bounds,
                                &map,
                                x + radius * angle.cos(),
                                y + radius * angle.sin(),
                            ),
                            3.,
                            rgb(0xf0c36a),
                        );
                    }
                }
                paint_dot(
                    window,
                    local_to_bounds(&bounds, &map, x, y),
                    8.,
                    rgb(0xf0c36a),
                );
            }
            paint_dot(
                window,
                local_to_bounds(&bounds, &map, pose.0, pose.1),
                7.,
                rgb(0xf0b429),
            );
            if warn {
                paint_dot(
                    window,
                    local_to_bounds(&bounds, &map, pose.0, pose.1),
                    3.,
                    rgb(0xe24b4b),
                );
            }
        },
    )
    .absolute()
    .size_full()
}

fn local_to_bounds(
    bounds: &Bounds<Pixels>,
    map: &argos_core::LocalMap,
    x: f64,
    y: f64,
) -> Point<Pixels> {
    let fx = if map.span_x.abs() < 1e-6 {
        0.5
    } else {
        ((x - map.origin_x) / map.span_x).clamp(0., 1.)
    };
    let fy = if map.span_y.abs() < 1e-6 {
        0.5
    } else {
        ((y - map.origin_y) / map.span_y).clamp(0., 1.)
    };
    point(
        bounds.origin.x + bounds.size.width * fx as f32,
        bounds.origin.y + bounds.size.height * (1. - fy as f32),
    )
}

fn local_pixel(
    width: f32,
    height: f32,
    map: &argos_core::LocalMap,
    x: f64,
    y: f64,
) -> Point<Pixels> {
    let bounds = Bounds {
        origin: point(px(0.), px(0.)),
        size: size(px(width), px(height)),
    };
    local_to_bounds(&bounds, map, x, y)
}

pub(super) fn point_to_local(
    frame: &Bounds<Pixels>,
    map: &argos_core::LocalMap,
    position: Point<Pixels>,
) -> Option<(f64, f64)> {
    let width = f32::from(frame.size.width);
    let height = f32::from(frame.size.height);
    if width <= 0. || height <= 0. {
        return None;
    }
    let fx = (f32::from(position.x - frame.origin.x) / width).clamp(0., 1.) as f64;
    let fy = (f32::from(position.y - frame.origin.y) / height).clamp(0., 1.) as f64;
    Some((
        map.origin_x + fx * map.span_x,
        map.origin_y + (1. - fy) * map.span_y,
    ))
}

fn paint_dot(window: &mut Window, at: Point<Pixels>, span: f32, color: Rgba) {
    window.paint_quad(quad(
        Bounds {
            origin: point(at.x - px(span * 0.5), at.y - px(span * 0.5)),
            size: size(px(span), px(span)),
        },
        Corners::default(),
        color,
        Edges::default(),
        transparent_black(),
        BorderStyle::default(),
    ));
}

pub(super) fn stroke(
    window: &mut Window,
    from: Point<Pixels>,
    to: Point<Pixels>,
    color: Rgba,
    dashed: bool,
) {
    let dx = f32::from(to.x - from.x);
    let dy = f32::from(to.y - from.y);
    let len = dx.hypot(dy);
    if len < 1. {
        return;
    }
    let steps = (len / 5.).max(1.) as i32;
    for step in 0..steps {
        if dashed && step % 2 == 0 {
            continue;
        }
        let t = step as f32 / steps as f32;
        let at = point(from.x + px(dx * t), from.y + px(dy * t));
        window.paint_quad(quad(
            Bounds {
                origin: point(at.x - px(1.5), at.y - px(1.5)),
                size: size(px(3.), px(3.)),
            },
            Corners::default(),
            color,
            Edges::default(),
            transparent_black(),
            BorderStyle::default(),
        ));
    }
    if dashed {
        let at = point(from.x + px(dx * 0.72), from.y + px(dy * 0.72));
        window.paint_quad(quad(
            Bounds {
                origin: point(at.x - px(4.), at.y - px(4.)),
                size: size(px(8.), px(8.)),
            },
            Corners::default(),
            color,
            Edges::default(),
            transparent_black(),
            BorderStyle::default(),
        ));
    }
}

fn drag_handle_at(
    id: u64,
    anchor: DragAnchor,
    at: Point<Pixels>,
    on_map: bool,
    cx: &mut Context<Dashboard>,
) -> impl IntoElement {
    let slot = match anchor {
        DragAnchor::Rover => 0,
        DragAnchor::Goal => 1,
    } + if on_map { 2 } else { 0 };
    div()
        .id(("drag", id.saturating_mul(4).saturating_add(slot)))
        .absolute()
        .left(at.x - px(9.))
        .top(at.y - px(9.))
        .w(px(18.))
        .h(px(18.))
        .rounded_md()
        .cursor_pointer()
        .bg(rgb(if anchor == DragAnchor::Goal {
            0x3dcc9a
        } else {
            0xf0b429
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.begin_drag(id, anchor, on_map);
                cx.stop_propagation();
                cx.notify();
            }),
        )
}
