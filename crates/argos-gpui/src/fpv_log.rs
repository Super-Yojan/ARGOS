//! FPV in the session log.
//!
//! Until now the session log had nothing about video, so a failed or frozen
//! FPV call left no trace (2026-11-09: "there was a problem with FPV" could
//! not be traced from log 26a9241f or 3866429a). Every record here is
//! `kind: "fpv"` with an `event`:
//!
//! | event | when |
//! | --- | --- |
//! | `start`, `stop`, `rotate` | the operator pressed the button |
//! | `follow` | live video switched to a newly selected rover |
//! | `phase` | call phase or its detail text changed (Starting, Connecting, Connected, Failed, Closed) |
//! | `first_frame` | first decoded picture of a call, with seconds since Start |
//! | `frozen` | connected but no new picture for [`FROZEN_AFTER_S`] |
//! | `resumed` | pictures came back after `frozen`, with the gap (a call that ends while frozen puts `frozen_s` on its `phase` record) |
//! | `stats` | every [`STATS_EVERY_S`] while connected: fps, bitrate, RTT, size |
//!
//! The tracker only reads [`FpvSample`]s, so it is tested without a window.

use crate::FpvView;
use serde_json::{Value, json};

/// No new picture for this long while connected counts as frozen.
pub const FROZEN_AFTER_S: f64 = 2.0;
/// Stats record period while connected.
pub const STATS_EVERY_S: f64 = 10.0;

/// What the tracker needs from one look at the FPV view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FpvSample {
    pub phone_id: Option<u64>,
    pub session: Option<String>,
    pub phase: &'static str,
    pub detail: String,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub bitrate_kbps: f32,
    pub rtt_ms: Option<f32>,
    pub frame_seq: u64,
}

impl FpvSample {
    pub fn from_view(view: &FpvView) -> Self {
        Self {
            phone_id: view.phone_id,
            session: view.session.clone(),
            phase: view.phase.label(),
            detail: view.detail.clone(),
            codec: view.codec.clone(),
            width: view.width,
            height: view.height,
            fps: view.fps,
            bitrate_kbps: view.bitrate_kbps,
            rtt_ms: view.rtt_ms,
            frame_seq: view.frame_seq,
        }
    }
}

#[derive(Default)]
pub struct FpvLog {
    phase: &'static str,
    detail: String,
    /// Session time the current call was started (Start pressed or Starting seen).
    started_at: Option<f64>,
    first_frame_logged: bool,
    last_seq: u64,
    last_frame_at: Option<f64>,
    frozen_since: Option<f64>,
    last_stats_at: Option<f64>,
    /// Rover a follow switch was requested for, until video is on it.
    follow_pending: Option<u64>,
}

impl FpvLog {
    /// Record for an operator button press (`start`, `stop`, `rotate`).
    pub fn action(
        &mut self,
        now: f64,
        event: &str,
        rover_id: Option<u64>,
        turns: Option<u8>,
    ) -> Value {
        if event == "start" {
            self.started_at = Some(now);
        }
        let mut record = json!({ "kind": "fpv", "event": event, "rover_id": rover_id });
        if let Some(turns) = turns {
            record["rotation_deg"] = (u32::from(turns) * 90).into();
        }
        record
    }

    /// Compare one sample with the last and return the records to log.
    pub fn observe(&mut self, now: f64, sample: &FpvSample) -> Vec<Value> {
        let mut records = Vec::new();
        let base = |event: &str| {
            json!({
                "kind": "fpv",
                "event": event,
                "rover_id": sample.phone_id,
                "session": sample.session,
            })
        };
        if self.phase.is_empty() && sample.phase == "Idle" {
            // The window's first look: nothing has happened yet.
            self.phase = sample.phase;
            self.detail.clone_from(&sample.detail);
        }
        if sample.phase != self.phase || sample.detail != self.detail {
            if sample.phase == "Starting" && self.phase != "Starting" {
                self.started_at.get_or_insert(now);
                self.first_frame_logged = false;
                self.last_seq = sample.frame_seq;
                self.last_frame_at = None;
                self.frozen_since = None;
                self.last_stats_at = None;
            }
            let mut record = base("phase");
            record["phase"] = sample.phase.into();
            record["from"] = self.phase.into();
            record["detail"] = sample.detail.clone().into();
            if let Some(start) = self.started_at {
                record["since_start_s"] = (now - start).into();
            }
            if matches!(sample.phase, "Failed" | "Closed" | "Idle") {
                // A call that ends while frozen says how long it had been frozen.
                if let Some(since) = self.frozen_since.take() {
                    record["frozen_s"] = (now - since).into();
                }
                self.started_at = None;
                self.last_frame_at = None;
                self.last_stats_at = None;
            }
            records.push(record);
            self.phase = sample.phase;
            self.detail.clone_from(&sample.detail);
        }

        if sample.frame_seq != self.last_seq {
            self.last_seq = sample.frame_seq;
            if !self.first_frame_logged && sample.frame_seq > 0 {
                self.first_frame_logged = true;
                let mut record = base("first_frame");
                record["codec"] = sample.codec.clone().into();
                record["width"] = sample.width.into();
                record["height"] = sample.height.into();
                if let Some(start) = self.started_at {
                    record["since_start_s"] = (now - start).into();
                }
                records.push(record);
            }
            if let Some(since) = self.frozen_since.take() {
                let mut record = base("resumed");
                record["gap_s"] = (now - self.last_frame_at.unwrap_or(since)).into();
                records.push(record);
            }
            self.last_frame_at = Some(now);
        }

        if sample.phase == "Connected" {
            let quiet_since = self.last_frame_at.or(self.started_at);
            if self.frozen_since.is_none()
                && let Some(since) = quiet_since
                && now - since >= FROZEN_AFTER_S
            {
                self.frozen_since = Some(since);
                let mut record = base("frozen");
                record["no_frame_s"] = (now - since).into();
                record["frames"] = sample.frame_seq.into();
                record["rtt_ms"] = sample.rtt_ms.into();
                records.push(record);
            }
            if self
                .last_stats_at
                .is_none_or(|at| now - at >= STATS_EVERY_S)
            {
                self.last_stats_at = Some(now);
                let mut record = base("stats");
                record["codec"] = sample.codec.clone().into();
                record["width"] = sample.width.into();
                record["height"] = sample.height.into();
                record["fps"] = sample.fps.into();
                record["bitrate_kbps"] = sample.bitrate_kbps.into();
                record["rtt_ms"] = sample.rtt_ms.into();
                record["frames"] = sample.frame_seq.into();
                records.push(record);
            }
        }
        records
    }
}

impl FpvLog {
    /// Video follows the selection: while a call is live (starting,
    /// connecting or connected) and the operator selects another rover,
    /// return that rover once so the caller restarts FPV on it. The request
    /// is not repeated while the switch is on its way. Selecting nothing
    /// keeps the call.
    pub fn follow(
        &mut self,
        phase: &str,
        watching: Option<u64>,
        selected: Option<u64>,
    ) -> Option<u64> {
        if self
            .follow_pending
            .is_some_and(|id| watching == Some(id) || selected != Some(id))
        {
            self.follow_pending = None;
        }
        let live = matches!(phase, "Starting" | "Connecting" | "Connected");
        let target = selected.filter(|id| live && watching != Some(*id))?;
        if self.follow_pending == Some(target) {
            return None;
        }
        self.follow_pending = Some(target);
        Some(target)
    }
}

#[cfg(test)]
#[path = "../../../tests/argos-gpui/unit/fpv_log_tests.rs"]
mod tests;
