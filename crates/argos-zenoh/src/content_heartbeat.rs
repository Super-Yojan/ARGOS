//! A heartbeat confirms a freshly observed, unchanged sensor sample. It can
//! refresh an existing matching baseline, never create content or authorize control.

use argos_core::OccupancyGrid;
use serde_json::Value;
use std::time::Instant;

pub(super) struct Identity {
    frame: String,
    sequence: u64,
}

fn identity(bytes: &[u8], id: u64, version: &str, frame: &str) -> Option<Identity> {
    if bytes.len() > 1024 {
        return None;
    }
    let value: Value = serde_json::from_slice(bytes).ok()?;
    if value[version].as_u64() != Some(1) || value["rover_id"].as_u64() != Some(id) {
        return None;
    }
    let frame = value[frame].as_str()?;
    if frame.is_empty() || frame.len() > 256 {
        return None;
    }
    Some(Identity {
        frame: frame.into(),
        sequence: value["sequence"].as_u64()?,
    })
}

pub(super) fn occupancy_identity(bytes: &[u8], id: u64) -> Option<Identity> {
    identity(bytes, id, "schema_version", "run_id")
}

pub(super) fn cloud_identity(bytes: &[u8], id: u64) -> Option<Identity> {
    identity(bytes, id, "version", "frameID")
}

/// Sequenced clouds are immutable within a frame. Older/conflicting samples
/// cannot replace the baseline that subsequent heartbeats reference.
pub(super) fn accepts_cloud(previous: &Value, next: &Value) -> bool {
    let sequenced = |value: &Value| {
        value["version"].as_u64() == Some(1)
            && value["frameID"]
                .as_str()
                .is_some_and(|frame| !frame.is_empty())
            && value["sequence"].as_u64().is_some()
    };
    if !sequenced(previous) || !sequenced(next) || previous["frameID"] != next["frameID"] {
        // Preserve legacy unsequenced publishers and permit a new coordinate frame.
        return true;
    }
    let old = previous["sequence"].as_u64().unwrap();
    let new = next["sequence"].as_u64().unwrap();
    new > old
}

pub(super) fn refresh_grid(
    cached: &mut (OccupancyGrid, f64, Option<String>),
    heartbeat: &Identity,
    now: f64,
) {
    if cached.0.run_id == heartbeat.frame && cached.0.sequence == heartbeat.sequence {
        cached.1 = now;
    }
}

pub(super) fn refresh_cloud(cached: &mut (Value, Instant), heartbeat: &Identity, now: Instant) {
    if cached.0["version"].as_u64() == Some(1)
        && cached.0["frameID"].as_str() == Some(heartbeat.frame.as_str())
        && cached.0["sequence"].as_u64() == Some(heartbeat.sequence)
    {
        cached.1 = now;
    }
}

#[cfg(test)]
#[path = "../../../tests/argos-zenoh/unit/content_heartbeat_tests.rs"]
mod tests;
