//! L5 task autonomy on Terra's `task` level.
//!
//! The operator types a task here. Terra's phone runs its own language model:
//! Terra builds each prompt from the phone's occupancy map, the model picks
//! waypoints, and Terra checks every waypoint with A* and drives it on the L3
//! route. This station sends the task, answers the model's questions, and shows
//! Terra's task status, which is nested on `autonomy/status` as `task`.

use crate::{Result, invalid, valid_token};
use serde_json::{Value, json};

/// Budgets the console offers, in minutes.
pub const TASK_BUDGETS_MIN: &[u32] = &[3, 5, 10];
/// Terra's limit on a task instruction or a reply, in characters.
pub const TASK_TEXT_MAX_CHARS: usize = 500;
/// Terra's largest task budget.
pub const TASK_MAX_BUDGET_SECONDS: f64 = 3600.;

/// Trimmed text Terra accepts, or why not.
pub fn task_text(text: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(invalid("Type a task first"));
    }
    if text.chars().count() > TASK_TEXT_MAX_CHARS {
        return Err(invalid("Keep the task under 500 characters"));
    }
    if text.chars().any(|c| c.is_control() && c != '\n') {
        return Err(invalid("The task has control characters"));
    }
    Ok(text.to_string())
}

/// `{"level":"task","token","task":{"task_id","instruction","budget_seconds"}}`:
/// one message selects L5 and gives the task.
pub fn encode_task_level_request(
    token: &str,
    task_id: &str,
    instruction: &str,
    budget_seconds: f64,
) -> Result<Vec<u8>> {
    if !valid_token(token) || !valid_token(task_id) {
        return Err(invalid("invalid task token"));
    }
    if !(budget_seconds.is_finite()
        && budget_seconds > 0.
        && budget_seconds <= TASK_MAX_BUDGET_SECONDS)
    {
        return Err(invalid("invalid task budget"));
    }
    serde_json::to_vec(&json!({
        "level": "task",
        "token": token,
        "task": {
            "task_id": task_id,
            "instruction": task_text(instruction)?,
            "budget_seconds": budget_seconds,
        },
    }))
    .map_err(|e| crate::Error(e.to_string()))
}

/// `task/action` reply: answers the model's question or adds guidance.
pub fn encode_task_reply(task_id: &str, text: &str, token: &str) -> Result<Vec<u8>> {
    if !valid_token(token) || !valid_token(task_id) {
        return Err(invalid("invalid task token"));
    }
    serde_json::to_vec(&json!({
        "task_id": task_id,
        "action": "reply",
        "text": task_text(text)?,
        "token": token,
    }))
    .map_err(|e| crate::Error(e.to_string()))
}

/// `task/action` cancel. Neutral like a goal cancel: it needs no authority revision.
pub fn encode_task_cancel(task_id: &str, token: &str) -> Result<Vec<u8>> {
    if !valid_token(token) || !valid_token(task_id) {
        return Err(invalid("invalid task token"));
    }
    serde_json::to_vec(&json!({"task_id": task_id, "action": "cancel", "token": token}))
        .map_err(|e| crate::Error(e.to_string()))
}

/// Check a `task/action` before it is sent.
pub fn validate_task_action(payload: &str) -> Result<Value> {
    if payload.len() > 2048 {
        return Err(invalid("task action too large"));
    }
    let v: Value = serde_json::from_str(payload).map_err(|_| invalid("invalid task JSON"))?;
    let o = v
        .as_object()
        .ok_or_else(|| invalid("task action object required"))?;
    if ["task_id", "token"]
        .iter()
        .any(|k| v[k].as_str().is_none_or(|s| !valid_token(s)))
    {
        return Err(invalid("invalid task action"));
    }
    match v["action"].as_str() {
        Some("cancel") => {}
        Some("reply") => {
            task_text(v["text"].as_str().unwrap_or(""))?;
        }
        _ => return Err(invalid("task action is reply or cancel")),
    }
    let allowed = [
        "task_id",
        "action",
        "text",
        "token",
        "run_id",
        "authority_revision",
    ];
    if o.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(invalid("unknown task action field"));
    }
    Ok(v)
}

/// Why the console will not start a task now, or `None` when it may.
pub fn task_refusal(stopped: bool, outdoor_gps: bool, supported: bool) -> Option<&'static str> {
    if !supported {
        Some("This phone has no on-device model, so it does not offer L5")
    } else if stopped {
        Some("Reset stop before starting a task")
    } else if outdoor_gps {
        Some("Switch navigation to Local before starting a task")
    } else {
        None
    }
}

/// One validated waypoint of the model's plan, in local metres.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskWaypoint {
    pub x: f64,
    pub y: f64,
    /// Map id the model chose (`F2`, `O1`, `L1`) or `point`.
    pub label: String,
}

/// Terra's `TaskStatus`, the fields the console shows.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskReport {
    pub task_id: String,
    pub instruction: String,
    /// `planning`, `navigating`, `waiting_operator`, `completed`, `failed`, `cancelled`.
    pub phase: String,
    pub reason: String,
    pub step: u64,
    pub max_steps: u64,
    pub elapsed_s: f64,
    pub budget_s: f64,
    pub remaining_s: f64,
    pub model: Option<String>,
    pub waiting_s: Option<f64>,
    pub latency_s: Option<f64>,
    pub plan: Vec<TaskWaypoint>,
    pub question: Option<String>,
    pub rationale: Option<String>,
    pub last_error: Option<String>,
    /// `(kind, text)`, oldest first.
    pub history: Vec<(String, String)>,
}

impl TaskReport {
    pub fn running(&self) -> bool {
        matches!(
            self.phase.as_str(),
            "planning" | "navigating" | "waiting_operator"
        )
    }
    pub fn waiting_for_operator(&self) -> bool {
        self.phase == "waiting_operator"
    }
    /// One line for the console: what the task is doing, or how it ended.
    pub fn state_label(&self) -> String {
        match (self.phase.as_str(), self.reason.as_str()) {
            ("planning", "retrying_model") => "Asking the model again".into(),
            ("planning", _) => match self.waiting_s {
                Some(s) => format!("Model is thinking · {s:.0} s"),
                None => "Waiting for the model".into(),
            },
            ("navigating", "route_blocked") => "Route blocked · will ask the model again".into(),
            ("navigating", _) => "Driving the model's waypoints".into(),
            ("waiting_operator", _) => "The model asked you a question".into(),
            ("completed", _) => "Done".into(),
            ("failed", reason) => format!("Failed · {}", task_end_reason_label(reason)),
            ("cancelled", reason) => format!("Cancelled · {}", task_end_reason_label(reason)),
            (phase, _) => phase.to_string(),
        }
    }
    /// Label and value rows for the console.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = vec![
            ("State", self.state_label()),
            ("Task", self.instruction.clone()),
            (
                "Steps",
                format!("{} of {} model calls", self.step, self.max_steps),
            ),
            (
                "Time",
                format!(
                    "{:.0} of {:.0} s · {:.0} s left",
                    self.elapsed_s, self.budget_s, self.remaining_s
                ),
            ),
        ];
        if let Some(model) = &self.model {
            let latency = self
                .latency_s
                .map(|s| format!(" · last answer {s:.1} s"))
                .unwrap_or_default();
            rows.push(("Model", format!("{model}{latency}")));
        }
        if !self.plan.is_empty() {
            rows.push((
                "Plan",
                self.plan
                    .iter()
                    .map(|w| format!("{} ({:.1}, {:.1})", w.label, w.x, w.y))
                    .collect::<Vec<_>>()
                    .join(" → "),
            ));
        }
        if let Some(rationale) = &self.rationale {
            rows.push(("Reason", rationale.clone()));
        }
        if let Some(error) = &self.last_error {
            rows.push(("Last error", error.clone()));
        }
        rows
    }
}

pub fn task_end_reason_label(reason: &str) -> &'static str {
    match reason {
        "model_done" => "the model finished the task",
        "model_gave_up" => "the model gave up",
        "model_failures" => "three bad or missing model answers in a row",
        "budget_exhausted" => "time budget used up",
        "step_limit" => "model call limit reached",
        "operator_cancel" => "cancelled here",
        "operator_stop" => "stop",
        "level_changed" => "another level was selected",
        "operator_reselection" => "L5 was selected again",
        "replaced" => "a new task replaced it",
        _ => "ended",
    }
}

/// Decode `autonomy/status.task`, or `None` before the first task.
pub fn decode_task(status: &Value) -> Option<TaskReport> {
    let value = status.get("task").filter(|v| v.is_object())?;
    let text = |key: &str| {
        value[key]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let number = |key: &str| value[key].as_f64().filter(|n| n.is_finite() && *n >= 0.);
    let plan = value["plan"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|w| {
                    let (x, y) = (w["x"].as_f64()?, w["y"].as_f64()?);
                    (x.is_finite() && y.is_finite()).then(|| TaskWaypoint {
                        x,
                        y,
                        label: w["label"].as_str().unwrap_or("point").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let history = value["history"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|h| {
                    Some((
                        h["kind"].as_str()?.to_string(),
                        h["text"].as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(TaskReport {
        task_id: text("task_id")?,
        instruction: text("instruction").unwrap_or_default(),
        phase: text("phase")?,
        reason: text("reason").unwrap_or_default(),
        step: value["step"].as_u64().unwrap_or(0),
        max_steps: value["max_steps"].as_u64().unwrap_or(0),
        elapsed_s: number("elapsed_seconds").unwrap_or(0.),
        budget_s: number("budget_seconds").unwrap_or(0.),
        remaining_s: number("remaining_seconds").unwrap_or(0.),
        model: text("model"),
        waiting_s: number("waiting_seconds"),
        latency_s: number("last_latency_seconds"),
        plan,
        question: text("question"),
        rationale: text("rationale"),
        last_error: text("last_error"),
        history,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_inline_task_request_terra_decodes() {
        let bytes =
            encode_task_level_request("tok-1", "task-1", "  Find the bucket. ", 300.).unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["level"], "task");
        assert_eq!(v["task"]["instruction"], "Find the bucket.");
        assert_eq!(v["task"]["budget_seconds"], 300.);
        assert!(encode_task_level_request("tok", "task", "   ", 300.).is_err());
        assert!(encode_task_level_request("tok", "task", "go", 0.).is_err());
        assert!(encode_task_level_request("tok", "task", &"x".repeat(501), 60.).is_err());
    }

    #[test]
    fn task_actions_are_checked_before_sending() {
        let reply = String::from_utf8(encode_task_reply("t", "left side", "r").unwrap()).unwrap();
        assert!(validate_task_action(&reply).is_ok());
        let cancel = String::from_utf8(encode_task_cancel("t", "c").unwrap()).unwrap();
        assert!(validate_task_action(&cancel).is_ok());
        assert!(validate_task_action(r#"{"task_id":"t","action":"reply","token":"r"}"#).is_err());
        assert!(
            validate_task_action(r#"{"task_id":"t","action":"cancel","token":"r","linear":1}"#)
                .is_err()
        );
    }

    #[test]
    fn decodes_terra_task_status() {
        let status = json!({"task": {
            "run_id": "r", "task_id": "t1", "instruction": "find it", "phase": "waiting_operator",
            "reason": "awaiting_operator", "step": 2, "max_steps": 20, "elapsed_seconds": 12.0,
            "budget_seconds": 300.0, "remaining_seconds": 288.0, "model": "apple-foundation-models",
            "query_id": null, "waiting_seconds": null, "last_latency_seconds": 3.2,
            "plan": [{"x": 1.0, "y": 2.0, "label": "F1"}], "question": "Which wall?",
            "rationale": "two walls", "last_error": null,
            "history": [{"step": 1, "kind": "arrived", "text": "reached F1"}]
        }});
        let report = decode_task(&status).unwrap();
        assert!(report.running() && report.waiting_for_operator());
        assert_eq!(report.question.as_deref(), Some("Which wall?"));
        assert_eq!(report.plan[0].label, "F1");
        assert_eq!(report.history[0].1, "reached F1");
        assert!(
            report
                .rows()
                .iter()
                .any(|(k, v)| *k == "Model" && v.contains("3.2 s"))
        );
        assert!(decode_task(&json!({})).is_none());
        let failed = decode_task(
            &json!({"task": {"task_id": "t", "phase": "failed", "reason": "step_limit"}}),
        )
        .unwrap();
        assert_eq!(failed.state_label(), "Failed · model call limit reached");
    }
}
