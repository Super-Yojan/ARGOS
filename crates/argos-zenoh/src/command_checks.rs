//! The checks `Client::operator_command` runs before it publishes anything.
//!
//! Three stages, in order: is the payload well formed for its topic, is the
//! command neutral (allowed even with stale telemetry), and does the rover's
//! current authority, hardware, and run still allow it. Terra repeats its
//! own checks; these keep the dashboard from sending what Terra would refuse.

use crate::{Client, require_relay};
use argos_core::*;

/// Reject a payload whose fields do not fit its topic `kind`.
pub(crate) fn check_command_shape(
    kind: &str,
    payload: &str,
    value: &serde_json::Value,
) -> Result<()> {
    if kind == "navigation/settings" {
        if value["token"]
            .as_str()
            .is_none_or(|v| v.is_empty() || v.len() > 64)
            || !value["run_id"].is_string()
            || value["authority_revision"].as_u64().is_none()
            || value["settings_revision"].as_u64().is_none()
            || !value["parameters"].is_object()
        {
            return Err(invalid("invalid navigation settings request"));
        }
    } else if matches!(kind, "navigation/source" | "navigation/resume") {
        if value["schema_version"] != 1
            || value["token"].as_str().is_none_or(|t| !valid_token(t))
            || !value["run_id"].is_string()
            || value["navigation_revision"].as_u64().is_none()
        {
            return Err(invalid("invalid navigation request"));
        }
    } else if kind.starts_with("search") {
        validate_search_command(kind, payload)?;
    } else if kind == "task/action" {
        argos_core::validate_task_action(payload)?;
    } else if kind == "dock" {
        argos_core::validate_dock_action(payload)?;
    } else if kind == "teleop" {
        let linear = value["linear"]
            .as_f64()
            .ok_or_else(|| invalid("missing linear"))?;
        let angular = value["angular"]
            .as_f64()
            .ok_or_else(|| invalid("missing angular"))?;
        if !linear.is_finite() || !angular.is_finite() || linear.abs() > 0.5 || angular.abs() > 1. {
            return Err(invalid("drive exceeds dashboard limits"));
        }
    } else if kind == "autonomy" && value.get("assigned_level").is_some() {
        let level = value["assigned_level"].as_str();
        if !(value["assigned_level"].is_null()
            || level.is_some_and(|level| {
                argos_core::STUDY_LEVELS
                    .iter()
                    .any(|(wire, _)| *wire == level)
            }))
            || value["token"].as_str().is_none_or(|t| !valid_token(t))
            || value.as_object().is_some_and(|o| o.len() != 2)
        {
            return Err(invalid("invalid study condition"));
        }
    } else if !matches!(
        value["level"].as_str(),
        Some(
            "teleop"
                | "assisted_teleop"
                | "waypoint"
                | "waypoint_direct"
                | "supervised"
                | "target_search"
                | "explore"
                | "task"
        )
    ) {
        return Err(invalid("unsupported autonomy level"));
    } else if value["level"] == "explore"
        && value["budget_seconds"]
            .as_f64()
            .is_none_or(|seconds| !argos_core::valid_explore_budget(seconds))
    {
        return Err(invalid("explore needs a budget"));
    } else if value["level"] == "task"
        && value["task"]["instruction"]
            .as_str()
            .is_none_or(|text| argos_core::task_text(text).is_err())
    {
        return Err(invalid("L5 needs a task"));
    }
    Ok(())
}

/// Stops, cancels, report acks, and a zero twist. These may be sent while
/// telemetry is lost; every other command needs fresh fleet and pose data.
pub(crate) fn is_neutral(kind: &str, value: &serde_json::Value) -> bool {
    kind == "search/report/ack"
        || (kind == "search/action" && matches!(value["action"].as_str(), Some("cancel" | "pause")))
        || (kind == "task/action" && value["action"] == "cancel")
        || (kind == "dock" && value["action"] == "cancel")
        || kind == "teleop"
            && value["linear"].as_f64() == Some(0.)
            && value["angular"].as_f64() == Some(0.)
}

impl Client {
    /// The rover's last authority report (and hardware, outdoor status where
    /// the command needs them) must still allow this command.
    pub(crate) fn check_vehicle_authority(
        &self,
        id: u64,
        kind: &str,
        value: &serde_json::Value,
        freshness: f64,
    ) -> Result<()> {
        let states = self.operator_states.lock().unwrap();
        let (authority, received) = states
            .get(&(id, "authority".into()))
            .ok_or_else(|| invalid("vehicle authority unavailable"))?;
        if received.elapsed().as_secs_f64() >= freshness {
            return Err(invalid("vehicle authority is stale"));
        }
        if kind.starts_with("search") {
            if value["run_id"] != authority["run_id"]
                || authority["requested_level"] != "target_search"
            {
                return Err(invalid("search authority changed"));
            }
        } else if kind == "task/action" {
            if authority["requested_level"] != "task" {
                return Err(invalid("Select L5 before replying to a task"));
            }
        } else if kind == "navigation/settings" {
            let (hardware, at) = states
                .get(&(id, "hardware".into()))
                .ok_or_else(|| invalid("hardware status unavailable"))?;
            require_relay(hardware, at, true)?;
            if value["run_id"] != authority["run_id"]
                || value["authority_revision"] != authority["revision"]
                || value["settings_revision"] != authority["navigation"]["revision"]
                || authority["navigation"]["schema_version"] != 1
                || !matches!(authority["safety"].as_str(), Some("clear" | "hold"))
            {
                return Err(invalid("Settings require current vehicle authority"));
            }
        } else if kind == "navigation/resume" {
            let (raw, at) = states
                .get(&(id, "outdoor".into()))
                .ok_or_else(|| invalid("Outdoor status unavailable"))?;
            let mut report = argos_core::OutdoorReport::decode(raw)?;
            report.age_s = Some(at.elapsed().as_secs_f64());
            if let Some(reason) = argos_core::outdoor_resume_refusal(
                &report,
                value["run_id"].as_str().unwrap(),
                value["navigation_revision"].as_u64().unwrap(),
            ) {
                return Err(invalid(reason));
            }
            let (hardware, at) = states
                .get(&(id, "hardware".into()))
                .ok_or_else(|| invalid("Hardware unavailable"))?;
            require_relay(hardware, at, false)?;
            if !authority["requested_level"]
                .as_str()
                .is_some_and(|level| report.accepts_goal_at(level))
                || authority["safety"] == "emergency_stop"
            {
                return Err(invalid(if report.goal_levels.len() > 1 {
                    "Select L2–L5 with ready hardware before resuming"
                } else {
                    "Select L3 with ready hardware before resuming"
                }));
            }
        } else if kind == "navigation/source" {
            let source = value["source"].as_str();
            if value["schema_version"] != 1
                || !matches!(source, Some("local" | "outdoor_gps"))
                || value["token"]
                    .as_str()
                    .is_none_or(|token| !valid_token(token))
                || !value["run_id"].is_string()
                || value["navigation_revision"].as_u64().is_none()
            {
                return Err(invalid("invalid navigation source request"));
            }
            let (hardware, at) = states
                .get(&(id, "hardware".into()))
                .ok_or_else(|| invalid("hardware status unavailable"))?;
            require_relay(hardware, at, true)?;
            if value["run_id"] != authority["run_id"] {
                return Err(invalid("navigation run changed"));
            }
            if self.snapshot().rovers.iter().any(|rover| {
                rover.id == id
                    && rover
                        .goal
                        .as_ref()
                        .is_some_and(|goal| goal.state == GoalState::Active)
            }) {
                return Err(invalid(
                    "Navigation source changes require no active mission",
                ));
            }
            if source == Some("outdoor_gps") {
                let (outdoor, outdoor_at) = states
                    .get(&(id, "outdoor".into()))
                    .ok_or_else(|| invalid("App update required"))?;
                if outdoor_at.elapsed().as_secs_f64() >= 3.
                    || argos_core::OutdoorReport::decode(outdoor).is_err()
                    || argos_core::app_update_required(
                        argos_core::OutdoorReport::decode(outdoor).ok().as_ref(),
                    )
                    || outdoor["navigation_revision"] != value["navigation_revision"]
                    || outdoor["run_id"] != value["run_id"]
                {
                    return Err(invalid("App update required"));
                }
            }
        } else if kind == "teleop" {
            let (hardware, received) = states
                .get(&(id, "hardware".into()))
                .ok_or_else(|| invalid("hardware status unavailable"))?;
            require_relay(hardware, received, false)?;
            if value["run_id"] != authority["run_id"]
                || value["authority_revision"] != authority["revision"]
                || value["operator_session_id"]
                    .as_str()
                    .is_none_or(|s| s.is_empty() || s.len() > 64)
                || value["sequence"].as_u64().is_none()
                || !matches!(
                    authority["requested_level"].as_str(),
                    Some("teleop" | "assisted_teleop")
                )
                || authority["effective_level"] != authority["requested_level"]
                || !matches!(authority["safety"].as_str(), Some("clear" | "active"))
            {
                return Err(invalid("drive authority changed"));
            }
        } else if kind == "autonomy" && value.get("assigned_level").is_some() {
            // Validated above; Terra checks the level against its own contract.
        } else if kind == "dock" {
            // Start and dump move the rover or open the gate; Terra checks the
            // bucket, level and alignment against its own dock contract.
            let (hardware, at) = states
                .get(&(id, "hardware".into()))
                .ok_or_else(|| invalid("hardware status unavailable"))?;
            require_relay(hardware, at, false)?;
            if authority["safety"] == "emergency_stop" {
                return Err(invalid("Stop is active · reset it before docking"));
            }
        } else if value["token"]
            .as_str()
            .is_none_or(|s| s.is_empty() || s.len() > 64)
            || !authority["supported_levels"]
                .as_array()
                .is_some_and(|levels| levels.contains(&value["level"]))
        {
            return Err(invalid("unsupported vehicle autonomy request"));
        }
        Ok(())
    }
}
