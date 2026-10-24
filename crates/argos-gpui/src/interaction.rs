//! Click-to-place destinations, the confirm popup's actions, and goal
//! submission. A proposal is only a preview; only Send goal publishes it.
use super::{ActiveDrag, Dashboard};
use argos_core::{
    DragAnchor, DragEnd, DragGate, DragSpace, GoalDrag, OutdoorAdmission, PlanarPoint,
    drag_disabled_reason, drive_block,
};
use argos_gpui::RoverMark;
use gpui_kit::{prelude::*, *};

impl Dashboard {
    /// A plain click on the main map proposes a destination for the selected
    /// rover. Nothing is sent until the operator confirms in the popup.
    pub(super) fn click_map(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            return;
        };
        if !self.claim.held(self.now) {
            self.note = "Take control to place a destination".into();
            return;
        }
        self.drag = None;
        self.begin_drag(id, DragAnchor::Goal, true);
        if self.drag.is_none() {
            return;
        }
        if !self.move_drag(position, cx) {
            self.drag = None;
            self.note = "No ground under that point".into();
        }
    }
    pub(super) fn begin_drag(&mut self, id: u64, anchor: DragAnchor, from_map: bool) {
        if self.driving {
            self.note = "Leave drive before placing a goal".into();
            return;
        }
        let Some(mark) = self.marks.iter().find(|mark| mark.id == id).cloned() else {
            return;
        };
        if !from_map && mark.outdoor_gps {
            self.note = "Outdoor GPS is the source. Click the map to send a WGS84 goal.".into();
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
                self.drag = Some(ActiveDrag { goal, from_map });
            }
            Err(reason) => self.note = reason.into(),
        }
    }
    /// Points the proposed destination at a window position on the map.
    /// Returns false when there is no usable ground there.
    pub(super) fn move_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(active) = self.drag.as_ref() else {
            return false;
        };
        if !active.from_map {
            return false;
        }
        let space = active.goal.space;
        let Some((latitude, longitude)) = self.view.read(cx).ground_at(position) else {
            return false;
        };
        let next = if space == DragSpace::Geographic {
            PlanarPoint {
                x: latitude,
                y: longitude,
            }
        } else {
            let Some(frame) = self.selected_mark().and_then(|mark| mark.map_frame) else {
                return false;
            };
            let Ok((x, y)) = frame.local(latitude, longitude) else {
                return false;
            };
            PlanarPoint { x, y }
        };
        let Some(active) = self.drag.as_mut() else {
            return false;
        };
        active.goal.move_to(next);
        self.note = drag_note(&active.goal);
        true
    }
    pub(super) fn release_drag(&mut self) {
        let Some(active) = self.drag.take() else {
            return;
        };
        let id = active.goal.id;
        let radius = active.goal.arrival_radius_m;
        let geographic = active.goal.space == DragSpace::Geographic;
        let Some(mark) = self.marks.iter().find(|mark| mark.id == id).cloned() else {
            self.note = "Goal not sent".into();
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
                DragEnd::Send(_) => "Goal not sent".into(),
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
            "{:.1} m GPS · arrival radius {radius:.0} m · confirm to send, Esc cancels",
            goal.distance_m()
        ),
        None => format!("{:.1} m · confirm to send, Esc cancels", goal.distance_m()),
    }
}

pub(super) fn goal_geo(mark: &RoverMark) -> Option<PlanarPoint> {
    let goal = mark.goal.as_ref()?;
    Some(PlanarPoint {
        x: goal.latitude?,
        y: goal.longitude?,
    })
}
