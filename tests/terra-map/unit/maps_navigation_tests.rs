use super::super::camera::MapMode;
use super::*;
#[test]
fn navigation_crosses_date_line_and_preserves_camera_settings() {
    let mut camera = Camera::new(20., 179., 100., 700., 60., 40.);
    camera.set_mode(MapMode::TwoD);
    let flight = Flight::new(
        &camera,
        GeoPosition::new(22., -179., f64::NAN),
        Duration::from_secs(1),
    );
    assert!(!flight.at(&mut camera, Duration::from_millis(500)));
    assert_eq!(camera.longitude, -180.);
    assert!(flight.at(&mut camera, Duration::from_secs(1)));
    assert_eq!(camera.latitude, 22.);
    assert_eq!(camera.altitude, 100.);
    assert_eq!(camera.distance, 700.);
    assert_eq!(camera.mode, MapMode::TwoD);
    camera.set_mode(MapMode::ThreeD);
    assert!((camera.heading.to_degrees() - 60.).abs() < 1e-9);
}
