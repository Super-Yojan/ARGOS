use super::*;
fn patch(height: f64) -> [DVec3; 3] {
    let base = ecef(0., 0., height);
    [
        base + DVec3::new(0., -10., -10.),
        base + DVec3::new(0., 10., -10.),
        base + DVec3::new(0., 0., 10.),
    ]
}
#[test]
fn picks_visible_surface_above_reference_and_misses_outside() {
    let surface = Surface::from_triangles(vec![patch(140.), patch(160.)]);
    assert!((surface.altitude(0., 0.).unwrap() - 160.).abs() < 1e-5);
    assert!(surface.altitude(1., 1.).is_none());
}
#[test]
fn refinement_replaces_old_height_and_negative_heights_work() {
    let surface = Surface::from_triangles(vec![patch(-30.)]);
    assert!((surface.altitude(0., 0.).unwrap() + 30.).abs() < 1e-5);
    assert!(Surface::from_triangles(vec![]).altitude(0., 0.).is_none());
}
