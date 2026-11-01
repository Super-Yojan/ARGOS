//! Drag-to-goal. Press, move, release sends. Esc or a drop on the rover sends nothing.
//!
//! The outdoor GPS spec drafted a click and then asked for a separate confirm.
//! The dashboard uses the release as that explicit operator action instead.

use crate::{GoalRequest, OutdoorAdmission, admit_outdoor_goal, geographic_goal_refusal};

/// Ground distance that still counts as dropping the ghost on the rover.
pub const DROP_ON_SELF_METRES: f64 = 0.75;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragAnchor {
    Rover,
    Goal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragSpace {
    /// Local metres, +x north and +y west.
    Local,
    /// WGS84 degrees. Latitude is `x`, longitude is `y`.
    Geographic,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanarPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct DragGate<'a> {
    pub selected: bool,
    pub link_healthy: bool,
    pub pose_fresh: bool,
    pub degraded: bool,
    pub level: Option<&'a str>,
    pub geographic: bool,
    pub outdoor: bool,
    /// Full outdoor readiness. When set, it replaces the boolean capability flag.
    /// Latitude and longitude are the destination under review.
    pub admission: Option<OutdoorAdmission<'a>>,
}

/// Why a drag cannot start. `None` means the press may begin a drag.
pub fn drag_disabled_reason(gate: DragGate<'_>) -> Option<&'static str> {
    if !gate.selected {
        return Some("Select this rover before dragging a goal");
    }
    if !gate.link_healthy {
        return Some("Link is down");
    }
    if !gate.pose_fresh {
        return Some("Pose is stale");
    }
    if gate.geographic {
        if let Some(mut admission) = gate.admission {
            if admission.level.is_none() {
                admission.level = Some("");
            }
            if let Err(reason) = admit_outdoor_goal(admission) {
                return Some(reason);
            }
        } else if let Some(reason) = geographic_goal_refusal(gate.outdoor) {
            return Some(reason);
        } else if gate.level != Some("waypoint") {
            return Some("Outdoor goals need L3 Planned");
        }
    } else if !matches!(gate.level, Some("waypoint" | "waypoint_direct")) {
        return Some("Drag a goal from L2 Direct or L3 Planned");
    }
    if gate.degraded {
        return Some("pose is degraded; not sending a goal");
    }
    None
}

#[derive(Clone, Debug, PartialEq)]
pub struct GoalDrag {
    pub id: u64,
    pub anchor: DragAnchor,
    pub space: DragSpace,
    pub rover: PlanarPoint,
    pub cursor: PlanarPoint,
    /// Set for a geographic drag. Frozen for the life of the drag.
    pub arrival_radius_m: Option<f64>,
}

#[derive(Clone, Debug)]
pub enum DragEnd {
    Send(GoalRequest),
    Cancelled(&'static str),
}

impl GoalDrag {
    pub fn start(
        id: u64,
        anchor: DragAnchor,
        space: DragSpace,
        rover: PlanarPoint,
        cursor: PlanarPoint,
        arrival_radius_m: Option<f64>,
        gate: DragGate<'_>,
    ) -> Result<Self, &'static str> {
        if let Some(reason) = drag_disabled_reason(gate) {
            return Err(reason);
        }
        if !finite_point(rover) || !finite_point(cursor) {
            return Err("Pose is stale");
        }
        let arrival_radius_m = match space {
            DragSpace::Geographic => Some(arrival_radius_m.unwrap_or(5.).clamp(2., 20.)),
            DragSpace::Local => None,
        };
        Ok(Self {
            id,
            anchor,
            space,
            rover,
            cursor,
            arrival_radius_m,
        })
    }

    pub fn move_to(&mut self, cursor: PlanarPoint) {
        if finite_point(cursor) {
            self.cursor = cursor;
        }
    }

    pub fn distance_m(&self) -> f64 {
        match self.space {
            DragSpace::Local => {
                let dx = self.cursor.x - self.rover.x;
                let dy = self.cursor.y - self.rover.y;
                dx.hypot(dy)
            }
            DragSpace::Geographic => {
                crate::geodesic_metres(self.rover.x, self.rover.y, self.cursor.x, self.cursor.y)
            }
        }
    }

    pub fn escape(self) -> DragEnd {
        DragEnd::Cancelled("Drag cancelled")
    }

    /// Release is the explicit operator action. A geographic drop is admitted
    /// again at the cursor, using the radius frozen when the drag began.
    pub fn release(self, gate: DragGate<'_>) -> DragEnd {
        if self.distance_m() < DROP_ON_SELF_METRES {
            return DragEnd::Cancelled("Dropped on the rover; goal not sent");
        }
        let gate = reviewed_gate(&self, gate);
        if let Some(reason) = drag_disabled_reason(gate) {
            return DragEnd::Cancelled(reason);
        }
        let goal = match self.space {
            DragSpace::Local => GoalRequest::Local {
                x: self.cursor.x,
                y: self.cursor.y,
                yaw: None,
            },
            DragSpace::Geographic => GoalRequest::Geographic {
                latitude: self.cursor.x,
                longitude: self.cursor.y,
                yaw: None,
            },
        };
        DragEnd::Send(goal)
    }
}

fn reviewed_gate<'a>(drag: &GoalDrag, mut gate: DragGate<'a>) -> DragGate<'a> {
    if drag.space == DragSpace::Geographic
        && let Some(admission) = gate.admission.as_mut()
    {
        admission.latitude = drag.cursor.x;
        admission.longitude = drag.cursor.y;
        if let Some(radius) = drag.arrival_radius_m {
            admission.arrival_radius_m = radius;
        }
    }
    gate
}

fn finite_point(point: PlanarPoint) -> bool {
    point.x.is_finite() && point.y.is_finite()
}
