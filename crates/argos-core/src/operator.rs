//! Ordered held-key input, authority generations, and operator message validation.
//! Run changes retire old authority so delayed reports cannot revive old commands.

use crate::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Default)]
pub struct HeldInput {
    keys: BTreeSet<String>,
}
impl HeldInput {
    pub fn press(&mut self, key: &str) {
        if matches!(key, "w" | "s" | "a" | "d") {
            self.keys.insert(key.into());
        }
    }
    pub fn release(&mut self, key: &str) {
        self.keys.remove(key);
    }
    pub fn clear(&mut self) {
        self.keys.clear();
    }
    pub fn sample(&self) -> (f64, f64) {
        (
            (self.keys.contains("w") as i8 - self.keys.contains("s") as i8) as f64,
            (self.keys.contains("a") as i8 - self.keys.contains("d") as i8) as f64,
        )
    }
}
/// Per-key ordering preserves independent releases; clear fences every earlier press.
#[derive(Default)]
pub struct InputOrder {
    keys: BTreeMap<String, u64>,
    cleared: u64,
}
impl InputOrder {
    pub fn accept(&mut self, key: &str, sequence: u64) -> bool {
        if sequence <= self.cleared || self.keys.get(key).is_some_and(|last| sequence <= *last) {
            return false;
        }
        self.keys.insert(key.into(), sequence);
        true
    }
    pub fn clear(&mut self, sequence: u64) -> bool {
        if sequence <= self.cleared {
            return false;
        }
        self.cleared = sequence;
        self.keys.retain(|_, status| *status > sequence);
        true
    }
}
#[derive(Default)]
struct OperatorEntry {
    run_id: Option<String>,
    retired_runs: BTreeSet<String>,
    status: Option<(Value, f64)>,
    proposal: Option<Value>,
    mission: Option<Value>,
    experiment: Option<(Value, f64)>,
    pending: Option<(String, f64, String)>,
}
#[derive(Default)]
pub struct OperatorCache {
    entries: BTreeMap<u64, OperatorEntry>,
}
impl OperatorCache {
    pub fn generation(&self, id: u64) -> Option<(String, u64)> {
        let (status, _) = self.entries.get(&id)?.status.as_ref()?;
        Some((
            status["run_id"].as_str()?.into(),
            status["revision"].as_u64()?,
        ))
    }
    pub fn matches_authority(&self, id: u64, run: &str, revision: u64) -> bool {
        self.entries
            .get(&id)
            .and_then(|entry| entry.status.as_ref())
            .is_some_and(|(status, _)| {
                status["run_id"].as_str() == Some(run)
                    && status["revision"].as_u64() == Some(revision)
            })
    }
    pub fn run_id(&self, id: u64) -> Option<String> {
        self.entries.get(&id).and_then(|entry| entry.run_id.clone())
    }
    pub fn pending(&mut self, id: u64, token: String, now: f64) {
        self.entries.entry(id).or_default().pending = Some((token, now, "pending".into()));
    }
    pub fn apply(&mut self, id: u64, kind: &str, bytes: &[u8], now: f64) -> Result<()> {
        if bytes.len() > MAX_STATE_BYTES || !now.is_finite() {
            return Err(invalid("invalid operator state"));
        }
        let value: Value =
            serde_json::from_slice(bytes).map_err(|_| invalid("malformed operator state"))?;
        let entry = self.entries.entry(id).or_default();
        if matches!(kind, "autonomy/status" | "experiment/status")
            && let Some(run) = value["run_id"].as_str()
        {
            if entry.retired_runs.contains(run) {
                return Ok(());
            }
            if entry.run_id.as_deref() != Some(run) {
                let mut retired = std::mem::take(&mut entry.retired_runs);
                if let Some(old) = entry.run_id.take() {
                    retired.insert(old);
                }
                *entry = OperatorEntry {
                    run_id: Some(run.into()),
                    retired_runs: retired,
                    ..Default::default()
                };
            }
        }
        if kind == "goal/proposal"
            && !value.is_null()
            && entry.run_id.as_deref() != value["run_id"].as_str()
        {
            return Ok(());
        }
        match kind {
            "autonomy/status" => {
                let level = value["requested_level"]
                    .as_str()
                    .ok_or_else(|| invalid("missing level"))?;
                if !matches!(
                    level,
                    "teleop"
                        | "assisted_teleop"
                        | "waypoint"
                        | "waypoint_direct"
                        | "supervised"
                        | "target_search"
                ) || value["revision"].as_u64().is_none()
                {
                    return Err(invalid("invalid authority"));
                }
                if entry.status.as_ref().is_some_and(|(status, _)| {
                    status["revision"].as_u64() > value["revision"].as_u64()
                }) {
                    return Ok(());
                }
                if let Some((token, at, phase)) = &mut entry.pending
                    && value["token"].as_str() == Some(token)
                    && now >= *at
                {
                    *phase = value["result"].as_str().unwrap_or("unconfirmed").into();
                }
                entry.status = Some((value, now));
            }
            "goal/proposal" => {
                entry.proposal = if value.is_null() {
                    None
                } else {
                    if value["proposal_id"].as_u64().is_none()
                        || ["x", "y", "expires_at"]
                            .iter()
                            .any(|k| value[*k].as_f64().is_none_or(|n| !n.is_finite()))
                    {
                        return Err(invalid("invalid proposal"));
                    }
                    Some(value)
                };
            }
            "mission/status" => entry.mission = Some(value),
            "experiment/status" => entry.experiment = Some((value, now)),
            _ => return Err(invalid("unknown state topic")),
        }
        Ok(())
    }
    pub fn can_drive(&self, id: u64, now: f64) -> bool {
        self.entries
            .get(&id)
            .filter(|entry| {
                entry
                    .pending
                    .as_ref()
                    .is_none_or(|p| p.2 == "accepted" || p.2 == "rejected")
            })
            .and_then(|entry| entry.status.as_ref())
            .is_some_and(|(status, t)| {
                now - *t < STALE_SECONDS
                    && status["safety"] == "clear"
                    && matches!(
                        status["effective_level"].as_str(),
                        Some("teleop" | "assisted_teleop")
                    )
            })
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn retain(&mut self, ids: &[u64]) {
        self.entries.retain(|id, _| ids.contains(id));
    }
    pub fn snapshot(&self, now: f64) -> Value {
        let mut out = serde_json::Map::new();
        for (id, entry) in &self.entries {
            let phase = entry
                .pending
                .as_ref()
                .map(|(_, t, p)| {
                    if p == "pending" && now - *t >= 2. {
                        "unconfirmed"
                    } else {
                        p.as_str()
                    }
                })
                .unwrap_or("none");
            let remaining = entry
                .proposal
                .as_ref()
                .and_then(|p| p["expires_at"].as_f64())
                .zip(
                    entry
                        .experiment
                        .as_ref()
                        .and_then(|status| status.0["run_elapsed"].as_f64()),
                )
                .map(|(expiry, elapsed)| {
                    expiry
                        - elapsed
                        - entry
                            .experiment
                            .as_ref()
                            .map(|status| now - status.1)
                            .unwrap_or(0.)
                });
            out.insert(
                id.to_string(),
                json!({
                    "proposal_remaining": remaining,
                    "status": entry.status.as_ref().map(|status|&status.0),
                    "age": entry.status.as_ref().map(|status|now-status.1),
                    "proposal": entry.proposal,
                    "mission": entry.mission,
                    "experiment": entry.experiment.as_ref().map(|status|&status.0),
                    "action_phase": phase,
                    "action_token": entry.pending.as_ref().map(|p|&p.0),
                }),
            );
        }
        Value::Object(out)
    }
}
pub fn operator_payload(kind: &str, mut value: Value, token: &str) -> Result<Vec<u8>> {
    if !valid_token(token) || !value.is_object() {
        return Err(invalid("invalid action"));
    }
    let fields: &[&str] = match kind {
        "autonomy" => {
            if !matches!(
                value["level"].as_str(),
                Some(
                    "teleop"
                        | "assisted_teleop"
                        | "waypoint"
                        | "waypoint_direct"
                        | "supervised"
                        | "target_search"
                )
            ) {
                return Err(invalid("invalid level"));
            }
            &["level"]
        }
        "safety" => {
            if !matches!(value["action"].as_str(), Some("stop" | "reset")) {
                return Err(invalid("invalid safety action"));
            }
            &["action"]
        }
        "goal/decision" => {
            if !matches!(
                value["decision"].as_str(),
                Some("approve" | "reject" | "resume")
            ) {
                return Err(invalid("invalid decision"));
            }
            if value["decision"] != "resume" && value["proposal_id"].as_u64().is_none() {
                return Err(invalid("missing proposal ID"));
            }
            if value["run_id"]
                .as_str()
                .is_none_or(|status| !valid_token(status))
            {
                return Err(invalid("missing run identity"));
            }
            &["decision", "proposal_id", "run_id"]
        }
        "mission/report" => {
            if value["survivor_id"].as_u64().is_none() {
                return Err(invalid("missing survivor ID"));
            }
            &["survivor_id"]
        }
        _ => return Err(invalid("unknown action")),
    };
    if value
        .as_object()
        .unwrap()
        .keys()
        .any(|k| !fields.contains(&k.as_str()))
    {
        return Err(invalid("unexpected action field"));
    }
    value["token"] = token.into();
    serde_json::to_vec(&value).map_err(|entry| Error(entry.to_string()))
}
