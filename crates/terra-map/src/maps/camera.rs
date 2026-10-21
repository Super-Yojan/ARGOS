use glam::{DMat4, DVec3, DVec4};

pub const EARTH_RADIUS: f64 = 6_378_137.0;
pub const FOV: f64 = 50.0_f64.to_radians();

pub fn ecef(latitude: f64, longitude: f64, altitude: f64) -> DVec3 {
    let (lat, lon) = (latitude.to_radians(), longitude.to_radians());
    let e2 = 6.694_379_990_14e-3;
    let n = EARTH_RADIUS / (1.0 - e2 * lat.sin().powi(2)).sqrt();
    DVec3::new(
        (n + altitude) * lat.cos() * lon.cos(),
        (n + altitude) * lat.cos() * lon.sin(),
        (n * (1.0 - e2) + altitude) * lat.sin(),
    )
}

#[derive(Clone, Copy, Debug)]
pub struct Sphere {
    pub center: DVec3,
    pub radius: f64,
}

/// Conservative sphere: arc length bounds any chord, including date-line regions.
pub fn region_sphere(r: [f64; 6]) -> Sphere {
    let [west, south, mut east, north, low, high] = r;
    if east < west {
        east += std::f64::consts::TAU;
    }
    let center = ecef(
        ((north + south) * 0.5).to_degrees(),
        ((east + west) * 0.5).to_degrees(),
        (low + high) * 0.5,
    );
    let angular = ((north - south).abs() + (east - west).abs()) * 0.5;
    Sphere {
        center,
        radius: ((EARTH_RADIUS + low.abs().max(high.abs())) * angular.min(2.0)
            + (high - low).abs() * 0.5)
            .max(1.0),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapMode {
    #[default]
    ThreeD,
    TwoD,
}

#[derive(Clone, Debug)]
pub struct Camera {
    pub mode: MapMode,
    saved_orientation: Option<(f64, f64)>,
    pub target: DVec3,
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f64,
    pub distance: f64,
    pub heading: f64,
    pub pitch: f64,
}

impl Camera {
    pub fn new(
        latitude: f64,
        longitude: f64,
        altitude: f64,
        distance: f64,
        heading: f64,
        pitch: f64,
    ) -> Self {
        Self {
            mode: MapMode::ThreeD,
            saved_orientation: None,
            target: ecef(latitude, longitude, altitude),
            latitude,
            longitude,
            altitude,
            distance: distance.clamp(20., 5_000_000.),
            heading: heading.to_radians(),
            pitch: pitch.to_radians().clamp(0.1, 1.55),
        }
    }
    pub fn set_mode(&mut self, mode: MapMode) {
        if self.mode == mode {
            return;
        }
        match mode {
            MapMode::TwoD => {
                self.saved_orientation = Some((self.heading, self.pitch));
                self.heading = 0.;
                self.pitch = std::f64::consts::FRAC_PI_2;
            }
            MapMode::ThreeD => {
                if let Some((heading, pitch)) = self.saved_orientation.take() {
                    self.heading = heading;
                    self.pitch = pitch;
                }
            }
        }
        self.mode = mode;
    }
    pub fn orbit(&mut self, x: f64, y: f64) {
        if self.mode == MapMode::TwoD {
            self.pan(x, y);
            return;
        }
        self.heading -= x * 0.005;
        self.pitch = (self.pitch + y * 0.005).clamp(0.1, 1.55);
    }
    pub fn zoom(&mut self, delta: f64) {
        self.distance = (self.distance * (-delta * 0.002).exp()).clamp(20., 5_000_000.);
    }
    /// Pointer deltas are logical pixels; match the orthographic map footprint.
    pub fn pan_in_view(&mut self, x: f64, y: f64, logical_height: f64) {
        let scale =
            if self.mode == MapMode::TwoD && logical_height.is_finite() && logical_height > 0. {
                2. * (FOV * 0.5).tan() / (logical_height * 0.0015)
            } else {
                1.
            };
        self.pan(x * scale, y * scale);
    }
    pub fn pan(&mut self, x: f64, y: f64) {
        let meters = self.distance * 0.0015;
        let east = (-x * self.heading.cos() + y * self.heading.sin()) * meters;
        let north = (x * self.heading.sin() + y * self.heading.cos()) * meters;
        self.latitude = (self.latitude + (north / EARTH_RADIUS).to_degrees()).clamp(-85., 85.);
        self.longitude = (self.longitude
            + (east / (EARTH_RADIUS * self.latitude.to_radians().cos().max(0.05))).to_degrees()
            + 180.)
            .rem_euclid(360.)
            - 180.;
        self.target = ecef(self.latitude, self.longitude, self.altitude);
    }
    /// Pixel from the top-left of a viewport `width` by `height` logical pixels.
    pub fn project_ground(
        &self,
        width: u32,
        height: u32,
        latitude: f64,
        longitude: f64,
    ) -> Option<(f64, f64)> {
        let (x, y) = self.project_ground_unclipped(width, height, latitude, longitude)?;
        if x < -0.025 * width as f64
            || x > 1.025 * width as f64
            || y < -0.025 * height as f64
            || y > 1.025 * height as f64
        {
            return None;
        }
        Some((x, y))
    }
    /// Project polygon vertices outside the viewport; callers clip the polygon.
    pub fn project_ground_unclipped(
        &self,
        width: u32,
        height: u32,
        latitude: f64,
        longitude: f64,
    ) -> Option<(f64, f64)> {
        if width == 0 || height == 0 {
            return None;
        }
        let view = self.view(width, height);
        let world = ecef(latitude, longitude, self.altitude);
        let clip = view.view_projection * (world - view.origin).extend(1.);
        if clip.w <= 1e-6 || !clip.x.is_finite() || !clip.y.is_finite() {
            return None;
        }
        let ndc_x = clip.x / clip.w;
        let ndc_y = clip.y / clip.w;
        Some((
            (ndc_x + 1.) * 0.5 * f64::from(width),
            (1. - ndc_y) * 0.5 * f64::from(height),
        ))
    }
    /// Screen bounds of a transformed model box, clipped to the viewport.
    pub fn project_box(
        &self,
        width: u32,
        height: u32,
        transform: DMat4,
        min: DVec3,
        max: DVec3,
    ) -> Option<((f64, f64), (f64, f64))> {
        if width == 0 || height == 0 || !min.is_finite() || !max.is_finite() {
            return None;
        }
        let view = self.view(width, height);
        let mut lo = (f64::INFINITY, f64::INFINITY);
        let mut hi = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for x in [min.x, max.x] {
            for y in [min.y, max.y] {
                for z in [min.z, max.z] {
                    let world = transform.transform_point3(DVec3::new(x, y, z));
                    let clip = view.view_projection * (world - view.origin).extend(1.);
                    // Do not create a giant hit area across the near plane.
                    if !clip.is_finite() || clip.w <= 1e-6 {
                        return None;
                    }
                    let px = (clip.x / clip.w + 1.) * 0.5 * f64::from(width);
                    let py = (1. - clip.y / clip.w) * 0.5 * f64::from(height);
                    lo = (lo.0.min(px), lo.1.min(py));
                    hi = (hi.0.max(px), hi.1.max(py));
                }
            }
        }
        lo = (lo.0.max(0.), lo.1.max(0.));
        hi = (hi.0.min(f64::from(width)), hi.1.min(f64::from(height)));
        (hi.0 > lo.0 && hi.1 > lo.1).then_some((lo, hi))
    }
    /// Latitude and longitude of a viewport pixel on the camera's ground plane.
    pub fn unproject_ground(&self, width: u32, height: u32, x: f64, y: f64) -> Option<(f64, f64)> {
        self.unproject_ground_at_altitude(width, height, x, y, self.altitude)
    }
    pub fn unproject_ground_at_altitude(
        &self,
        width: u32,
        height: u32,
        x: f64,
        y: f64,
        altitude: f64,
    ) -> Option<(f64, f64)> {
        if width == 0 || height == 0 || !x.is_finite() || !y.is_finite() {
            return None;
        }
        let view = self.view(width, height);
        let inv = view.view_projection.inverse();
        let ndc_x = (x / f64::from(width)) * 2. - 1.;
        let ndc_y = 1. - (y / f64::from(height)) * 2.;
        let lift = |z: f64| {
            let hit = inv * DVec4::new(ndc_x, ndc_y, z, 1.);
            if !hit.w.is_finite() || hit.w.abs() < 1e-8 {
                return None;
            }
            Some(hit.truncate() / hit.w + view.origin)
        };
        let (near, far) = match (lift(-1.), lift(1.)) {
            (Some(near), Some(far)) if near.distance_squared(far) > 1. => (near, far),
            _ => lift(0.).zip(lift(1.))?,
        };
        let dir = far - near;
        let (lat0, lon0) = (self.latitude.to_radians(), self.longitude.to_radians());
        let up = DVec3::new(lat0.cos() * lon0.cos(), lat0.cos() * lon0.sin(), lat0.sin());
        let denom = dir.dot(up);
        if denom.abs() < 1e-8 {
            return None;
        }
        let plane = ecef(self.latitude, self.longitude, altitude);
        let t = (plane - near).dot(up) / denom;
        let hit = near + dir * t;
        let east = DVec3::new(-lon0.sin(), lon0.cos(), 0.).normalize();
        let north = up.cross(east).normalize();
        let delta = hit - plane;
        let north_m = delta.dot(north);
        let east_m = delta.dot(east);
        let latitude = self.latitude + (north_m / EARTH_RADIUS).to_degrees();
        let longitude = self.longitude
            + (east_m / (EARTH_RADIUS * self.latitude.to_radians().cos().max(0.05))).to_degrees();
        (latitude.is_finite() && longitude.is_finite()).then_some((latitude, longitude))
    }
    pub fn view(&self, width: u32, height: u32) -> View {
        let (lat, lon) = (self.latitude.to_radians(), self.longitude.to_radians());
        let up = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
        let east = DVec3::new(-lon.sin(), lon.cos(), 0.);
        let north = up.cross(east);
        let toward = north * self.heading.cos() + east * self.heading.sin();
        let relative_eye = (up * self.pitch.sin() - toward * self.pitch.cos()) * self.distance;
        let view = DMat4::look_at_rh(
            relative_eye,
            DVec3::ZERO,
            if self.mode == MapMode::TwoD {
                north
            } else {
                up
            },
        );
        let near = (self.distance * 0.0001).clamp(0.5, 500.);
        let aspect = width.max(1) as f64 / height.max(1) as f64;
        let half_height = self.distance * (FOV * 0.5).tan();
        let projection = if self.mode == MapMode::TwoD {
            DMat4::orthographic_rh(
                -half_height * aspect,
                half_height * aspect,
                -half_height,
                half_height,
                near,
                30_000_000.,
            )
        } else {
            DMat4::perspective_rh(FOV, aspect, near, 30_000_000.)
        };
        View {
            origin: self.target,
            eye: self.target + relative_eye,
            view_projection: projection * view,
            height: height.max(1),
            meters_per_pixel: (self.mode == MapMode::TwoD)
                .then_some(2. * half_height / height.max(1) as f64),
        }
    }
}

pub struct View {
    pub origin: DVec3,
    pub eye: DVec3,
    pub view_projection: DMat4,
    pub height: u32,
    pub meters_per_pixel: Option<f64>,
}
impl View {
    pub fn visible(&self, sphere: Sphere) -> bool {
        let m = self.view_projection.transpose();
        let rows = m.to_cols_array_2d().map(DVec4::from_array);
        let p = (sphere.center - self.origin).extend(1.);
        [
            rows[3] + rows[0],
            rows[3] - rows[0],
            rows[3] + rows[1],
            rows[3] - rows[1],
            rows[2],
            rows[3] - rows[2],
        ]
        .iter()
        .all(|plane| plane.dot(p) >= -sphere.radius * plane.truncate().length())
    }
    pub fn error(&self, sphere: Sphere, error: f64) -> f64 {
        if let Some(scale) = self.meters_per_pixel {
            return error / scale;
        }
        error * self.height as f64
            / (2. * (FOV * 0.5).tan() * (self.eye.distance(sphere.center) - sphere.radius).max(1.))
    }
}
