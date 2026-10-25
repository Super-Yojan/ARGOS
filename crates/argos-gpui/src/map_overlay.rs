//! Geographic occupancy, route, and destination overlays on the 3D map.
use super::Dashboard;
use super::local_map::{cell_color, stroke};
use argos_core::{CellClass, DragSpace, Travel, geodesic_metres};
use argos_gpui::{RoverMark, occupancy_runs};
use gpui_kit::{prelude::*, *};
use terra_map::MapView;

pub(super) fn map_paint(dashboard: &Dashboard) -> impl IntoElement {
    let view = dashboard.view.clone();
    let drag = dashboard.drag.as_ref().map(|active| active.goal.clone());
    let marks = dashboard.marks.clone();
    let show_occupancy = dashboard.show_occupancy;
    canvas(
        |bounds, _, _| bounds,
        move |_bounds, _, window, cx| {
            let map = view.read(cx);
            if show_occupancy {
                window.with_content_mask(Some(gpui::ContentMask { bounds: _bounds }), |window| {
                    paint_occupancy(window, map, &marks);
                });
            }
            for mark in &marks {
                let Some(origin) = mark.overlay_frame else {
                    continue;
                };
                let mut previous: Option<Point<Pixels>> = None;
                for sample in &mark.route {
                    let Ok((latitude, longitude)) = origin.geographic(sample.x, sample.y) else {
                        previous = None;
                        continue;
                    };
                    let Some(at) = map.project(latitude, longitude) else {
                        previous = None;
                        continue;
                    };
                    if let Some(from) = previous {
                        stroke(
                            window,
                            from,
                            at,
                            if sample.travel == Travel::Reverse {
                                rgb(0xf0c36a)
                            } else {
                                rgb(0x7eb6ff)
                            },
                            sample.travel == Travel::Reverse,
                        );
                    }
                    previous = Some(at);
                }
            }
            let Some(goal) = &drag else {
                return;
            };
            let Some(mark) = marks.iter().find(|mark| mark.id == goal.id) else {
                return;
            };
            let cursor = match goal.space {
                DragSpace::Geographic => Some((goal.cursor.x, goal.cursor.y)),
                DragSpace::Local => mark
                    .map_frame
                    .and_then(|frame| frame.geographic(goal.cursor.x, goal.cursor.y).ok()),
            };
            let Some((latitude, longitude)) = cursor else {
                return;
            };
            let Some(ghost) = map.project(latitude, longitude) else {
                return;
            };
            let rover_at = map.project(mark.latitude, mark.longitude);
            if let Some(rover) = rover_at {
                stroke(window, rover, ghost, rgb(0xf0c36a), false);
            }
            if let Some(radius) = goal.arrival_radius_m {
                let mut metres = ground_metres_per_pixel(map, latitude, longitude);
                if let Some(rover) = rover_at {
                    let span = f32::from(ghost.x - rover.x).hypot(f32::from(ghost.y - rover.y));
                    let distance = goal.distance_m();
                    if distance > 2. && span > 8. {
                        metres = distance / f64::from(span);
                    }
                }
                let pixels = (radius / metres).clamp(8., 240.) as f32;
                for step in 0..24 {
                    let angle = step as f32 / 24. * std::f32::consts::TAU;
                    let at = point(
                        ghost.x + px(pixels * angle.cos()),
                        ghost.y + px(pixels * angle.sin()),
                    );
                    window.paint_quad(quad(
                        Bounds {
                            origin: point(at.x - px(2.), at.y - px(2.)),
                            size: size(px(4.), px(4.)),
                        },
                        Corners::default(),
                        rgb(0xf0c36a),
                        Edges::default(),
                        transparent_black(),
                        BorderStyle::default(),
                    ));
                }
            }
            window.paint_quad(quad(
                Bounds {
                    origin: point(ghost.x - px(6.), ghost.y - px(6.)),
                    size: size(px(12.), px(12.)),
                },
                Corners::default(),
                rgb(0xf0c36a),
                Edges::default(),
                transparent_black(),
                BorderStyle::default(),
            ));
        },
    )
    // Pin to the map's corner. Without an inset it lays out below the map
    // view, and the occupancy content mask then clips every cell.
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Local occupancy draped under each rover through its overlay frame: the
/// localization origin, or the GPS fix and fused heading outdoors. Cells stay
/// in the rover frame and are only as well placed as that fix and heading.
fn paint_occupancy(window: &mut Window, map: &MapView, marks: &[RoverMark]) {
    for mark in marks {
        let Some(origin) = mark.overlay_frame else {
            continue;
        };
        let Some(grid) = &mark.grid else {
            continue;
        };
        if grid.cols == 0 {
            continue;
        }
        let half_x = grid.span_x / f64::from(grid.cols) * 0.5;
        let half_y = grid.span_y / f64::from(grid.rows) * 0.5;
        for (class, start, end) in occupancy_runs(grid) {
            let (Some(&(x0, y)), Some(&(x1, _))) = (grid.centers.get(start), grid.centers.get(end))
            else {
                continue;
            };
            let corners: Option<Vec<_>> = [
                (x0 - half_x, y - half_y),
                (x1 + half_x, y - half_y),
                (x1 + half_x, y + half_y),
                (x0 - half_x, y + half_y),
            ]
            .into_iter()
            .map(|(x, y)| {
                let (latitude, longitude) = origin.geographic(x, y).ok()?;
                // A span may cross the viewport with both ends outside it.
                // Clip its painted polygon, not its individual vertices.
                map.project_unclipped(latitude, longitude)
            })
            .collect();
            let Some(corners) = corners else {
                continue;
            };
            let mut color = rgb(cell_color(class));
            color.a = match class {
                CellClass::Free => 0.38,
                CellClass::Occupied => 0.78,
                CellClass::Unknown => 0.,
            };
            let mut path = gpui::PathBuilder::fill();
            path.move_to(corners[0]);
            for corner in &corners[1..] {
                path.line_to(*corner);
            }
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        }
    }
}

/// Ground metres per pixel from the camera, measured near a coordinate.
/// The 3D view does not publish a ground scale, so two projected points supply it.
fn ground_metres_per_pixel(map: &MapView, latitude: f64, longitude: f64) -> f64 {
    let fallback = map.metres_per_pixel().unwrap_or(1.).max(0.05);
    let Some(origin) = map.project(latitude, longitude) else {
        return fallback;
    };
    // A long step can leave the 3D frustum and make project() return nothing.
    for step in [0.00002_f64, -0.00002, 0.00005] {
        let Some(other) = map.project(latitude + step, longitude) else {
            continue;
        };
        let pixels = f32::from(other.x - origin.x).hypot(f32::from(other.y - origin.y));
        let metres = geodesic_metres(latitude, longitude, latitude + step, longitude);
        if metres.is_finite() && metres > 0.2 && pixels > 2. {
            return (metres / f64::from(pixels)).max(0.02);
        }
    }
    fallback
}
