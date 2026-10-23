//! Rover pickup, destination preview, cancellation, and goal submission.
//! Model positions are previews; only release publishes a destination.
use super::local_map::point_to_local;
use super::{ActiveDrag, Dashboard};
use argos_core::{
    DragAnchor, DragEnd, DragGate, DragSpace, GoalDrag, OutdoorAdmission, PlanarPoint,
    drag_disabled_reason, drive_block,
};
use argos_gpui::{RoverMark, model_position};
use gpui_kit::{prelude::*, *};
use terra_map::{Color, GeoPosition};

impl Dashboard {
    pub(super) fn pick_model(&mut self, id: u64, event: &MouseDownEvent, cx: &mut Context<Self>) {
        self.select(id);
        if !event.modifiers.control {
            self.note = format!(
                "Rover {id} selected · control active in this window. Control-drag to place a destination"
            );
            return;
        }
        let ground = self
            .selected_mark()
            .and_then(|mark| self.view.read(cx).project(mark.latitude, mark.longitude));
        self.begin_drag(id, DragAnchor::Rover, true);
        if let (Some(active), Some(ground)) = (self.drag.as_mut(), ground) {
            active.grab_offset = Some(point(
                ground.x - event.position.x,
                ground.y - event.position.y,
            ));
        }
    }
    pub(super) fn begin_drag(&mut self, id: u64, anchor: DragAnchor, from_map: bool) {
        if self.driving {
            self.note = "Leave drive before dragging a goal".into();
            return;
        }
        let Some(mark) = self.marks.iter().find(|mark| mark.id == id).cloned() else {
            return;
        };
        if !from_map && mark.outdoor_gps {
            self.note =
                "Outdoor GPS is the source. Drag the map marker to send a WGS84 goal.".into();
            return;
        }
        let geographic = from_map && mark.outdoor_gps;
        if from_map && !geographic && mark.map_frame.is_none() {
            self.note =
                "GPS position only · use the local grid for local waypoints, or select Outdoor GPS"
                    .into();
            return;
        }
        if let Some(reason) = drive_block(&mark.hardware) {
            let level = mark
                .effective_level
                .clone()
                .or_else(|| mark.requested_level.clone());
            let gate = if geographic {
                geographic_gate(&mark, &self.link, self.selected == Some(id))
            } else {
                DragGate {
                    selected: self.selected == Some(id),
                    link_healthy: self.link == "healthy",
                    pose_fresh: mark.status == "online",
                    degraded: mark.degraded,
                    level: level.as_deref(),
                    geographic: false,
                    outdoor: false,
                    admission: None,
                }
            };
            if drag_disabled_reason(gate).is_none() {
                self.note = reason.into();
                return;
            }
        }
        let level = mark
            .effective_level
            .clone()
            .or_else(|| mark.requested_level.clone());
        let gate = if geographic {
            geographic_gate(&mark, &self.link, self.selected == Some(id))
        } else {
            DragGate {
                selected: self.selected == Some(id),
                link_healthy: self.link == "healthy",
                pose_fresh: mark.status == "online",
                degraded: mark.degraded,
                level: level.as_deref(),
                geographic: false,
                outdoor: false,
                admission: None,
            }
        };
        let (space, rover, cursor, radius) = if geographic {
            let rover = gps_point(&mark);
            let cursor = if anchor == DragAnchor::Goal {
                goal_geo(&mark).unwrap_or(rover)
            } else {
                rover
            };
            (
                DragSpace::Geographic,
                rover,
                cursor,
                Some(mark.arrival_radius),
            )
        } else {
            let rover = PlanarPoint {
                x: mark.local_x,
                y: mark.local_y,
            };
            let cursor = if anchor == DragAnchor::Goal {
                mark.goal
                    .as_ref()
                    .map(|goal| PlanarPoint {
                        x: goal.x,
                        y: goal.y,
                    })
                    .unwrap_or(rover)
            } else {
                rover
            };
            (DragSpace::Local, rover, cursor, None)
        };
        match GoalDrag::start(id, anchor, space, rover, cursor, radius, gate) {
            Ok(goal) => {
                self.note = drag_note(&goal);
                self.drag = Some(ActiveDrag {
                    goal,
                    from_map,
                    grab_offset: None,
                });
            }
            Err(reason) => self.note = reason.into(),
        }
    }
    pub(super) fn move_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(active) = self.drag.as_ref() else {
            return;
        };
        let from_map = active.from_map;
        let space = active.goal.space;
        let position = active.grab_offset.map_or(position, |offset| {
            point(position.x + offset.x, position.y + offset.y)
        });
        let next = if from_map {
            let Some((latitude, longitude)) = self.view.read(cx).ground_at(position) else {
                return;
            };
            if space == DragSpace::Geographic {
                PlanarPoint {
                    x: latitude,
                    y: longitude,
                }
            } else {
                let Some(frame) = self.selected_mark().and_then(|mark| mark.map_frame) else {
                    return;
                };
                let Ok((x, y)) = frame.local(latitude, longitude) else {
                    return;
                };
                PlanarPoint { x, y }
            }
        } else {
            let frame = *self.grid_frame.lock().unwrap();
            let Some(frame) = frame else {
                return;
            };
            let Some(grid) = self.selected_mark().and_then(|mark| mark.grid.clone()) else {
                return;
            };
            let Some((x, y)) = point_to_local(&frame, &grid, position) else {
                return;
            };
            PlanarPoint { x, y }
        };
        let Some(active) = self.drag.as_mut() else {
            return;
        };
        active.goal.move_to(next);
        self.note = drag_note(&active.goal);
        if active.from_map
            && active.goal.anchor == DragAnchor::Rover
            && let Some(mark) = self.marks.iter().find(|mark| mark.id == active.goal.id)
            && let Some(object) = self.objects.get(&mark.id)
        {
            let (latitude, longitude) = model_position(
                mark.id,
                (mark.latitude, mark.longitude),
                mark.map_frame,
                Some(&active.goal),
            );
            let altitude = self
                .map
                .surface_altitude(latitude, longitude)
                .unwrap_or_else(|| self.view.read(cx).ground_altitude());
            let _ = self
                .map
                .set_object_position(*object, GeoPosition::new(latitude, longitude, altitude));
            let _ = self
                .map
                .illuminate_3d_object(*object, Color::rgb(255, 160, 50));
        }
    }
    pub(super) fn release_drag(&mut self) {
        let Some(active) = self.drag.take() else {
            return;
        };
        let id = active.goal.id;
        let radius = active.goal.arrival_radius_m;
        let geographic = active.goal.space == DragSpace::Geographic;
        let Some(mark) = self.marks.iter().find(|mark| mark.id == id).cloned() else {
            self.note = "Drag cancelled".into();
            return;
        };
        let level = mark
            .effective_level
            .clone()
            .or_else(|| mark.requested_level.clone());
        let gate = if geographic {
            geographic_gate(&mark, &self.link, self.selected == Some(id))
        } else {
            DragGate {
                selected: self.selected == Some(id),
                link_healthy: self.link == "healthy",
                pose_fresh: mark.status == "online",
                degraded: mark.degraded,
                level: level.as_deref(),
                geographic: false,
                outdoor: false,
                admission: None,
            }
        };
        match active.goal.release(gate) {
            DragEnd::Cancelled(reason) => self.note = reason.into(),
            DragEnd::Send(goal) => {
                self.feed.publish_goal(id, goal, radius);
                self.note = match radius {
                    Some(radius) => format!("Goal sent. Arrival radius frozen at {radius:.0} m"),
                    None => "Goal sent".into(),
                };
            }
        }
    }
    pub(super) fn cancel_drag(&mut self) {
        if let Some(active) = self.drag.take() {
            self.note = match active.goal.escape() {
                DragEnd::Cancelled(reason) => reason.into(),
                DragEnd::Send(_) => "Drag cancelled".into(),
            };
        }
    }
}

pub(super) fn gps_point(mark: &RoverMark) -> PlanarPoint {
    if let Some(gps) = mark.raw_gps {
        return PlanarPoint {
            x: gps.latitude,
            y: gps.longitude,
        };
    }
    if let Some(report) = &mark.outdoor
        && let (Some(latitude), Some(longitude)) = (report.gps_latitude, report.gps_longitude)
    {
        return PlanarPoint {
            x: latitude,
            y: longitude,
        };
    }
    PlanarPoint {
        x: mark.latitude,
        y: mark.longitude,
    }
}

pub(super) fn geographic_gate<'a>(mark: &'a RoverMark, link: &str, selected: bool) -> DragGate<'a> {
    let level = mark
        .effective_level
        .as_deref()
        .or(mark.requested_level.as_deref());
    let point = gps_point(mark);
    let radius = mark.arrival_radius.clamp(2., 20.);
    DragGate {
        selected,
        link_healthy: link == "healthy",
        pose_fresh: mark.status == "online",
        degraded: mark.degraded,
        level,
        geographic: true,
        outdoor: mark.outdoor_gps,
        admission: Some(OutdoorAdmission {
            report: mark.outdoor.as_ref(),
            latitude: point.x,
            longitude: point.y,
            arrival_radius_m: radius,
            level,
            run_id: &mark.run_id,
            navigation_revision: mark.navigation_revision,
        }),
    }
}

pub(super) fn drag_note(goal: &GoalDrag) -> String {
    match goal.arrival_radius_m {
        Some(radius) => format!(
            "{:.1} m GPS · arrival radius {radius:.0} m — release sends, Esc cancels",
            goal.distance_m()
        ),
        None => format!("{:.1} m — release sends, Esc cancels", goal.distance_m()),
    }
}

pub(super) fn goal_geo(mark: &RoverMark) -> Option<PlanarPoint> {
    let goal = mark.goal.as_ref()?;
    Some(PlanarPoint {
        x: goal.latitude?,
        y: goal.longitude?,
    })
}
