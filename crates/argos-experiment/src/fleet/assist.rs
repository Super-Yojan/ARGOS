//! Obstacle assist (level L1): the operator steers, and this slows the
//! rover down and stops it before it drives into something.
//!
//! The rover sends the obstacles it has mapped (`RemoteState`); the console
//! answers with the stick command, changed by [`filter`]:
//!
//! 1. Pretend the rover holds the commanded twist for [`HORIZON_S`] and
//!    follow the arc it would drive.
//! 2. Find the first moment the rover's body would touch an obstacle it is
//!    getting closer to.
//! 3. No such moment: the command passes unchanged. Otherwise scale the
//!    command down so that the rover would need the whole horizon to get
//!    there. Closer than [`STOP_S`] away: stop.
//!
//! Scaling `v` and `w` together keeps the arc, so the rover goes where the
//! operator points, only slower. An obstacle the rover is moving away from
//! never blocks, so the operator can always back out or turn on the spot.
//!
//! The network delay is inside this loop. It is good enough to keep a
//! rover off a wall at walking pace, not a substitute for watching it.

use terra_comms::RemoteState;

/// How far ahead the command is simulated, seconds.
const HORIZON_S: f64 = 1.5;
/// A touch this soon means stop, seconds. Covers the round trip to the rover.
const STOP_S: f64 = 0.4;
/// Time between simulated poses, seconds.
const STEP_S: f64 = 0.1;
/// Obstacles are kept this far from the rover's centre, metres: the circle
/// that holds the body (0.375 m on the current rover) plus a margin.
const KEEP_OUT_M: f64 = 0.5;
/// Obstacle data older than this is not trusted, seconds.
pub const MAX_STATE_AGE_S: f64 = 0.5;

/// What the assist did to a command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verdict {
    /// Nothing in the way.
    Clear,
    /// Something ahead: the command was scaled to this fraction (0..1).
    Slowed(f64),
    /// Too close to keep going that way.
    Blocked,
}

/// The stick command `(v, w)` made safe against the obstacles the rover
/// reports, and what was done to it.
pub fn filter(v: f64, w: f64, state: &RemoteState) -> (f64, f64, Verdict) {
    let Some(touch_s) = time_to_touch(v, w, state) else {
        return (v, w, Verdict::Clear);
    };
    if touch_s <= STOP_S {
        return (0.0, 0.0, Verdict::Blocked);
    }
    let fraction = ((touch_s - STOP_S) / (HORIZON_S - STOP_S)).clamp(0.0, 1.0);
    (v * fraction, w * fraction, Verdict::Slowed(fraction))
}

/// Seconds until the rover, holding `(v, w)`, touches an obstacle it is
/// approaching. `None`: not within the horizon.
fn time_to_touch(v: f64, w: f64, state: &RemoteState) -> Option<f64> {
    let (mut x, mut y, mut yaw) = (state.pose.x, state.pose.y, state.pose.yaw);
    let distance_from =
        |x: f64, y: f64, obstacle: &[f64; 2]| (obstacle[0] - x).hypot(obstacle[1] - y);
    let mut before: Vec<f64> = state
        .obstacles
        .iter()
        .map(|obstacle| distance_from(x, y, obstacle))
        .collect();

    let steps = (HORIZON_S / STEP_S).round() as usize;
    for step in 1..=steps {
        yaw += w * STEP_S;
        x += v * yaw.cos() * STEP_S;
        y += v * yaw.sin() * STEP_S;

        for (obstacle, before) in state.obstacles.iter().zip(&mut before) {
            let now = distance_from(x, y, obstacle);
            let approaching = now < *before;
            if now < KEEP_OUT_M && approaching {
                return Some(step as f64 * STEP_S);
            }
            *before = now;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use terra_comms::{Pose2, Twist};

    /// A rover at the origin facing +X, with these obstacles.
    fn rover_seeing(obstacles: &[[f64; 2]]) -> RemoteState {
        RemoteState {
            t: 0.0,
            pose: Pose2 {
                x: 0.0,
                y: 0.0,
                yaw: 0.0,
            },
            twist: Twist { v: 0.0, w: 0.0 },
            path: Vec::new(),
            obstacles: obstacles.to_vec(),
        }
    }

    #[test]
    fn open_space_changes_nothing() {
        assert_eq!(
            filter(0.4, 0.3, &rover_seeing(&[])),
            (0.4, 0.3, Verdict::Clear)
        );
        // A wall far ahead is beyond the horizon.
        assert_eq!(
            filter(0.4, 0.0, &rover_seeing(&[[5.0, 0.0]])).2,
            Verdict::Clear
        );
    }

    #[test]
    fn slows_down_then_stops_as_a_wall_gets_closer() {
        let (v_far, _, far) = filter(0.4, 0.0, &rover_seeing(&[[1.0, 0.0]]));
        assert!(
            matches!(far, Verdict::Slowed(_)) && v_far > 0.0 && v_far < 0.4,
            "{far:?}"
        );

        let (v_near, w_near, near) = filter(0.4, 0.0, &rover_seeing(&[[0.6, 0.0]]));
        assert_eq!((v_near, w_near, near), (0.0, 0.0, Verdict::Blocked));
    }

    #[test]
    fn the_operator_can_always_get_away() {
        let touching_ahead = rover_seeing(&[[0.3, 0.0]]);
        assert_eq!(
            filter(-0.4, 0.0, &touching_ahead).2,
            Verdict::Clear,
            "backing away"
        );
        assert_eq!(
            filter(0.0, 1.0, &touching_ahead).2,
            Verdict::Clear,
            "turning on the spot"
        );
        assert_eq!(
            filter(0.4, 0.0, &touching_ahead).2,
            Verdict::Blocked,
            "pushing on"
        );
    }

    #[test]
    fn an_obstacle_beside_the_arc_does_not_block() {
        assert_eq!(
            filter(0.4, 0.0, &rover_seeing(&[[0.8, 1.0]])).2,
            Verdict::Clear
        );
    }
}
