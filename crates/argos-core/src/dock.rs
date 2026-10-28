//! Bucket dock: Terra lines its dump outlet up over a found bucket, then opens
//! the dump gate.
//!
//! The status is `dock` inside `autonomy/status`. Requests go on `dock`:
//! `start` on a found bucket, `dump` to approve the gate at L0–L3 (L4 and L5
//! dump on their own), `cancel` to end it. See Terra `docs/autonomy/DOCK.md`.
use crate::{Result, invalid, valid_token};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub struct DockReport {
    pub dock_id: u64,
    /// `lock`, `creep`, `settle`, `aligned`, `dumping`, `closing`, `backoff`,
    /// `done`, `failed` or `cancelled`.
    pub phase: String,
    pub reason: String,
    pub object_id: Option<u32>,
    pub color: Option<String>,
    /// Bucket centre left of the rover axis, metres.
    pub lateral_m: f64,
    /// Bucket centre beyond the dock range, metres.
    pub range_m: f64,
    pub heading_deg: f64,
    pub in_tolerance: bool,
    pub fix_age_s: Option<f64>,
    pub retries: u32,
    /// Aligned on a fresh camera fix, not on odometry alone.
    pub verified: bool,
    /// `operator` (L0–L3) or `auto` (L4–L5).
    pub approval: String,
    pub awaiting_approval: bool,
    pub gate_open: bool,
}

impl DockReport {
    pub fn active(&self) -> bool {
        !matches!(self.phase.as_str(), "done" | "failed" | "cancelled")
    }

    /// "Orange bucket 4" or "Bucket 4".
    pub fn target_label(&self) -> String {
        let id = self
            .object_id
            .map(|id| format!(" {id}"))
            .unwrap_or_default();
        match &self.color {
            Some(color) => {
                let mut label = format!("{color} bucket{id}");
                if let Some(first) = label.get_mut(..1) {
                    first.make_ascii_uppercase();
                }
                label
            }
            None => format!("Bucket{id}"),
        }
    }

    /// One line for the inspector and the pop-up.
    pub fn headline(&self) -> String {
        let phase = match self.phase.as_str() {
            "lock" => "Locking on",
            "creep" => "Lining up",
            "settle" => "Settling",
            "aligned" if self.awaiting_approval => "Aligned · approve the dump",
            "aligned" => "Aligned",
            "dumping" => "Gate open · dumping",
            "closing" => "Closing gate",
            "backoff" => "Backing off to retry",
            "done" => "Dumped",
            "failed" => "Dock failed",
            "cancelled" => "Dock cancelled",
            other => other,
        };
        format!("{} · {phase}", self.target_label())
    }

    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = vec![
            ("Dock", self.headline()),
            (
                "Error",
                format!(
                    "{:+.2} m side · {:+.2} m range · {:+.0}°",
                    self.lateral_m, self.range_m, self.heading_deg
                ),
            ),
        ];
        if !self.reason.is_empty() && self.reason != "started" {
            rows.push(("Reason", self.reason.replace('_', " ")));
        }
        rows.push((
            "Camera fix",
            match self.fix_age_s {
                Some(age) => format!("{age:.1} s ago"),
                None => "none yet".into(),
            },
        ));
        rows.push((
            "Dump",
            if self.approval == "auto" {
                "automatic at L4–L5".into()
            } else {
                "needs your approval".into()
            },
        ));
        if self.phase == "aligned" && !self.verified {
            rows.push(("Check", "finished on odometry; check the camera".into()));
        }
        if self.retries > 0 {
            rows.push(("Retries", self.retries.to_string()));
        }
        rows
    }
}

/// `autonomy/status.dock`, when a dock has started this run.
pub fn decode_dock(value: &Value) -> Option<DockReport> {
    let dock = value.get("dock")?;
    let errors = &dock["errors"];
    let finite = |v: &Value| v.as_f64().filter(|v| v.is_finite());
    Some(DockReport {
        dock_id: dock["dock_id"].as_u64()?,
        phase: dock["phase"].as_str()?.to_string(),
        reason: dock["reason"].as_str().unwrap_or_default().to_string(),
        object_id: dock["object_id"]
            .as_u64()
            .and_then(|id| u32::try_from(id).ok()),
        color: dock["color"].as_str().map(str::to_string),
        lateral_m: finite(&errors["lateral"]).unwrap_or(0.),
        range_m: finite(&errors["range"]).unwrap_or(0.),
        heading_deg: finite(&errors["heading_deg"]).unwrap_or(0.),
        in_tolerance: dock["in_tolerance"].as_bool().unwrap_or(false),
        fix_age_s: finite(&dock["fix_age"]),
        retries: dock["retries"].as_u64().unwrap_or(0) as u32,
        verified: dock["verified"].as_bool().unwrap_or(false),
        approval: dock["approval"].as_str().unwrap_or("operator").to_string(),
        awaiting_approval: dock["awaiting_approval"].as_bool().unwrap_or(false),
        gate_open: dock["gate_open"].as_bool().unwrap_or(false),
    })
}

/// `dock` start on a found bucket.
pub fn encode_dock_start(object_id: u32, token: &str) -> Result<Vec<u8>> {
    encode(
        json!({"action": "start", "object_id": object_id, "token": token}),
        token,
    )
}

/// `dock` dump approval for `dock_id`.
pub fn encode_dock_dump(dock_id: u64, token: &str) -> Result<Vec<u8>> {
    encode(
        json!({"action": "dump", "dock_id": dock_id, "token": token}),
        token,
    )
}

/// `dock` cancel. Neutral like a goal cancel: it needs no authority revision.
pub fn encode_dock_cancel(token: &str) -> Result<Vec<u8>> {
    encode(json!({"action": "cancel", "token": token}), token)
}

fn encode(value: Value, token: &str) -> Result<Vec<u8>> {
    if !valid_token(token) {
        return Err(invalid("invalid dock token"));
    }
    serde_json::to_vec(&value).map_err(|e| crate::Error(e.to_string()))
}

/// Check a `dock` request before it is sent.
pub fn validate_dock_action(payload: &str) -> Result<Value> {
    if payload.len() > 2048 {
        return Err(invalid("dock request too large"));
    }
    let v: Value = serde_json::from_str(payload).map_err(|_| invalid("invalid dock JSON"))?;
    let o = v
        .as_object()
        .ok_or_else(|| invalid("dock request object required"))?;
    if v["token"].as_str().is_none_or(|s| !valid_token(s)) {
        return Err(invalid("invalid dock token"));
    }
    let required = match v["action"].as_str() {
        Some("start") => Some("object_id"),
        Some("dump") => Some("dock_id"),
        Some("cancel") => None,
        _ => return Err(invalid("dock action is start, dump or cancel")),
    };
    if required.is_some_and(|key| v[key].as_u64().is_none()) {
        return Err(invalid("dock request is missing its id"));
    }
    let allowed = [
        "action",
        "token",
        "object_id",
        "dock_id",
        "run_id",
        "authority_revision",
    ];
    if o.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(invalid("unknown dock field"));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(phase: &str, awaiting: bool) -> Value {
        json!({"dock": {
            "schema_version": 1, "dock_id": 2, "token": "t", "phase": phase,
            "reason": "verified", "object_id": 4, "color": "orange",
            "target": [1.0, 2.0],
            "errors": {"lateral": 0.01, "range": -0.02, "heading_deg": 1.2},
            "in_tolerance": true, "fix_age": 0.2, "fixes": 9, "retries": 0,
            "verified": true, "approval": "operator", "awaiting_approval": awaiting,
            "gate_open": false, "elapsed": 12.0
        }})
    }

    #[test]
    fn decodes_terra_dock_status() {
        let report = decode_dock(&status("aligned", true)).unwrap();
        assert_eq!(report.dock_id, 2);
        assert!(report.awaiting_approval);
        assert!(report.active());
        assert_eq!(
            report.headline(),
            "Orange bucket 4 · Aligned · approve the dump"
        );
        assert!(decode_dock(&json!({})).is_none());
        assert!(!decode_dock(&status("done", false)).unwrap().active());
    }

    #[test]
    fn requests_round_trip_through_validation() {
        for bytes in [
            encode_dock_start(4, "a").unwrap(),
            encode_dock_dump(2, "b").unwrap(),
            encode_dock_cancel("c").unwrap(),
        ] {
            validate_dock_action(std::str::from_utf8(&bytes).unwrap()).unwrap();
        }
        assert!(validate_dock_action(r#"{"action":"dump","token":"a"}"#).is_err());
        assert!(validate_dock_action(r#"{"action":"open","token":"a"}"#).is_err());
        assert!(validate_dock_action(r#"{"action":"cancel","token":"a","x":1}"#).is_err());
        assert!(encode_dock_cancel("bad token").is_err());
    }
}
