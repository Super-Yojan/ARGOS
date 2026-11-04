//! FrameAligner: the local map stays put while GPS wanders.
use argos_core::{Anchor, FixOutcome, FrameAligner, LocalFrame};

const LAT: f64 = 38.8297;
const LON: f64 = -77.3075;

/// Geographic point `north`, `east` metres from the test origin.
fn geo(north: f64, east: f64) -> (f64, f64) {
    Anchor {
        latitude: LAT,
        longitude: LON,
    }
    .geographic(north, -east)
    .unwrap()
}

/// North and east metres of a frame's local point from the test origin.
fn ne(frame: &LocalFrame, x: f64, y: f64) -> (f64, f64) {
    let (lat, lon) = frame.geographic(x, y).unwrap();
    let (north, west) = Anchor {
        latitude: LAT,
        longitude: LON,
    }
    .local(lat, lon)
    .unwrap();
    (north, -west)
}

/// Deterministic noise in [-1, 1].
struct Noise(u64);
impl Noise {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64 / (1u64 << 53) as f64) * 2. - 1.
    }
}

#[test]
fn a_parked_rover_keeps_its_map_while_gps_wanders() {
    let mut a = FrameAligner::default();
    let mut noise = Noise(7);
    let mut previous: Option<(f64, f64)> = None;
    let mut worst_step = 0f64;
    // Parked at the origin; local +x points north. GPS wanders up to ±3 m.
    for i in 0..120 {
        let t = i as f64;
        a.observe_pose("frame-1", t, 0., 0.);
        a.observe_heading(t, 0., 0., Some(5.));
        let (lat, lon) = geo(3. * noise.next(), 3. * noise.next());
        a.observe_fix(t, t, 0., lat, lon, 4.);
        let frame = a.frame().unwrap();
        // A grid cell 10 m ahead of the rover.
        let cell = ne(&frame, 10., 0.);
        if let Some(last) = previous
            && i > 10
        {
            worst_step = worst_step.max((cell.0 - last.0).hypot(cell.1 - last.1));
        }
        previous = Some(cell);
    }
    // Pinning to each fix moved this cell by up to ~8 m per fix.
    assert!(worst_step < 0.6, "cell moved {worst_step} m in one fix");
    let (north, east) = ne(&a.frame().unwrap(), 0., 0.);
    assert!(north.hypot(east) < 1.5, "rover at {north}, {east}");
}

#[test]
fn the_driven_track_cancels_compass_bias() {
    let mut a = FrameAligner::default();
    let mut noise = Noise(11);
    // Local +x is truly north, but the compass reads 20 degrees east of it.
    // Drive 30 m.
    for i in 0..=200 {
        let t = i as f64 * 0.5;
        let x = 0.3 * t;
        a.observe_pose("frame-1", t, x, 0.);
        a.observe_heading(t, 0., 20., Some(10.));
        if i % 2 == 0 {
            let (lat, lon) = geo(x + 2. * noise.next(), 2. * noise.next());
            a.observe_fix(t, t, 0., lat, lon, 3.);
        }
    }
    let frame = a.frame().unwrap();
    let heading = frame.heading_degrees(0.);
    let error = (heading + 180.).rem_euclid(360.) - 180.;
    assert!(error.abs() < 5., "heading {heading}");
    assert!(a.status().track_rotation);
}

#[test]
fn a_new_local_frame_starts_over() {
    let mut a = FrameAligner::default();
    a.observe_pose("frame-1", 0., 0., 0.);
    let (lat, lon) = geo(0., 0.);
    assert_eq!(a.observe_fix(0., 0., 0., lat, lon, 3.), FixOutcome::Used);
    assert!(a.frame().is_some());
    a.observe_pose("frame-2", 1., 0., 0.);
    assert!(a.frame().is_none());
    // A pose that teleports is a new frame too.
    a.observe_fix(1., 1., 0., lat, lon, 3.);
    a.observe_pose("frame-2", 1.1, 25., 0.);
    assert!(a.frame().is_none());
}

#[test]
fn one_bad_fix_is_ignored_and_a_lasting_shift_reanchors() {
    let mut a = FrameAligner::default();
    for i in 0..30 {
        let t = i as f64;
        a.observe_pose("frame-1", t, 0., 0.);
        a.observe_heading(t, 0., 0., Some(5.));
        let (lat, lon) = geo(0., 0.);
        a.observe_fix(t, t, 0., lat, lon, 3.);
    }
    let (far_lat, far_lon) = geo(40., 0.);
    a.observe_pose("frame-1", 30., 0., 0.);
    assert_eq!(
        a.observe_fix(30., 30., 0., far_lat, far_lon, 3.),
        FixOutcome::Rejected
    );
    let (north, _) = ne(&a.frame().unwrap(), 0., 0.);
    assert!(north.abs() < 0.5);
    a.observe_pose("frame-1", 31., 0., 0.);
    assert_eq!(
        a.observe_fix(31., 31., 0., far_lat, far_lon, 3.),
        FixOutcome::Rejected
    );
    a.observe_pose("frame-1", 32., 0., 0.);
    assert_eq!(
        a.observe_fix(32., 32., 0., far_lat, far_lon, 3.),
        FixOutcome::Reanchored
    );
    let (north, _) = ne(&a.frame().unwrap(), 0., 0.);
    assert!((north - 40.).abs() < 0.5, "rover at {north}");
}

#[test]
fn a_pinned_landmark_beats_biased_gps() {
    let mut a = FrameAligner::default();
    // GPS reads 4 m east of the truth the whole time.
    for i in 0..60 {
        let t = i as f64;
        a.observe_pose("frame-1", t, 0., 0.);
        a.observe_heading(t, 0., 0., Some(3.));
        let (lat, lon) = geo(0., 4.);
        a.observe_fix(t, t, 0., lat, lon, 4.);
    }
    // A bucket seen 6 m ahead whose spot was pinned on the satellite map.
    let (lat, lon) = geo(6., 0.);
    assert_eq!(
        a.observe_landmark(60., 6., 0., lat, lon, 0.5),
        FixOutcome::Used
    );
    // Let the published frame slide to the new fit.
    for i in 61..80 {
        let t = i as f64;
        a.observe_pose("frame-1", t, 0., 0.);
        a.observe_heading(t, 0., 0., Some(3.));
    }
    let (north, east) = ne(&a.frame().unwrap(), 0., 0.);
    assert!(north.hypot(east) < 1., "rover at {north}, {east}");
}

#[test]
fn corrections_slide_instead_of_snapping() {
    let mut a = FrameAligner::default();
    a.observe_pose("frame-1", 0., 0., 0.);
    a.observe_heading(0., 0., 0., Some(5.));
    let (lat, lon) = geo(0., 0.);
    a.observe_fix(0., 0., 0., lat, lon, 1.);
    // A fix 2.5 m away (inside the 3 m gate) arrives one second later.
    a.observe_pose("frame-1", 1., 0., 0.);
    let (lat, lon) = geo(2.5, 0.);
    assert_eq!(a.observe_fix(1., 1., 0., lat, lon, 1.), FixOutcome::Used);
    let (north, _) = ne(&a.frame().unwrap(), 0., 0.);
    assert!(north > 0. && north <= 0.5 + 1e-9, "moved {north} m in 1 s");
}

#[test]
fn fixes_pair_with_the_pose_at_fix_time() {
    let mut a = FrameAligner::default();
    a.observe_heading(0., 0., 0., Some(3.));
    // Driving north at 1 m/s; each fix is reported 1 s late.
    for i in 0..=40 {
        let t = i as f64 * 0.1;
        a.observe_pose("frame-1", t, t, 0.);
    }
    let (lat, lon) = geo(3., 0.);
    assert_eq!(a.observe_fix(4., 3., 1., lat, lon, 2.), FixOutcome::Used);
    let (north, _) = ne(&a.frame().unwrap(), 4., 0.);
    assert!((north - 4.).abs() < 0.05, "rover at {north}");
}
