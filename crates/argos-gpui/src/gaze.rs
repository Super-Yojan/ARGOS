//! Operator gaze and blinks from an eye tracker, mapped to rovers on screen.
//!
//! An eye tracker process (`tools/eye-tracker/argos_eye_tracker.py`, webcam
//! based) sends one JSON object per UDP datagram to `127.0.0.1:ARGOS_GAZE_PORT`
//! (default [`GAZE_PORT`]):
//!
//! | `type` | Fields |
//! | --- | --- |
//! | `gaze` | `utc`, `x`, `y` (screen points, top-left origin), `confidence` 0–1 |
//! | `no_face` | `utc`: the camera does not see the operator's eyes |
//! | `blink` | `utc` (when the eyes reopened), `duration_ms` |
//! | `status` | `utc`, `tracker`, `calibrated`, `error_px`, plus anything else |
//!
//! The window turns each gaze point into window coordinates, finds the area
//! under it ([`hit`]) and logs dwell spans ([`DwellTracker`]), so the log says
//! which rover the operator looked at and for how long.

use serde_json::Value;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Default UDP port for the eye tracker.
pub const GAZE_PORT: u16 = 47_870;

/// A new area must hold the gaze this long before the dwell switches to it.
/// Shorter glances are saccades passing over it.
pub const DWELL_SWITCH_S: f64 = 0.1;

/// Keep at most this many unread messages (a stalled window drops the oldest).
const QUEUE: usize = 4_096;

/// One message from the eye tracker.
#[derive(Clone, Debug, PartialEq)]
pub enum GazeMessage {
    Gaze {
        utc: f64,
        x: f64,
        y: f64,
        confidence: f64,
    },
    NoFace {
        utc: f64,
    },
    Blink {
        utc: f64,
        duration_ms: f64,
    },
    Status(Value),
}

impl GazeMessage {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let value: Value = serde_json::from_slice(bytes).ok()?;
        let utc = value["utc"].as_f64().unwrap_or(0.);
        match value["type"].as_str()? {
            "gaze" => Some(Self::Gaze {
                utc,
                x: value["x"].as_f64().filter(|x| x.is_finite())?,
                y: value["y"].as_f64().filter(|y| y.is_finite())?,
                confidence: value["confidence"].as_f64().unwrap_or(1.),
            }),
            "no_face" => Some(Self::NoFace { utc }),
            "blink" => Some(Self::Blink {
                utc,
                duration_ms: value["duration_ms"].as_f64()?,
            }),
            "status" => Some(Self::Status(value)),
            _ => None,
        }
    }
}

/// Listens for eye tracker datagrams on a background thread.
#[derive(Clone)]
pub struct GazeReceiver {
    queue: Arc<Mutex<Vec<GazeMessage>>>,
    pub port: u16,
}

impl GazeReceiver {
    /// Bind `127.0.0.1:port`. Returns `None` if the port is taken; the
    /// dashboard then runs without gaze.
    pub fn start(port: u16) -> Option<Self> {
        let socket = UdpSocket::bind(("127.0.0.1", port)).ok()?;
        socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .ok()?;
        let queue = Arc::new(Mutex::new(Vec::new()));
        let sink = queue.clone();
        std::thread::Builder::new()
            .name("argos-gaze".into())
            .spawn(move || {
                let mut buffer = [0u8; 2048];
                loop {
                    let Ok(size) = socket.recv(&mut buffer) else {
                        continue;
                    };
                    if let Some(message) = GazeMessage::parse(&buffer[..size]) {
                        let mut queue = sink.lock().unwrap();
                        if queue.len() >= QUEUE {
                            queue.remove(0);
                        }
                        queue.push(message);
                    }
                }
            })
            .ok()?;
        Some(Self { queue, port })
    }

    /// From `ARGOS_GAZE_PORT`, or [`GAZE_PORT`]. `ARGOS_GAZE_PORT=0` turns gaze off.
    pub fn from_env() -> Option<Self> {
        let port = match std::env::var("ARGOS_GAZE_PORT") {
            Ok(text) => text.trim().parse().ok()?,
            Err(_) => GAZE_PORT,
        };
        (port != 0).then(|| Self::start(port)).flatten()
    }

    pub fn drain(&self) -> Vec<GazeMessage> {
        std::mem::take(&mut *self.queue.lock().unwrap())
    }
}

/// A screen area the operator can look at, in window coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Area {
    /// `card`, `marker`, `inspector`, `map`, `console`, ...
    pub kind: &'static str,
    /// The rover this area shows, if it shows one.
    pub rover: Option<u64>,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Area {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

/// What the operator is looking at.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub kind: &'static str,
    pub rover: Option<u64>,
}

/// The first area that contains the point, so list the most specific areas
/// first. Inside the window but in no area is `window`; outside it is
/// `off_window`.
pub fn hit(areas: &[Area], x: f32, y: f32, window: (f32, f32)) -> Target {
    if let Some(area) = areas.iter().find(|area| area.contains(x, y)) {
        return Target {
            kind: area.kind,
            rover: area.rover,
        };
    }
    let inside = x >= 0. && y >= 0. && x <= window.0 && y <= window.1;
    Target {
        kind: if inside { "window" } else { "off_window" },
        rover: None,
    }
}

/// One finished look at one target.
#[derive(Clone, Debug, PartialEq)]
pub struct Dwell {
    pub target: Target,
    pub start: f64,
    pub duration: f64,
}

/// Turns a stream of hit targets into dwell spans.
#[derive(Default)]
pub struct DwellTracker {
    current: Option<(Target, f64)>,
    candidate: Option<(Target, f64)>,
}

impl DwellTracker {
    /// Feed the target under the gaze at `time`. Returns the dwell that
    /// ended, once a new target has held for [`DWELL_SWITCH_S`].
    pub fn feed(&mut self, target: Target, time: f64) -> Option<Dwell> {
        let Some((current, since)) = &self.current else {
            self.current = Some((target, time));
            return None;
        };
        if *current == target {
            self.candidate = None;
            return None;
        }
        let candidate_since = match &self.candidate {
            Some((candidate, at)) if *candidate == target => *at,
            _ => {
                self.candidate = Some((target, time));
                return None;
            }
        };
        if time - candidate_since < DWELL_SWITCH_S {
            return None;
        }
        let ended = Dwell {
            target: current.clone(),
            start: *since,
            duration: candidate_since - since,
        };
        self.current = Some((target, candidate_since));
        self.candidate = None;
        Some(ended)
    }

    /// Close the open dwell, e.g. when the face is lost.
    pub fn finish(&mut self, time: f64) -> Option<Dwell> {
        self.candidate = None;
        let (target, since) = self.current.take()?;
        Some(Dwell {
            target,
            start: since,
            duration: time - since,
        })
    }
}

#[cfg(test)]
#[path = "../../../tests/argos-gpui/unit/gaze_tests.rs"]
mod tests;
