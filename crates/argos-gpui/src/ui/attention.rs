//! Session log records for operator attention: which rover is selected or on
//! video, whether the window is in front, what each rover card says, rover
//! poses once a second, and eye tracker gaze and blinks mapped to rovers.
//! `tools/attention_times.py` turns these into wait, interaction and neglect
//! time per rover.

use crate::Dashboard;
use argos_gpui::FpvPhase;
use argos_gpui::console::Callout;
use argos_gpui::gaze::{Area, DwellTracker, GazeMessage, GazeReceiver, Target, hit};
use gpui_kit::*;
use serde_json::json;
use std::collections::BTreeMap;

/// Rover poses go to the log this often (seconds).
const POSE_PERIOD_S: f64 = 1.0;
/// Raw gaze points go to the log at most this often (seconds). Dwell spans
/// and blinks are logged in full.
const GAZE_SAMPLE_PERIOD_S: f64 = 0.1;
/// Half the side of the square around a rover's map marker that counts as
/// looking at it, in rem. Webcam gaze is good to a few degrees at best.
const MARKER_REACH_REM: f32 = 3.;

#[derive(Default)]
pub(crate) struct AttentionLog {
    selected: Option<Option<u64>>,
    video: Option<Option<u64>>,
    window_active: Option<bool>,
    callouts: BTreeMap<u64, (String, String)>,
    last_pose: f64,
    gaze: Option<GazeReceiver>,
    gaze_started: bool,
    last_gaze_sample: f64,
    dwell: DwellTracker,
    face: Option<bool>,
}

impl AttentionLog {
    pub(crate) fn start() -> Self {
        Self {
            gaze: GazeReceiver::from_env(),
            ..Self::default()
        }
    }
}

/// Where a stack of rover cards sits, in field coordinates (see `card_group`).
pub(super) struct CardStack {
    pub ids: Vec<u64>,
    pub x: f32,
    pub y: f32,
    pub below: bool,
    pub leading: bool,
}

impl Dashboard {
    /// Called from `render` with what it measured this frame. Field
    /// coordinates are relative to the map area (`field_frame`).
    pub(super) fn log_attention(
        &mut self,
        window: &Window,
        field: &[(u64, Option<Point<Pixels>>, Callout)],
        stacks: &[CardStack],
        card_size: (f32, f32),
    ) {
        let now = self.started.elapsed().as_secs_f64();
        let log = &mut self.attention;
        let feed = &self.feed;
        if log.selected != Some(self.selected) {
            feed.record(json!({
                "kind": "rover_focus",
                "via": "select",
                "rover_id": self.selected,
                "previous": log.selected.flatten(),
                "help_id": self.selected.and_then(|id| feed.open_help(id)),
            }));
            log.selected = Some(self.selected);
        }
        let fpv = self.fpv.view();
        let video = matches!(
            fpv.phase,
            FpvPhase::Starting | FpvPhase::Connecting | FpvPhase::Connected
        )
        .then_some(fpv.phone_id)
        .flatten();
        if log.video != Some(video) {
            feed.record(json!({
                "kind": "rover_focus",
                "via": "video",
                "rover_id": video,
                "previous": log.video.flatten(),
            }));
            log.video = Some(video);
        }
        let active = window.is_window_active();
        if log.window_active != Some(active) {
            feed.record(json!({"kind": "window_focus", "active": active}));
            log.window_active = Some(active);
        }
        for (id, _, callout) in field {
            let shown = (
                callout
                    .tag
                    .map(|tag| format!("{tag:?}").to_lowercase())
                    .unwrap_or_else(|| "none".into()),
                callout.state.clone(),
            );
            if log.callouts.get(id) != Some(&shown) {
                feed.record(json!({
                    "kind": "callout",
                    "rover_id": id,
                    "tag": shown.0,
                    "state": shown.1,
                    "attention": callout.attention,
                    "recovery": callout.recovery,
                }));
                log.callouts.insert(*id, shown);
            }
        }
        if now - log.last_pose >= POSE_PERIOD_S {
            log.last_pose = now;
            let rovers: Vec<_> = self
                .marks
                .iter()
                .map(|mark| {
                    json!({
                        "id": mark.id,
                        "x": mark.local_x,
                        "y": mark.local_y,
                        "yaw": mark.yaw,
                        "latitude": mark.latitude,
                        "longitude": mark.longitude,
                        "status": mark.status,
                        "level": mark.effective_level,
                    })
                })
                .collect();
            if !rovers.is_empty() {
                feed.record(json!({"kind": "pose_sample", "rovers": rovers}));
            }
        }
        let Some(gaze) = log.gaze.clone() else {
            return;
        };
        if !log.gaze_started {
            log.gaze_started = true;
            feed.record(json!({"kind": "gaze_status", "event": "listening", "port": gaze.port}));
        }
        let messages = gaze.drain();
        if messages.is_empty() {
            return;
        }
        // The window's content area on screen. `bounds` is the whole window;
        // a title bar takes the difference at the top.
        let bounds = window.bounds();
        let viewport = window.viewport_size();
        let origin_x = f32::from(bounds.origin.x);
        let origin_y = f32::from(bounds.origin.y) + f32::from(bounds.size.height - viewport.height);
        let size = (f32::from(viewport.width), f32::from(viewport.height));
        let areas = self.attention_areas(window, field, stacks, card_size);
        let log = &mut self.attention;
        for message in messages {
            match message {
                GazeMessage::Gaze {
                    utc,
                    x,
                    y,
                    confidence,
                } => {
                    let wx = x as f32 - origin_x;
                    let wy = y as f32 - origin_y;
                    let target = hit(&areas, wx, wy, size);
                    if log.face != Some(true) {
                        log.face = Some(true);
                        feed.record(
                            json!({"kind": "gaze_status", "event": "face", "tracker_utc": utc}),
                        );
                    }
                    if now - log.last_gaze_sample >= GAZE_SAMPLE_PERIOD_S {
                        log.last_gaze_sample = now;
                        feed.record(json!({
                            "kind": "gaze_sample",
                            "tracker_utc": utc,
                            "screen_x": x,
                            "screen_y": y,
                            "x": wx,
                            "y": wy,
                            "confidence": confidence,
                            "area": target.kind,
                            "rover_id": target.rover,
                        }));
                    }
                    if let Some(dwell) = log.dwell.feed(target, utc) {
                        record_dwell(feed, &dwell);
                    }
                }
                GazeMessage::NoFace { utc } => {
                    if log.face != Some(false) {
                        log.face = Some(false);
                        feed.record(
                            json!({"kind": "gaze_status", "event": "no_face", "tracker_utc": utc}),
                        );
                        if let Some(dwell) = log.dwell.finish(utc) {
                            record_dwell(feed, &dwell);
                        }
                    }
                }
                GazeMessage::Blink { utc, duration_ms } => {
                    feed.record(json!({
                        "kind": "blink",
                        "tracker_utc": utc,
                        "duration_ms": duration_ms,
                    }));
                }
                GazeMessage::Status(mut status) => {
                    status["kind"] = "gaze_status".into();
                    status["event"] = "tracker".into();
                    status["tracker_utc"] = status["utc"].take();
                    status["window"] = json!({
                        "x": origin_x, "y": origin_y, "width": size.0, "height": size.1,
                    });
                    feed.record(status);
                }
            }
        }
    }

    /// Screen areas in window coordinates, most specific first: rover cards,
    /// the inspector (the selected rover), map markers, the console bar, the map.
    fn attention_areas(
        &self,
        window: &Window,
        field: &[(u64, Option<Point<Pixels>>, Callout)],
        stacks: &[CardStack],
        card_size: (f32, f32),
    ) -> Vec<Area> {
        let Some(frame) = *self.field_frame.lock().unwrap() else {
            return Vec::new();
        };
        let fx = f32::from(frame.origin.x);
        let fy = f32::from(frame.origin.y);
        let rem = f32::from(window.rem_size());
        let mut areas = Vec::new();
        for stack in stacks {
            let height = card_size.1 * stack.ids.len() as f32;
            let left = if stack.leading {
                stack.x - rem - card_size.0
            } else {
                stack.x + rem
            };
            for (index, id) in stack.ids.iter().enumerate() {
                let offset = if stack.below {
                    rem + card_size.1 * index as f32
                } else {
                    -rem - height + card_size.1 * index as f32
                };
                areas.push(Area {
                    kind: "card",
                    rover: Some(*id),
                    x: fx + left,
                    y: fy + stack.y + offset,
                    width: card_size.0,
                    height: card_size.1,
                });
            }
        }
        if let Some(inspector) = *self.inspector_frame.lock().unwrap() {
            areas.push(Area {
                kind: "inspector",
                rover: self.selected,
                x: f32::from(inspector.origin.x),
                y: f32::from(inspector.origin.y),
                width: f32::from(inspector.size.width),
                height: f32::from(inspector.size.height),
            });
        }
        let reach = MARKER_REACH_REM * rem;
        for (id, at, _) in field {
            if let Some(at) = at {
                areas.push(Area {
                    kind: "marker",
                    rover: Some(*id),
                    x: fx + f32::from(at.x) - reach,
                    y: fy + f32::from(at.y) - reach,
                    width: 2. * reach,
                    height: 2. * reach,
                });
            }
        }
        areas.push(Area {
            kind: "console",
            rover: None,
            x: 0.,
            y: 0.,
            width: f32::from(window.viewport_size().width),
            height: fy,
        });
        areas.push(Area {
            kind: "map",
            rover: None,
            x: fx,
            y: fy,
            width: f32::from(frame.size.width),
            height: f32::from(frame.size.height),
        });
        areas
    }
}

fn record_dwell(feed: &argos_gpui::FleetFeed, dwell: &argos_gpui::gaze::Dwell) {
    let Target { kind, rover } = &dwell.target;
    feed.record(json!({
        "kind": "gaze_dwell",
        "area": kind,
        "rover_id": rover,
        // Tracker clock (Unix seconds), like each record's `utc`.
        "start_utc": dwell.start,
        "duration_s": dwell.duration,
        "help_id": rover.and_then(|id| feed.open_help(id)),
    }));
}
