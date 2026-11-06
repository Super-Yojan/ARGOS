//! Chase camera: ride behind a followed object, like a car's driving view.
//!
//! The app sends the object's position and bearing at its own rate (ARGOS
//! syncs every 100 ms). Every frame the camera eases toward a point a little
//! ahead of the object and swings around behind it, so it glides between
//! updates instead of jumping. Dragging orbits around the object; the orbit
//! is kept as an offset from "straight behind".

use super::{
    api::Follow,
    camera::{Camera, EARTH_RADIUS, MapMode, ecef},
};
use std::f64::consts::{PI, TAU};

/// Starting distance behind the object, metres.
pub const CHASE_DISTANCE: f64 = 5.;
/// Starting pitch, radians above the horizon (about 22°).
pub const CHASE_PITCH: f64 = 0.38;
/// Time constants: position settles fast, heading a little slower so turns
/// read as a swing, not a snap.
const POSITION_TAU: f64 = 0.12;
const HEADING_TAU: f64 = 0.35;
/// Farther than this from the object, jump instead of easing (first frame,
/// GPS re-anchor, a different rover).
const SNAP_METRES: f64 = 30.;
/// Below these the camera is settled and needs no new frame.
const SETTLED_METRES: f64 = 0.002;
const SETTLED_RADIANS: f64 = 0.0005;

pub(super) struct Chase {
    pub target: Follow,
    /// Orbit offset from straight behind, radians.
    offset: f64,
    /// Orbit view to restore when following ends.
    saved: (f64, f64, f64),
    /// The next step jumps to the target.
    snap: bool,
}

impl Chase {
    pub fn begin(camera: &mut Camera, target: Follow) -> Self {
        let saved = (camera.distance, camera.pitch, camera.heading);
        camera.chase = true;
        if camera.mode == MapMode::ThreeD {
            camera.distance = CHASE_DISTANCE;
            camera.pitch = CHASE_PITCH;
        }
        Self {
            target,
            offset: 0.,
            saved,
            snap: true,
        }
    }
    /// Back to the orbit view the operator had, centred where the object is.
    pub fn end(self, camera: &mut Camera) {
        camera.chase = false;
        if camera.mode == MapMode::ThreeD {
            let (distance, pitch, heading) = self.saved;
            camera.distance = distance;
            camera.pitch = pitch;
            camera.heading = heading;
        }
        camera.clamp_distance();
    }
    /// Drag: swing around the object and tilt.
    pub fn orbit(&mut self, camera: &mut Camera, x: f64, y: f64) {
        if camera.mode == MapMode::TwoD {
            return;
        }
        self.offset = wrap(self.offset - x * 0.005);
        camera.pitch = (camera.pitch + y * 0.005).clamp(0.1, 1.55);
    }
    /// Straight behind at the starting distance and pitch.
    pub fn reset(&mut self, camera: &mut Camera) {
        self.offset = 0.;
        if camera.mode == MapMode::ThreeD {
            camera.distance = CHASE_DISTANCE;
            camera.pitch = CHASE_PITCH;
        }
        self.snap = true;
    }
    /// After a 2D/3D switch: the close distance only applies in 3D.
    pub fn mode_changed(&mut self, camera: &mut Camera) {
        if camera.mode == MapMode::ThreeD {
            camera.distance = CHASE_DISTANCE;
        }
        camera.clamp_distance();
        self.snap = true;
    }
    /// Move the camera `dt` seconds toward its goal. Returns whether it moved
    /// enough to need a new frame.
    pub fn step(&mut self, camera: &mut Camera, dt: f64) -> bool {
        let three_d = camera.mode == MapMode::ThreeD;
        let bearing = self.target.bearing.to_radians();
        let look = bearing + self.offset;
        // Look at a point ahead of the object, along the view, so it sits low
        // and centred in the frame with the way ahead visible, as in a car's
        // driving view. Orbiting keeps it centred.
        let lead = if three_d { camera.distance * 0.25 } else { 0. };
        let (latitude, longitude) = offset(
            self.target.position.latitude,
            self.target.position.longitude,
            lead * look.cos(),
            lead * look.sin(),
        );
        let altitude = self.target.position.altitude;
        let (north, east) = metres_between(camera.latitude, camera.longitude, latitude, longitude);
        let away = north.hypot(east);
        let snap = self.snap || away > SNAP_METRES;
        self.snap = false;
        let position_blend = if snap { 1. } else { blend(dt, POSITION_TAU) };
        let heading_blend = if snap { 1. } else { blend(dt, HEADING_TAU) };
        let turn = if three_d {
            wrap(look - camera.heading)
        } else {
            0.
        };
        let climb = altitude - camera.altitude;
        if away < SETTLED_METRES && turn.abs() < SETTLED_RADIANS && climb.abs() < SETTLED_METRES {
            return false;
        }
        let (new_latitude, new_longitude) = offset(
            camera.latitude,
            camera.longitude,
            north * position_blend,
            east * position_blend,
        );
        camera.latitude = new_latitude;
        camera.longitude = new_longitude;
        camera.altitude += climb * position_blend;
        camera.heading = wrap(camera.heading + turn * heading_blend);
        camera.target = ecef(camera.latitude, camera.longitude, camera.altitude);
        true
    }
}

/// Fraction of the remaining gap to close in `dt` for time constant `tau`.
fn blend(dt: f64, tau: f64) -> f64 {
    if !dt.is_finite() || dt <= 0. {
        return 0.;
    }
    1. - (-dt / tau).exp()
}

/// Angle in (-π, π].
fn wrap(angle: f64) -> f64 {
    let wrapped = (angle + PI).rem_euclid(TAU) - PI;
    if wrapped == -PI { PI } else { wrapped }
}

/// Local north and east metres from one point to another (small distances).
fn metres_between(lat0: f64, lon0: f64, lat1: f64, lon1: f64) -> (f64, f64) {
    let north = (lat1 - lat0).to_radians() * EARTH_RADIUS;
    let dlon = (lon1 - lon0 + 180.).rem_euclid(360.) - 180.;
    let east = dlon.to_radians() * EARTH_RADIUS * lat0.to_radians().cos().max(0.05);
    (north, east)
}

fn offset(latitude: f64, longitude: f64, north: f64, east: f64) -> (f64, f64) {
    let new_latitude = (latitude + (north / EARTH_RADIUS).to_degrees()).clamp(-85., 85.);
    let new_longitude = (longitude
        + (east / (EARTH_RADIUS * latitude.to_radians().cos().max(0.05))).to_degrees()
        + 180.)
        .rem_euclid(360.)
        - 180.;
    (new_latitude, new_longitude)
}

#[cfg(test)]
#[path = "../../../../tests/terra-map/unit/maps_chase_tests.rs"]
mod tests;
