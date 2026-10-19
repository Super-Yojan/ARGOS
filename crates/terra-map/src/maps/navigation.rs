use super::{
    api::GeoPosition,
    camera::{Camera, ecef},
};
use std::time::{Duration, Instant};

pub(super) struct Flight {
    start: GeoPosition,
    end: GeoPosition,
    started: Instant,
    duration: Duration,
}
impl Flight {
    pub fn new(camera: &Camera, mut end: GeoPosition, duration: Duration) -> Self {
        if end.altitude.is_nan() {
            end.altitude = camera.altitude;
        }
        Self {
            start: GeoPosition::new(camera.latitude, camera.longitude, camera.altitude),
            end,
            started: Instant::now(),
            duration,
        }
    }
    pub fn advance(&self, camera: &mut Camera) -> bool {
        self.at(camera, self.started.elapsed())
    }
    fn at(&self, camera: &mut Camera, elapsed: Duration) -> bool {
        let t = if self.duration.is_zero() {
            1.
        } else {
            (elapsed.as_secs_f64() / self.duration.as_secs_f64()).min(1.)
        };
        let ease = t * t * (3. - 2. * t);
        let longitude_delta =
            (self.end.longitude - self.start.longitude + 180.).rem_euclid(360.) - 180.;
        camera.latitude = self.start.latitude + (self.end.latitude - self.start.latitude) * ease;
        camera.longitude =
            (self.start.longitude + longitude_delta * ease + 180.).rem_euclid(360.) - 180.;
        camera.altitude = self.start.altitude + (self.end.altitude - self.start.altitude) * ease;
        camera.target = ecef(camera.latitude, camera.longitude, camera.altitude);
        t >= 1.
    }
}
#[cfg(test)]
mod tests {
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
}
