//! `FleetFeed`: the window's handle to the fleet. It starts the background
//! thread, hands out the latest `FleetPicture`, and holds teleop limits.
//! The commands it sends are in `commands.rs`.

use super::config::DashboardConfig;
use super::marks::FleetPicture;
use super::marks::position_label;
use super::poll::run;
use argos_core::point_capture::PointLabel;
use argos_core::{LocalFrame, TeleopLimits};
use argos_zenoh::Client;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Fleet subscriber. Drop does not stop the thread; [`FleetFeed::shutdown`] does.
#[derive(Clone)]
pub struct FleetFeed {
    pub(super) picture: Arc<Mutex<FleetPicture>>,
    pub(super) client: Arc<Mutex<Option<Arc<Client>>>>,
    pub(super) limits: Arc<Mutex<TeleopLimits>>,
    pub(super) origin: LocalFrame,
    /// The rover whose heavy topics the poll loop keeps subscribed.
    pub(super) focus: Arc<Mutex<Option<u64>>>,
    /// Set by `shutdown`; the thread closes its session and exits.
    pub(super) stopped: Arc<AtomicBool>,
    /// Who the next command is for, carried in its token. See [`FleetFeed::as_source`].
    pub(super) source: Arc<Mutex<argos_core::CommandSource>>,
}

/// Inspector line for a live stream: frame age and rate, or why it ended.
pub fn live_line(status: &argos_zenoh::LiveStatus) -> String {
    if let Some(reason) = &status.ended {
        return match reason.as_str() {
            "replaced" => "Live 3D ended: another capture replaced it".into(),
            "lease_expired" => "Live 3D ended: the rover stopped hearing ARGOS".into(),
            other => format!("Live 3D ended: {}", other.replace('_', " ")),
        };
    }
    match &status.cloud {
        None if status.elapsed > 4. => {
            "Live 3D · no frames yet; the rover needs scene depth and a running map".into()
        }
        None => "Live 3D · waiting for the first frame…".into(),
        Some((cloud, at)) => {
            let age = at.elapsed().as_secs_f64();
            let total: usize = cloud.label_counts.iter().sum();
            let returns = total - cloud.label_counts[PointLabel::NoReturn as usize];
            let shown = |label: PointLabel| cloud.label_counts[label as usize];
            format!(
                "Live 3D · {} frames · last {age:.1} s ago · {returns} returns: free {} · obstacle {} · overhead {} · out of range {} · outside band {} · floor veto {} · stopped at wall/glass {}",
                status.frames,
                shown(PointLabel::Free),
                shown(PointLabel::Obstacle),
                shown(PointLabel::Overhead),
                shown(PointLabel::OutOfRange),
                shown(PointLabel::OutsideBand),
                shown(PointLabel::FloorVeto),
                shown(PointLabel::Barrier),
            )
        }
    }
}

/// Inspector lines for a capture: progress or the saved file, then the label counts.
pub fn capture_lines(status: &argos_zenoh::CaptureStatus) -> (String, Option<String>) {
    let missing = match status.detail.as_str() {
        "" | "cancelled" => String::new(),
        "no_depth" => " · no depth frames for some slots".into(),
        "link_busy" => " · link too busy for some frames".into(),
        other => format!(" · {other}"),
    };
    let progress = match (&status.saved, status.finished) {
        (Some(Ok(path)), _) => format!(
            "Saved {} of {} frames{missing} · {}",
            status.received,
            status.expected,
            path.display()
        ),
        (Some(Err(error)), _) => format!("Capture not saved: {error}"),
        (None, true) => "Writing PLY…".into(),
        (None, false) => format!(
            "Capturing · {} of {} frames{missing}",
            status.received, status.expected
        ),
    };
    let total: usize = status.label_counts.iter().sum();
    let counts = (total > 0).then(|| {
        argos_core::point_capture::PointLabel::ALL
            .iter()
            .filter(|label| **label != argos_core::point_capture::PointLabel::NoReturn)
            .map(|label| {
                format!(
                    "{} {}",
                    label.name().replace('_', " "),
                    status.label_counts[*label as usize]
                )
            })
            .collect::<Vec<_>>()
            .join(" · ")
    });
    (progress, counts)
}

pub(super) fn unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

impl FleetFeed {
    pub fn start(config: DashboardConfig) -> Self {
        let limits = Arc::new(Mutex::new(config.teleop));
        if let Some(notice) = &config.connect_notice {
            return Self {
                picture: Arc::new(Mutex::new(FleetPicture {
                    link: "password".into(),
                    detail: notice.clone(),
                    command: String::new(),
                    marks: Vec::new(),
                    presence: Vec::new(),
                    observed_at: std::time::Instant::now(),
                })),
                client: Arc::new(Mutex::new(None)),
                limits,
                origin: config.origin,
                focus: Arc::new(Mutex::new(None)),
                stopped: Arc::new(AtomicBool::new(false)),
                source: Arc::new(Mutex::new(argos_core::CommandSource::Operator)),
            };
        }
        let origin = config.origin;
        let picture = Arc::new(Mutex::new(FleetPicture {
            link: "connecting".into(),
            detail: format!("Connecting to {} ({})", config.endpoint, config.prefix),
            command: String::new(),
            marks: Vec::new(),
            presence: Vec::new(),
            observed_at: std::time::Instant::now(),
        }));
        let client = Arc::new(Mutex::new(None));
        let focus = Arc::new(Mutex::new(None));
        let shared = picture.clone();
        let slot = client.clone();
        let wanted = focus.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        std::thread::Builder::new()
            .name("argos-fleet".into())
            .spawn(move || run(config, shared, slot, wanted, stop))
            .expect("argos-fleet thread");
        Self {
            picture,
            client,
            limits,
            origin,
            focus,
            stopped,
            source: Arc::new(Mutex::new(argos_core::CommandSource::Operator)),
        }
    }
    /// Run `send` with its commands' tokens marked as coming from `source`,
    /// so the session log and receipts tell an agent's or an accepted
    /// recommendation's commands from the operator's own.
    pub fn as_source<R>(
        &self,
        source: argos_core::CommandSource,
        send: impl FnOnce(&Self) -> R,
    ) -> R {
        let previous = self.swap_source(source);
        let result = send(self);
        self.swap_source(previous);
        result
    }
    /// Mark commands as coming from `source` until the next swap; returns the
    /// source to put back. For callers that cannot use [`Self::as_source`].
    pub fn swap_source(&self, source: argos_core::CommandSource) -> argos_core::CommandSource {
        if let Ok(client) = self.connected() {
            client.set_command_source(
                (source != argos_core::CommandSource::Operator).then(|| source.name()),
            );
        }
        std::mem::replace(&mut *self.source.lock().unwrap(), source)
    }
    pub(super) fn token_source(&self) -> argos_core::CommandSource {
        *self.source.lock().unwrap()
    }
    /// Add an event to the session log, stamped like the client's own records.
    pub fn record(&self, event: Value) {
        if let Ok(client) = self.connected() {
            client.record_event(event);
        }
    }
    /// The operator telemetry the client caches for one rover, as JSON.
    pub fn rover_telemetry(&self, id: u64) -> Value {
        self.connected()
            .ok()
            .and_then(|client| {
                serde_json::from_str::<Value>(&client.operator_state_snapshot()).ok()
            })
            .map(|mut all| {
                let mut rover = all[id.to_string()].take();
                // Grids and point clouds are far too big for an agent's context.
                if let Some(rover) = rover.as_object_mut() {
                    rover.remove("cloud");
                    rover.remove("occupancy");
                }
                rover
            })
            .unwrap_or(Value::Null)
    }
    /// Zero any held drive input, then close the session in the background.
    /// Commands fail with "Not connected" from here on. Used when the window
    /// switches between simulation and the real fleet.
    pub fn shutdown(&self) {
        self.end_drive();
        self.stopped.store(true, Ordering::Release);
        self.client.lock().unwrap().take();
    }
    /// Keep this rover's occupancy grid, point cloud, and depth subscribed,
    /// or none. Applied on the next poll and again after a reconnect.
    pub fn focus(&self, id: Option<u64>) {
        *self.focus.lock().unwrap() = id;
    }
    pub fn origin(&self) -> LocalFrame {
        self.origin
    }
    pub fn picture(&self) -> FleetPicture {
        let mut picture = self.picture.lock().unwrap().clone();
        let elapsed = picture.observed_at.elapsed().as_secs_f64();
        for mark in &mut picture.marks {
            if let Some(gps) = &mut mark.raw_gps {
                gps.age_s = gps.age_s.map(|age| age + elapsed);
            }
        }
        for row in &mut picture.presence {
            row.observation_age = row.observation_age.map(|age| age + elapsed);
            if let Some(mark) = picture.marks.iter().find(|mark| mark.id == row.id) {
                row.position_reason = position_label(mark);
            }
            if !row.source_reason.is_empty() {
                row.position_reason.push_str(" · ");
                row.position_reason.push_str(&row.source_reason);
            }
        }
        picture
    }
    pub fn limits(&self) -> TeleopLimits {
        *self.limits.lock().unwrap()
    }
    pub fn set_linear(&self, linear: f64) -> Result<(), String> {
        self.update_limits(|limits| limits.linear = linear)
    }
    pub fn set_spin(&self, allow_spin: bool) -> Result<(), String> {
        self.update_limits(|limits| limits.allow_spin = allow_spin)
    }
    pub fn set_yaw_rate(&self, yaw_rate: f64) -> Result<(), String> {
        self.update_limits(|limits| limits.yaw_rate = yaw_rate)
    }
    pub fn set_yaw_accel(&self, yaw_accel: f64) -> Result<(), String> {
        self.update_limits(|limits| limits.yaw_accel = yaw_accel)
    }
    pub fn set_yaw_decel(&self, yaw_decel: f64) -> Result<(), String> {
        self.update_limits(|limits| limits.yaw_decel = yaw_decel)
    }
    pub fn set_min_radius(&self, min_turn_radius: f64) -> Result<(), String> {
        self.update_limits(|limits| limits.min_turn_radius = min_turn_radius)
    }
    fn update_limits(&self, edit: impl FnOnce(&mut TeleopLimits)) -> Result<(), String> {
        let mut limits = self.limits.lock().unwrap();
        edit(&mut limits);
        let checked = (*limits).validate().map_err(|error| error.to_string())?;
        *limits = checked;
        if let Some(client) = self.client.lock().unwrap().clone() {
            client.set_teleop_guard(*limits);
        }
        Ok(())
    }
    pub(super) fn connected(&self) -> Result<Arc<Client>, String> {
        self.client
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "Not connected".to_string())
    }
}
