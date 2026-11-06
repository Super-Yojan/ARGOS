use super::*;
use crate::maps::api::GeoPosition;
use crate::maps::camera::CHASE_MIN_DISTANCE;

fn follow(latitude: f64, longitude: f64, bearing: f64) -> Follow {
    Follow {
        position: GeoPosition::new(latitude, longitude, 40.),
        bearing,
    }
}

fn camera() -> Camera {
    Camera::new(38.8297, -77.3075, 40., 400., 0., 50.)
}

#[test]
fn begin_comes_close_and_end_restores_the_orbit_view() {
    let mut camera = camera();
    let before = (camera.distance, camera.pitch, camera.heading);
    let chase = Chase::begin(&mut camera, follow(38.8297, -77.3075, 90.));
    assert!(camera.chase);
    assert_eq!(camera.distance, CHASE_DISTANCE);
    assert_eq!(camera.pitch, CHASE_PITCH);
    chase.end(&mut camera);
    assert!(!camera.chase);
    assert_eq!((camera.distance, camera.pitch, camera.heading), before);
}

#[test]
fn first_step_jumps_behind_the_object_facing_its_bearing() {
    let mut camera = camera();
    let target = follow(38.8300, -77.3080, 90.);
    let mut chase = Chase::begin(&mut camera, target);
    assert!(chase.step(&mut camera, 0.016));
    // Facing east: the look point is ahead (east) of the rover, same latitude.
    assert!((camera.latitude - target.position.latitude).abs() < 1e-7);
    assert!(camera.longitude > target.position.longitude);
    assert!((camera.heading - 90f64.to_radians()).abs() < 1e-9);
    // Eye sits behind (west of) the rover, above the ground.
    let view = camera.view(800, 600);
    let rover = ecef(38.8300, -77.3080, 40.);
    let look = camera.target;
    assert!(view.eye.distance(rover) < view.eye.distance(look));
}

#[test]
fn later_steps_ease_and_take_the_short_way_round() {
    let mut camera = camera();
    let mut chase = Chase::begin(&mut camera, follow(38.8297, -77.3075, 350.));
    chase.step(&mut camera, 0.016);
    chase.target = follow(38.8297, -77.3075, 10.);
    chase.step(&mut camera, 0.016);
    let heading = camera.heading.to_degrees().rem_euclid(360.);
    // From 350° toward 10° it passes through 0°, not back through 180°.
    assert!(!(10.0..350.0).contains(&heading), "heading {heading}");
    for _ in 0..200 {
        chase.step(&mut camera, 0.016);
    }
    assert!((camera.heading.to_degrees().rem_euclid(360.) - 10.).abs() < 0.05);
    // Settled: no more frames needed.
    assert!(!chase.step(&mut camera, 0.016));
}

#[test]
fn a_far_move_snaps_instead_of_sliding() {
    let mut camera = camera();
    let mut chase = Chase::begin(&mut camera, follow(38.8297, -77.3075, 0.));
    chase.step(&mut camera, 0.016);
    // About 110 m north: a re-anchor or a different rover.
    chase.target = follow(38.8307, -77.3075, 0.);
    chase.step(&mut camera, 0.016);
    let ahead = (CHASE_DISTANCE * 0.25 / EARTH_RADIUS).to_degrees();
    assert!((camera.latitude - (38.8307 + ahead)).abs() < 1e-7);
}

#[test]
fn drag_orbits_around_the_object_and_reset_goes_back_behind() {
    let mut camera = camera();
    let mut chase = Chase::begin(&mut camera, follow(38.8297, -77.3075, 0.));
    chase.step(&mut camera, 0.016);
    chase.orbit(&mut camera, -100., 0.);
    for _ in 0..200 {
        chase.step(&mut camera, 0.016);
    }
    assert!((camera.heading - 0.5).abs() < 1e-3);
    camera.zoom(5000.);
    assert_eq!(camera.distance, CHASE_MIN_DISTANCE);
    chase.reset(&mut camera);
    chase.step(&mut camera, 0.016);
    assert!(camera.heading.abs() < 1e-9);
    assert_eq!(camera.distance, CHASE_DISTANCE);
}

#[test]
fn two_d_follow_only_recentres_and_keeps_the_far_limit() {
    let mut camera = camera();
    camera.set_mode(MapMode::TwoD);
    let mut chase = Chase::begin(&mut camera, follow(38.8300, -77.3080, 90.));
    assert_eq!(camera.distance, 400.);
    chase.step(&mut camera, 0.016);
    assert!((camera.latitude - 38.8300).abs() < 1e-9);
    assert!((camera.longitude + 77.3080).abs() < 1e-9);
    assert_eq!(camera.heading, 0.);
    camera.zoom(50_000.);
    assert_eq!(camera.distance, crate::maps::camera::MIN_DISTANCE);
}

#[test]
fn close_chase_view_keeps_the_object_in_front_of_the_near_plane() {
    let mut camera = camera();
    let mut chase = Chase::begin(&mut camera, follow(38.8297, -77.3075, 0.));
    camera.distance = CHASE_MIN_DISTANCE;
    chase.step(&mut camera, 0.016);
    let (x, y) = camera
        .project_ground(800, 600, 38.8297, -77.3075)
        .expect("rover is on screen");
    assert!((x - 400.).abs() < 2.);
    // Rover sits below the centre, as in a driving view.
    assert!(y > 300.);
}

#[test]
fn orbiting_keeps_the_object_centred() {
    let mut camera = camera();
    let mut chase = Chase::begin(&mut camera, follow(38.8297, -77.3075, 0.));
    chase.step(&mut camera, 0.016);
    chase.orbit(&mut camera, -300., 0.);
    for _ in 0..300 {
        chase.step(&mut camera, 0.016);
    }
    let (x, _) = camera
        .project_ground(800, 600, 38.8297, -77.3075)
        .expect("rover is on screen");
    assert!((x - 400.).abs() < 2., "x {x}");
}
