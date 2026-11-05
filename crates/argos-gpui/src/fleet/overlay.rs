//! Steady GPS placement for a phone's local map on the satellite view.
//!
//! A phone without a localization origin is drawn by pinning its local frame
//! to the GPS fix and the reported heading. Re-pinning that frame on every
//! update made the map slide with the rover between fixes (the fix stays put
//! while VIO moves the rover) and snap at each new fix, and every compass
//! wobble turned the whole grid. This holds one frame per rover instead:
//!
//! - between fixes the frame does not move, so the rover travels across the
//!   map by VIO;
//! - a new fix is matched to the pose at the time it was measured and pulls
//!   the frame only part of the way toward it, unless it is far off;
//! - heading changes turn the frame gradually, unless the change is large.
use argos_core::{LocalFrame, Pose, RawGps, geodesic_metres};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Share of the gap to a new fix that the frame moves, per fix.
const POSITION_GAIN: f64 = 0.2;
/// A fix farther than this (or three times its accuracy) re-pins at once, metres.
const POSITION_SNAP_M: f64 = 15.;
/// Time constant for following heading changes, seconds.
const HEADING_TAU_S: f64 = 3.;
/// A heading change larger than this re-pins at once, degrees.
const HEADING_SNAP_DEG: f64 = 45.;
/// A pose step longer than this between updates is a local frame jump, metres.
const POSE_JUMP_M: f64 = 1.;
/// Pose history kept to match fixes to the pose when they were measured, seconds.
const POSE_HISTORY_S: f64 = 30.;

struct Held {
    frame: LocalFrame,
    scope: String,
    fix: (f64, f64),
    poses: VecDeque<(f64, f64, f64)>,
}

/// One steady overlay frame per rover; see the module docs.
#[derive(Default)]
pub struct OverlayFrames {
    rovers: BTreeMap<u64, Held>,
}

impl OverlayFrames {
    /// Frame for drawing rover `id`'s local data under its GPS fix.
    ///
    /// `scope` names the local frame (for example the localization frame id);
    /// a new scope starts over. `now` is seconds on any steady clock.
    pub fn place(
        &mut self,
        id: u64,
        scope: &str,
        now: f64,
        pose: &Pose,
        gps: &RawGps,
        heading_deg: f64,
    ) -> Option<LocalFrame> {
        let fix = (gps.latitude, gps.longitude);
        let held = self.rovers.get_mut(&id).filter(|held| held.scope == scope);
        let Some(held) = held else {
            let frame =
                LocalFrame::pinned(fix.0, fix.1, pose.x, pose.y, pose.yaw, heading_deg).ok()?;
            self.rovers.insert(
                id,
                Held {
                    frame,
                    scope: scope.to_string(),
                    fix,
                    poses: VecDeque::from([(now, pose.x, pose.y)]),
                },
            );
            return Some(frame);
        };

        // Where the held frame puts the rover now. After a local frame jump,
        // keep the rover where it was instead of letting it leap with VIO.
        let (last_t, last_x, last_y) = *held.poses.back()?;
        let jumped = (pose.x - last_x).hypot(pose.y - last_y) > POSE_JUMP_M;
        let (mut latitude, mut longitude) = if jumped {
            held.poses.clear();
            held.frame.geographic(last_x, last_y).ok()?
        } else {
            held.frame.geographic(pose.x, pose.y).ok()?
        };
        held.poses.push_back((now, pose.x, pose.y));
        while held
            .poses
            .front()
            .is_some_and(|(t, ..)| now - t > POSE_HISTORY_S)
        {
            held.poses.pop_front();
        }

        let shown = held.frame.heading_degrees(pose.yaw);
        let turn = (heading_deg - shown + 180.).rem_euclid(360.) - 180.;
        let share = if turn.abs() > HEADING_SNAP_DEG {
            1.
        } else {
            1. - (-(now - last_t).clamp(0., 1.) / HEADING_TAU_S).exp()
        };
        let heading = shown + share * turn;

        if fix != held.fix {
            held.fix = fix;
            // The fix describes where the rover was when it was measured.
            let measured_at = now - gps.age_s.unwrap_or(0.);
            let (_, fx, fy) = held
                .poses
                .iter()
                .min_by(|a, b| {
                    (a.0 - measured_at)
                        .abs()
                        .total_cmp(&(b.0 - measured_at).abs())
                })
                .copied()
                .unwrap_or((now, pose.x, pose.y));
            let target = LocalFrame::pinned(fix.0, fix.1, fx, fy, pose.yaw, heading)
                .ok()?
                .geographic(pose.x, pose.y)
                .ok()?;
            let error = geodesic_metres(latitude, longitude, target.0, target.1);
            let snap = POSITION_SNAP_M.max(3. * gps.accuracy_m.unwrap_or(0.));
            let gain = if error > snap { 1. } else { POSITION_GAIN };
            latitude += gain * (target.0 - latitude);
            longitude += gain * (target.1 - longitude);
        }

        held.frame =
            LocalFrame::pinned(latitude, longitude, pose.x, pose.y, pose.yaw, heading).ok()?;
        Some(held.frame)
    }

    /// Forget rovers that left the fleet.
    pub fn retain(&mut self, ids: &BTreeSet<u64>) {
        self.rovers.retain(|id, _| ids.contains(id));
    }
}
