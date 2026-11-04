//! A steady geographic frame for a rover's local (VIO) map.
//!
//! Terra's phone reports two things about where the rover is: a continuous
//! local pose from visual-inertial odometry, and raw GPS fixes that wander by
//! a few metres from one fix to the next. Pinning the local map to each new
//! fix makes the whole map, its route, goals and found objects jump with GPS
//! noise. This module keeps the local map rigid instead and only estimates the
//! one transform between local metres and the globe (a rotation and an
//! offset, the same idea as ROS `map -> odom`):
//!
//! * every GPS fix is paired with the local pose at the fix time, and a
//!   weighted rigid fit over a recent window of pairs gives the offset;
//! * the rotation comes from Terra's fused heading while the rover has barely
//!   moved, and more and more from the driven GPS track once it has, which
//!   also cancels a constant compass bias;
//! * a fix far outside the fit is rejected, and three rejected in a row
//!   re-anchor on the newest fix (the local frame really did move);
//! * the published frame moves toward each new fit at a bounded speed, so a
//!   correction slides the map instead of snapping it.
//!
//! Landmarks with known geographic positions (for example a bucket pinned on
//! the satellite map and matched to a detection) go into the same fit with
//! their own, much smaller, uncertainty.
use crate::{Anchor, LocalFrame};
use std::collections::VecDeque;
use std::f64::consts::PI;

/// Tuning for [`FrameAligner`]. The defaults suit a phone on a slow rover.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlignmentLimits {
    /// GPS fixes worse than this are not used, metres.
    pub max_fix_accuracy_m: f64,
    /// GPS fixes older than this when they arrive are not used, seconds.
    pub max_fix_age_s: f64,
    /// Smallest GPS sigma trusted, metres.
    pub min_fix_sigma_m: f64,
    /// GPS fixes older than this leave the fit, seconds.
    pub fix_window_s: f64,
    /// Landmark observations older than this leave the fit, seconds.
    pub landmark_window_s: f64,
    /// Heading samples older than this leave the rotation prior, seconds.
    pub heading_window_s: f64,
    /// Heading sigma used when Terra reports none, degrees.
    pub default_heading_sigma_deg: f64,
    /// Floor on the rotation prior's sigma, degrees. A phone compass near
    /// motors is often off by more than it reports; with this floor about
    /// 15 m of driven GPS track outweighs the compass.
    pub min_heading_sigma_deg: f64,
    /// GPS fix weight is divided by this. Phone fixes arrive about once a
    /// second but share most of their error over a few seconds.
    pub fix_correlation_s: f64,
    /// Driven span needed before the GPS track may steer the rotation, metres.
    pub min_track_span_m: f64,
    /// Floor on a GPS-only fit's offset sigma: averaging many fixes does not
    /// remove the few metres of bias they share, metres.
    pub gps_bias_m: f64,
    /// Innovations below this are never rejected, metres.
    pub min_reject_m: f64,
    /// Consecutive rejected fixes that re-anchor on the newest fix.
    pub max_rejects: u32,
    /// Fastest the published frame may slide at the rover, metres per second.
    pub slew_mps: f64,
    /// Fastest the published frame may turn, degrees per second.
    pub slew_dps: f64,
}

impl Default for AlignmentLimits {
    fn default() -> Self {
        Self {
            max_fix_accuracy_m: 15.,
            max_fix_age_s: 3.,
            min_fix_sigma_m: 1.,
            fix_window_s: 120.,
            landmark_window_s: 900.,
            heading_window_s: 30.,
            default_heading_sigma_deg: 30.,
            min_heading_sigma_deg: 15.,
            fix_correlation_s: 4.,
            min_track_span_m: 5.,
            gps_bias_m: 2.5,
            min_reject_m: 3.,
            max_rejects: 3,
            slew_mps: 0.5,
            slew_dps: 3.,
        }
    }
}

/// What happened to one GPS fix or landmark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixOutcome {
    /// Used in the fit.
    Used,
    /// Same fix as last time; ignored.
    Repeat,
    /// Too old, too coarse, or no local pose at its time.
    Unusable,
    /// Too far from the current fit; counted toward a re-anchor.
    Rejected,
    /// Started a fresh fit on this fix.
    Reanchored,
}

/// Where the published frame came from, for the console.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AlignmentStatus {
    /// One-sigma offset error, metres.
    pub sigma_m: f64,
    /// One-sigma rotation error, degrees.
    pub rotation_sigma_deg: f64,
    pub fixes: usize,
    pub landmarks: usize,
    /// The driven GPS track, not only the compass, sets the rotation.
    pub track_rotation: bool,
    pub rejected: u64,
    pub reanchors: u64,
}

#[derive(Clone, Copy, Debug)]
struct Pair {
    t: f64,
    /// Local metres.
    x: f64,
    y: f64,
    /// North and west of the plane origin, metres.
    north: f64,
    west: f64,
    sigma: f64,
    landmark: bool,
}

/// Local point `p` maps to plane point `R(rotation) p + (tn, tw)`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fit {
    rotation: f64,
    tn: f64,
    tw: f64,
    sigma_m: f64,
    rotation_sigma: f64,
    track_rotation: bool,
}

impl Fit {
    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let (s, c) = self.rotation.sin_cos();
        (c * x - s * y + self.tn, s * x + c * y + self.tw)
    }
}

/// One rover's local-to-geographic alignment. Feed it every pose, heading
/// and GPS fix; read [`FrameAligner::frame`] to draw the local map.
#[derive(Clone, Debug, Default)]
pub struct FrameAligner {
    limits: AlignmentLimits,
    /// Local frame identity (Terra frame ID and run). A change resets.
    key: Option<String>,
    origin: Option<(f64, f64)>,
    poses: VecDeque<(f64, f64, f64)>,
    headings: VecDeque<(f64, f64, f64)>,
    pairs: VecDeque<Pair>,
    last_fix_at: Option<f64>,
    rejects: u32,
    published: Option<Fit>,
    published_at: f64,
    rejected: u64,
    reanchors: u64,
}

impl FrameAligner {
    pub fn new(limits: AlignmentLimits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    /// Forget everything about the current local frame.
    pub fn reset(&mut self) {
        if self.published.take().is_some() {
            self.reanchors += 1;
        }
        self.origin = None;
        self.poses.clear();
        self.headings.clear();
        self.pairs.clear();
        self.last_fix_at = None;
        self.rejects = 0;
    }

    /// A local pose at `t` seconds. `key` names the local frame (Terra's
    /// frame ID and run ID); a new key, or a pose that teleports, is a new
    /// local frame and starts the alignment again.
    pub fn observe_pose(&mut self, key: &str, t: f64, x: f64, y: f64) {
        if !(t.is_finite() && x.is_finite() && y.is_finite()) {
            return;
        }
        if self.key.as_deref() != Some(key) {
            self.reset();
            self.key = Some(key.to_string());
        }
        if let Some(&(last_t, last_x, last_y)) = self.poses.back() {
            if t < last_t {
                return;
            }
            let step = (x - last_x).hypot(y - last_y);
            // A phone rover cannot cover this; the VIO frame relocalized.
            if step > 1. + 1.5 * (t - last_t) {
                self.reset();
            } else if t == last_t {
                return;
            }
        }
        self.poses.push_back((t, x, y));
        while self.poses.front().is_some_and(|&(old, _, _)| t - old > 10.) {
            self.poses.pop_front();
        }
    }

    /// Terra's fused true heading (degrees clockwise from north) for local
    /// `yaw` (radians, counter-clockwise), observed at `t`.
    pub fn observe_heading(
        &mut self,
        t: f64,
        yaw: f64,
        heading_deg: f64,
        accuracy_deg: Option<f64>,
    ) {
        if !(t.is_finite() && yaw.is_finite() && heading_deg.is_finite()) {
            return;
        }
        if self
            .headings
            .back()
            .is_some_and(|&(last, _, _)| t - last < 0.5)
        {
            return;
        }
        let sigma = accuracy_deg
            .filter(|a| a.is_finite() && *a > 0.)
            .unwrap_or(self.limits.default_heading_sigma_deg)
            .to_radians();
        self.headings
            .push_back((t, wrap(-heading_deg.to_radians() - yaw), sigma));
        let window = self.limits.heading_window_s;
        while self
            .headings
            .front()
            .is_some_and(|&(old, _, _)| t - old > window)
        {
            self.headings.pop_front();
        }
        if self.published.is_some() {
            self.refit(t, false);
        }
    }

    /// A GPS fix with one-sigma `accuracy_m`, received at `now` and taken
    /// `age_s` seconds earlier. `observed_at` identifies the fix so a repeated
    /// report is not counted twice.
    pub fn observe_fix(
        &mut self,
        now: f64,
        observed_at: f64,
        age_s: f64,
        latitude: f64,
        longitude: f64,
        accuracy_m: f64,
    ) -> FixOutcome {
        if self.last_fix_at == Some(observed_at) {
            return FixOutcome::Repeat;
        }
        self.last_fix_at = Some(observed_at);
        let limits = self.limits;
        if !(age_s.is_finite() && accuracy_m.is_finite())
            || age_s > limits.max_fix_age_s
            || accuracy_m > limits.max_fix_accuracy_m
        {
            return FixOutcome::Unusable;
        }
        let Some((x, y)) = self.pose_at(now - age_s.max(0.)) else {
            return FixOutcome::Unusable;
        };
        let Some((north, west)) = self.plane(latitude, longitude) else {
            return FixOutcome::Unusable;
        };
        let pair = Pair {
            t: now,
            x,
            y,
            north,
            west,
            sigma: accuracy_m.max(limits.min_fix_sigma_m),
            landmark: false,
        };
        if let Some(fit) = self.current_fit() {
            if self.outlier(&fit, &pair) {
                self.rejected += 1;
                self.rejects += 1;
                if self.rejects < limits.max_rejects {
                    return FixOutcome::Rejected;
                }
                // The local frame moved under us: start over on this fix,
                // keeping the heading prior, which does not depend on it.
                self.pairs.clear();
                self.rejects = 0;
                self.reanchors += 1;
                self.pairs.push_back(pair);
                self.refit(now, true);
                return FixOutcome::Reanchored;
            }
        }
        self.rejects = 0;
        self.pairs.push_back(pair);
        self.prune(now);
        self.refit(now, false);
        FixOutcome::Used
    }

    /// A landmark seen at local (`x`, `y`) whose true position is known,
    /// for example a pinned bucket matched to a detection. `sigma_m` covers
    /// both the pin and the detection.
    pub fn observe_landmark(
        &mut self,
        now: f64,
        x: f64,
        y: f64,
        latitude: f64,
        longitude: f64,
        sigma_m: f64,
    ) -> FixOutcome {
        if !(x.is_finite() && y.is_finite() && sigma_m.is_finite() && sigma_m > 0.) {
            return FixOutcome::Unusable;
        }
        let Some((north, west)) = self.plane(latitude, longitude) else {
            return FixOutcome::Unusable;
        };
        let pair = Pair {
            t: now,
            x,
            y,
            north,
            west,
            sigma: sigma_m,
            landmark: true,
        };
        if let Some(fit) = self.current_fit()
            && self.outlier(&fit, &pair)
        {
            // Most likely matched to the wrong pin.
            self.rejected += 1;
            return FixOutcome::Rejected;
        }
        self.pairs.push_back(pair);
        self.prune(now);
        self.refit(now, false);
        FixOutcome::Used
    }

    /// The frame to draw the local map through, once there is a fix.
    pub fn frame(&self) -> Option<LocalFrame> {
        let fit = self.published?;
        let (latitude, longitude) = self.origin?;
        // Plane = R p + t  ==  R (p - a)  with  a = -R^T t.
        let (s, c) = fit.rotation.sin_cos();
        let anchor_x = -(c * fit.tn + s * fit.tw);
        let anchor_y = -(-s * fit.tn + c * fit.tw);
        Some(LocalFrame {
            latitude,
            longitude,
            anchor_x,
            anchor_y,
            rotation: fit.rotation,
        })
    }

    pub fn status(&self) -> AlignmentStatus {
        let fit = self.published;
        AlignmentStatus {
            sigma_m: fit.map_or(f64::INFINITY, |f| f.sigma_m),
            rotation_sigma_deg: fit.map_or(f64::INFINITY, |f| f.rotation_sigma.to_degrees()),
            fixes: self.pairs.iter().filter(|p| !p.landmark).count(),
            landmarks: self.pairs.iter().filter(|p| p.landmark).count(),
            track_rotation: fit.is_some_and(|f| f.track_rotation),
            rejected: self.rejected,
            reanchors: self.reanchors,
        }
    }

    fn plane(&mut self, latitude: f64, longitude: f64) -> Option<(f64, f64)> {
        let (lat0, lon0) = *self.origin.get_or_insert((latitude, longitude));
        Anchor {
            latitude: lat0,
            longitude: lon0,
        }
        .local(latitude, longitude)
        .ok()
    }

    /// Local position at `t`, interpolated from the recent poses.
    fn pose_at(&self, t: f64) -> Option<(f64, f64)> {
        let &(last_t, last_x, last_y) = self.poses.back()?;
        if t >= last_t {
            return (t - last_t <= 1.).then_some((last_x, last_y));
        }
        let mut later = (last_t, last_x, last_y);
        for &(pt, px, py) in self.poses.iter().rev().skip(1) {
            if pt <= t {
                let span = later.0 - pt;
                let f = if span > 0. { (t - pt) / span } else { 0. };
                return Some((px + f * (later.1 - px), py + f * (later.2 - py)));
            }
            later = (pt, px, py);
        }
        None
    }

    fn outlier(&self, fit: &Fit, pair: &Pair) -> bool {
        let (n, w) = fit.apply(pair.x, pair.y);
        let innovation = (n - pair.north).hypot(w - pair.west);
        let sigma = pair.sigma.hypot(fit.sigma_m);
        // 2-D chi-square at 99% is 9.21, so 3.03 sigma.
        innovation > self.limits.min_reject_m && innovation > 3.03 * sigma
    }

    fn prune(&mut self, now: f64) {
        let limits = self.limits;
        self.pairs.retain(|p| {
            now - p.t
                <= if p.landmark {
                    limits.landmark_window_s
                } else {
                    limits.fix_window_s
                }
        });
    }

    /// The rotation prior from the heading window: a weighted circular mean,
    /// with a sigma that does not shrink with more samples, since compass
    /// error is mostly bias.
    fn heading_prior(&self) -> Option<(f64, f64)> {
        if self.headings.is_empty() {
            return None;
        }
        let (mut sx, mut sy, mut weights) = (0., 0., 0.);
        for &(_, rotation, sigma) in &self.headings {
            let w = 1. / (sigma * sigma);
            sx += w * rotation.cos();
            sy += w * rotation.sin();
            weights += w;
        }
        let sigma = (self.headings.len() as f64 / weights)
            .sqrt()
            .max(self.limits.min_heading_sigma_deg.to_radians());
        Some((sy.atan2(sx), sigma))
    }

    /// Rigid fit of local points to plane points with the heading prior.
    fn solve(&self) -> Option<Fit> {
        if self.pairs.is_empty() {
            return None;
        }
        let limits = self.limits;
        // Fixes close in time share most of their error (multipath, filter
        // lag), so they count for less than independent samples.
        let weight = |p: &Pair| {
            let w = 1. / (p.sigma * p.sigma);
            if p.landmark {
                w
            } else {
                w * (1. / limits.fix_correlation_s).min(1.)
            }
        };
        let total: f64 = self.pairs.iter().map(weight).sum();
        let mean = |f: &dyn Fn(&Pair) -> f64| {
            self.pairs.iter().map(|p| weight(p) * f(p)).sum::<f64>() / total
        };
        let (px, py) = (mean(&|p| p.x), mean(&|p| p.y));
        let (gn, gw) = (mean(&|p| p.north), mean(&|p| p.west));
        let (mut a, mut b, mut info, mut span) = (0., 0., 0., 0f64);
        for p in &self.pairs {
            let w = weight(p);
            let (x, y) = (p.x - px, p.y - py);
            let (n, v) = (p.north - gn, p.west - gw);
            a += w * (x * n + y * v);
            b += w * (x * v - y * n);
            info += w * (x * x + y * y);
            span = span.max(x.hypot(y));
        }
        let track = (span >= limits.min_track_span_m && info > 0.).then(|| (b.atan2(a), info));
        let prior = self.heading_prior();
        let (rotation, rotation_sigma, track_rotation) = match (prior, track) {
            (Some((h, hs)), Some((r, info))) => {
                let hi = 1. / (hs * hs);
                let rotation = wrap(h + wrap(r - h) * info / (info + hi));
                (rotation, (1. / (info + hi)).sqrt(), info > hi)
            }
            (Some((h, hs)), None) => (h, hs, false),
            (None, Some((r, info))) => (r, (1. / info).sqrt(), true),
            // No heading and no track yet: local +x north, as before.
            (None, None) => (0., PI, false),
        };
        let (s, c) = rotation.sin_cos();
        let mut sigma_m = (1. / total).sqrt();
        if !self.pairs.iter().any(|p| p.landmark) {
            sigma_m = sigma_m.max(limits.gps_bias_m);
        }
        Some(Fit {
            rotation,
            tn: gn - (c * px - s * py),
            tw: gw - (s * px + c * py),
            sigma_m,
            rotation_sigma,
            track_rotation,
        })
    }

    /// The fit a new observation is checked against; none before the first.
    fn current_fit(&self) -> Option<Fit> {
        self.published.and(self.solve())
    }

    fn refit(&mut self, now: f64, snap: bool) {
        let Some(mut target) = self.solve() else {
            return;
        };
        // Drop pairs the new fit cannot explain (early bad fixes), then refit.
        let before = self.pairs.len();
        let gate = |p: &Pair| {
            let (n, w) = target.apply(p.x, p.y);
            let innovation = (n - p.north).hypot(w - p.west);
            innovation <= self.limits.min_reject_m
                || innovation <= 3.03 * p.sigma.hypot(target.sigma_m)
        };
        let kept: VecDeque<Pair> = self.pairs.iter().copied().filter(gate).collect();
        if !kept.is_empty() && kept.len() < before {
            self.rejected += (before - kept.len()) as u64;
            self.pairs = kept;
            if let Some(fit) = self.solve() {
                target = fit;
            }
        }
        let dt = (now - self.published_at).clamp(0.1, 5.);
        self.published_at = now;
        let Some(current) = self.published.filter(|_| !snap) else {
            self.published = Some(target);
            return;
        };
        // Slide toward the new fit, pivoting at the rover so it moves the
        // least where the operator is looking.
        let (x, y) = self.poses.back().map_or((0., 0.), |&(_, x, y)| (x, y));
        let turn = wrap(target.rotation - current.rotation);
        let max_turn = self.limits.slew_dps.to_radians() * dt;
        let rotation = wrap(current.rotation + turn.clamp(-max_turn, max_turn));
        let (from_n, from_w) = current.apply(x, y);
        let (to_n, to_w) = target.apply(x, y);
        let (dn, dw) = (to_n - from_n, to_w - from_w);
        let distance = dn.hypot(dw);
        let max_step = self.limits.slew_mps * dt;
        let f = if distance > max_step {
            max_step / distance
        } else {
            1.
        };
        let (rover_n, rover_w) = (from_n + f * dn, from_w + f * dw);
        let (s, c) = rotation.sin_cos();
        self.published = Some(Fit {
            rotation,
            tn: rover_n - (c * x - s * y),
            tw: rover_w - (s * x + c * y),
            ..target
        });
    }
}

fn wrap(angle: f64) -> f64 {
    let a = (angle + PI).rem_euclid(2. * PI) - PI;
    if a <= -PI { a + 2. * PI } else { a }
}
