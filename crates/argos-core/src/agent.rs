//! Agents at the operator station.
//!
//! An MCP server (`tools/argos-mcp`) talks to the dashboard window over a local
//! bridge, one JSON request per line. The operator picks how much the agent may
//! do: **Off**, **Advise** (the default: read the field and propose
//! recommendations the operator accepts or rejects) or **Operator** (also
//! command rovers through the same checks as the window's own buttons). Raw
//! drive always waits for the operator to approve a time-limited grant.
//!
//! This module holds the policy and the wire format. The window owns the
//! socket, the buttons, and dispatch.
use crate::{
    EXPLORE_BUDGETS_MIN, OPERATOR_LEVELS, TASK_BUDGETS_MIN, TaskAgent, task_text, valid_token,
};
use serde_json::{Value, json};

/// Bumped when a request or reply changes shape.
pub const AGENT_PROTOCOL: u64 = 1;
/// Longest request line the bridge reads.
pub const AGENT_MAX_REQUEST_BYTES: usize = 64 * 1024;
/// Cards on screen at once.
pub const MAX_OPEN_RECOMMENDATIONS: usize = 3;
/// How long a card stays up unless the agent asks otherwise.
pub const DEFAULT_RECOMMENDATION_TTL_S: f64 = 90.;
pub const MIN_RECOMMENDATION_TTL_S: f64 = 15.;
pub const MAX_RECOMMENDATION_TTL_S: f64 = 300.;
/// After a rejection, the same kind of idea for the same rover is refused this long.
pub const REJECT_COOLDOWN_S: f64 = 180.;
/// Longest raw-drive grant the agent may ask for.
pub const DRIVE_GRANT_MAX_S: f64 = 60.;
/// Longest single drive call inside a grant.
pub const DRIVE_BURST_MAX_S: f64 = 2.;
/// An unanswered drive request lapses after this long.
pub const DRIVE_REQUEST_TIMEOUT_S: f64 = 60.;
/// Reasons the operator can tap when rejecting a card.
pub const REJECT_REASONS: &[(&str, &str)] = &[
    ("wrong_rover", "Wrong rover"),
    ("bad_timing", "Bad timing"),
    ("unsafe", "Unsafe"),
    ("already_doing", "Already doing it"),
    ("other", "Other"),
];
const REASON_MAX_CHARS: usize = 200;

/// How much the operator lets an agent do from this window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentMode {
    /// The bridge answers only `hello`.
    Off,
    /// Read the field and propose recommendations. Nothing moves without a click.
    #[default]
    Advise,
    /// Also command rovers, through the same checks as the buttons.
    Operator,
}

impl AgentMode {
    pub const ALL: [AgentMode; 3] = [AgentMode::Off, AgentMode::Advise, AgentMode::Operator];
    pub fn wire(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Advise => "advise",
            Self::Operator => "operator",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Advise => "Advise",
            Self::Operator => "Operator",
        }
    }
}

/// Where a command came from, carried in its token so the session log and
/// Terra's receipts can tell them apart. Every form keeps the `gpui-` prefix
/// the window's receipt check looks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandSource {
    /// A button, key or drag in the window.
    Operator,
    /// An agent in Operator mode.
    Agent,
    /// The operator accepted an agent's recommendation.
    Recommendation,
}

pub fn command_token(source: CommandSource, id: u64, millis: u128) -> String {
    match source {
        CommandSource::Operator => format!("gpui-{id}-{millis}"),
        CommandSource::Agent => format!("gpui-agent-{id}-{millis}"),
        CommandSource::Recommendation => format!("gpui-rec-{id}-{millis}"),
    }
}

/// `operator`, `agent` or `recommendation`, read back from a token.
pub fn token_source(token: &str) -> &'static str {
    if token.starts_with("gpui-agent-") {
        "agent"
    } else if token.starts_with("gpui-rec-") {
        "recommendation"
    } else {
        "operator"
    }
}

/// A goal from an agent or a recommendation.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentGoal {
    /// The rover's local map frame, metres.
    Local {
        x: f64,
        y: f64,
    },
    Geographic {
        latitude: f64,
        longitude: f64,
    },
}

/// Something an agent in Operator mode asks the window to do.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentCommand {
    SetLevel {
        rover: u64,
        level: String,
        budget_min: Option<u32>,
    },
    GoTo {
        rover: u64,
        goal: AgentGoal,
    },
    CancelGoal {
        rover: u64,
    },
    StartTask {
        rover: u64,
        instruction: String,
        budget_min: u32,
        agent: Option<TaskAgent>,
    },
    ReplyTask {
        rover: u64,
        text: String,
    },
    CancelTask {
        rover: u64,
    },
    Stop {
        rover: u64,
    },
    StopAll,
    RequestDrive {
        rover: u64,
        seconds: f64,
        reason: String,
    },
    Drive {
        rover: u64,
        linear: f64,
        angular: f64,
        seconds: f64,
    },
    EndDrive {
        rover: u64,
    },
}

impl AgentCommand {
    pub fn op(&self) -> &'static str {
        match self {
            Self::SetLevel { .. } => "set_level",
            Self::GoTo { .. } => "go_to",
            Self::CancelGoal { .. } => "cancel_goal",
            Self::StartTask { .. } => "start_task",
            Self::ReplyTask { .. } => "reply_task",
            Self::CancelTask { .. } => "cancel_task",
            Self::Stop { .. } => "stop",
            Self::StopAll => "stop_all",
            Self::RequestDrive { .. } => "request_drive",
            Self::Drive { .. } => "drive",
            Self::EndDrive { .. } => "end_drive",
        }
    }
    pub fn rover(&self) -> Option<u64> {
        match self {
            Self::SetLevel { rover, .. }
            | Self::GoTo { rover, .. }
            | Self::CancelGoal { rover }
            | Self::StartTask { rover, .. }
            | Self::ReplyTask { rover, .. }
            | Self::CancelTask { rover }
            | Self::Stop { rover }
            | Self::RequestDrive { rover, .. }
            | Self::Drive { rover, .. }
            | Self::EndDrive { rover } => Some(*rover),
            Self::StopAll => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentRequest {
    /// Names the engine and returns the mode and fleet.
    Hello {
        engine: String,
    },
    /// The whole field as the window sees it.
    State,
    /// The raw operator telemetry the window caches for one rover.
    Rover {
        rover: u64,
    },
    /// One or more recommendation bodies; each is checked on its own.
    Propose(Vec<Value>),
    /// Open cards and the latest decisions.
    Recommendations,
    Withdraw {
        rec_id: String,
    },
    Command(AgentCommand),
}

impl AgentRequest {
    pub fn op(&self) -> &'static str {
        match self {
            Self::Hello { .. } => "hello",
            Self::State => "state",
            Self::Rover { .. } => "rover",
            Self::Propose(_) => "propose",
            Self::Recommendations => "recommendations",
            Self::Withdraw { .. } => "withdraw",
            Self::Command(command) => command.op(),
        }
    }
}

/// One request line: the caller's `id` (echoed back), its token, and the request.
#[derive(Clone, Debug)]
pub struct AgentEnvelope {
    pub id: Value,
    pub token: String,
    pub request: Result<AgentRequest, String>,
}

pub fn parse_agent_line(line: &[u8]) -> AgentEnvelope {
    let value: Value = match serde_json::from_slice(line) {
        Ok(value @ Value::Object(_)) if line.len() <= AGENT_MAX_REQUEST_BYTES => value,
        _ => {
            return AgentEnvelope {
                id: Value::Null,
                token: String::new(),
                request: Err("Each line must be one JSON object under 64 KB".into()),
            };
        }
    };
    AgentEnvelope {
        id: value.get("id").cloned().unwrap_or(Value::Null),
        token: value["token"].as_str().unwrap_or_default().to_string(),
        request: parse_request(&value),
    }
}

fn rover(value: &Value) -> Result<u64, String> {
    value["rover"]
        .as_u64()
        .ok_or_else(|| "`rover` must be a rover id".to_string())
}

fn text(value: &Value, key: &str, max: usize) -> Result<String, String> {
    let text = value[key]
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("`{key}` is required"))?;
    if text.chars().count() > max {
        return Err(format!("`{key}` is longer than {max} characters"));
    }
    Ok(text.to_string())
}

fn number(value: &Value, key: &str) -> Result<f64, String> {
    value[key]
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("`{key}` must be a number"))
}

fn minutes(value: &Value, key: &str) -> Result<Option<u32>, String> {
    match &value[key] {
        Value::Null => Ok(None),
        v => v
            .as_u64()
            .and_then(|m| u32::try_from(m).ok())
            .map(Some)
            .ok_or_else(|| format!("`{key}` must be whole minutes")),
    }
}

/// A goal from `x`/`y` or `latitude`/`longitude`, exactly one pair.
pub fn parse_goal(value: &Value) -> Result<AgentGoal, String> {
    let local = value.get("x").is_some() || value.get("y").is_some();
    let geographic = value.get("latitude").is_some() || value.get("longitude").is_some();
    match (local, geographic) {
        (true, false) => Ok(AgentGoal::Local {
            x: number(value, "x")?,
            y: number(value, "y")?,
        }),
        (false, true) => {
            let latitude = number(value, "latitude")?;
            let longitude = number(value, "longitude")?;
            if latitude.abs() > 90. || longitude.abs() > 180. {
                return Err("latitude or longitude out of range".into());
            }
            Ok(AgentGoal::Geographic {
                latitude,
                longitude,
            })
        }
        _ => Err("Give either `x` and `y` (local metres) or `latitude` and `longitude`".into()),
    }
}

/// A level and, for L4 and L5, a budget the window's own picker offers.
pub fn check_level(level: &str, budget_min: Option<u32>) -> Result<(), String> {
    if !OPERATOR_LEVELS.iter().any(|(wire, _)| *wire == level) {
        let names: Vec<_> = OPERATOR_LEVELS.iter().map(|(wire, _)| *wire).collect();
        return Err(format!(
            "Unknown level `{level}`; use one of {}",
            names.join(", ")
        ));
    }
    match level {
        "explore" => check_budget(budget_min, EXPLORE_BUDGETS_MIN, "L4 Explore"),
        "task" => Err("Start L5 with start_task, which carries the task".into()),
        _ => Ok(()),
    }
}

fn check_budget(budget_min: Option<u32>, choices: &[u32], what: &str) -> Result<(), String> {
    match budget_min {
        Some(minutes) if choices.contains(&minutes) => Ok(()),
        _ => Err(format!(
            "{what} needs `budget_min`, one of {:?} like the operator's picker",
            choices
        )),
    }
}

fn parse_task_agent(value: &Value) -> Result<Option<TaskAgent>, String> {
    match value["agent"].as_str() {
        None => Ok(None),
        Some("builtin") => Ok(Some(TaskAgent::Builtin)),
        Some("mcp") => Ok(Some(TaskAgent::Mcp)),
        Some(_) => Err("`agent` is `builtin` or `mcp`".into()),
    }
}

fn parse_request(value: &Value) -> Result<AgentRequest, String> {
    let op = value["op"].as_str().ok_or("`op` is required")?;
    let command = |command| Ok(AgentRequest::Command(command));
    match op {
        "hello" => Ok(AgentRequest::Hello {
            engine: value["engine"]
                .as_str()
                .map(|e| e.chars().take(64).collect())
                .unwrap_or_else(|| "agent".into()),
        }),
        "state" => Ok(AgentRequest::State),
        "rover" => Ok(AgentRequest::Rover {
            rover: rover(value)?,
        }),
        "propose" => match &value["recommendations"] {
            Value::Array(items) if !items.is_empty() && items.len() <= 8 => {
                Ok(AgentRequest::Propose(items.clone()))
            }
            _ => Err("`recommendations` must be a list of 1 to 8 items".into()),
        },
        "recommendations" => Ok(AgentRequest::Recommendations),
        "withdraw" => Ok(AgentRequest::Withdraw {
            rec_id: text(value, "rec_id", 64)?,
        }),
        "set_level" => {
            let level = text(value, "level", 32)?;
            let budget_min = minutes(value, "budget_min")?;
            check_level(&level, budget_min)?;
            command(AgentCommand::SetLevel {
                rover: rover(value)?,
                level,
                budget_min,
            })
        }
        "go_to" => command(AgentCommand::GoTo {
            rover: rover(value)?,
            goal: parse_goal(value)?,
        }),
        "cancel_goal" => command(AgentCommand::CancelGoal {
            rover: rover(value)?,
        }),
        "start_task" => {
            let budget = minutes(value, "budget_min")?;
            check_budget(budget, TASK_BUDGETS_MIN, "An L5 task")?;
            let instruction =
                task_text(value["instruction"].as_str().unwrap_or_default()).map_err(|e| e.0)?;
            command(AgentCommand::StartTask {
                rover: rover(value)?,
                instruction,
                budget_min: budget.unwrap_or_default(),
                agent: parse_task_agent(value)?,
            })
        }
        "reply_task" => command(AgentCommand::ReplyTask {
            rover: rover(value)?,
            text: task_text(value["text"].as_str().unwrap_or_default()).map_err(|e| e.0)?,
        }),
        "cancel_task" => command(AgentCommand::CancelTask {
            rover: rover(value)?,
        }),
        "stop" => command(AgentCommand::Stop {
            rover: rover(value)?,
        }),
        "stop_all" => command(AgentCommand::StopAll),
        "request_drive" => {
            let seconds = number(value, "seconds")?;
            if !(seconds > 0. && seconds <= DRIVE_GRANT_MAX_S) {
                return Err(format!(
                    "`seconds` must be more than 0 and at most {DRIVE_GRANT_MAX_S:.0}"
                ));
            }
            command(AgentCommand::RequestDrive {
                rover: rover(value)?,
                seconds,
                reason: text(value, "reason", REASON_MAX_CHARS)?,
            })
        }
        "drive" => {
            let linear = number(value, "linear")?;
            let angular = number(value, "angular")?;
            drive_keys(linear, angular)?;
            let seconds = number(value, "seconds")?;
            if !(seconds > 0. && seconds <= DRIVE_BURST_MAX_S) {
                return Err(format!(
                    "`seconds` must be more than 0 and at most {DRIVE_BURST_MAX_S:.0}"
                ));
            }
            command(AgentCommand::Drive {
                rover: rover(value)?,
                linear,
                angular,
                seconds,
            })
        }
        "end_drive" => command(AgentCommand::EndDrive {
            rover: rover(value)?,
        }),
        other => Err(format!("Unknown op `{other}`")),
    }
}

/// Whether `mode` allows `request`. `Err` is the reason given to the agent.
pub fn agent_permitted(mode: AgentMode, request: &AgentRequest) -> Result<(), &'static str> {
    match (mode, request) {
        (_, AgentRequest::Hello { .. }) => Ok(()),
        (AgentMode::Off, _) => {
            Err("Agent access is Off in ARGOS; the operator can switch it to Advise")
        }
        (AgentMode::Advise, AgentRequest::Command(_)) => Err(
            "ARGOS is in Advise mode: propose a recommendation instead; the operator can switch to Operator",
        ),
        _ => Ok(()),
    }
}

/// Compare tokens without stopping at the first difference.
pub fn token_matches(given: &str, expected: &str) -> bool {
    let (a, b) = (given.as_bytes(), expected.as_bytes());
    !b.is_empty() && a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

pub fn agent_reply(id: &Value, result: Result<Value, String>) -> Value {
    match result {
        Ok(mut body) => {
            if !body.is_object() {
                body = json!({ "result": body });
            }
            body["ok"] = true.into();
            body["id"] = id.clone();
            body
        }
        Err(error) => json!({ "ok": false, "id": id, "error": error }),
    }
}

/// W/A/S/D keys for a drive direction. Linear and angular give the direction
/// only; the operator's drive limits set the speed, exactly as for keyboard
/// drive. Values under 0.1 in size count as zero.
pub fn drive_keys(linear: f64, angular: f64) -> Result<Vec<&'static str>, String> {
    if !linear.is_finite() || !angular.is_finite() || linear.abs() > 1. || angular.abs() > 1. {
        return Err("`linear` and `angular` must be between -1 and 1".into());
    }
    let mut keys = Vec::new();
    if linear >= 0.1 {
        keys.push("w");
    } else if linear <= -0.1 {
        keys.push("s");
    }
    if angular >= 0.1 {
        keys.push("a");
    } else if angular <= -0.1 {
        keys.push("d");
    }
    if keys.is_empty() {
        return Err("Both directions are zero; call end_drive to stop".into());
    }
    Ok(keys)
}

/// A raw-drive grant the agent asked for. Nothing moves until the operator approves.
#[derive(Clone, Debug, PartialEq)]
pub struct DriveGrant {
    pub rover: u64,
    pub seconds: f64,
    pub reason: String,
    pub requested_at: f64,
    pub approved_at: Option<f64>,
}

impl DriveGrant {
    pub fn request(rover: u64, seconds: f64, reason: String, now: f64) -> Self {
        Self {
            rover,
            seconds: seconds.clamp(0., DRIVE_GRANT_MAX_S),
            reason,
            requested_at: now,
            approved_at: None,
        }
    }
    pub fn approve(&mut self, now: f64) {
        self.approved_at.get_or_insert(now);
    }
    pub fn pending(&self) -> bool {
        self.approved_at.is_none()
    }
    pub fn active(&self, now: f64) -> bool {
        self.remaining(now) > 0.
    }
    /// Seconds left on an approved grant; 0 while pending.
    pub fn remaining(&self, now: f64) -> f64 {
        self.approved_at
            .map(|at| (at + self.seconds - now).max(0.))
            .unwrap_or(0.)
    }
    /// Approved and used up, or never answered in time.
    pub fn lapsed(&self, now: f64) -> bool {
        match self.approved_at {
            Some(_) => !self.active(now),
            None => now - self.requested_at >= DRIVE_REQUEST_TIMEOUT_S,
        }
    }
    pub fn to_json(&self, now: f64) -> Value {
        json!({
            "rover": self.rover,
            "seconds": self.seconds,
            "reason": self.reason,
            "state": if self.pending() { "waiting_for_operator" } else if self.active(now) { "granted" } else { "ended" },
            "remaining_s": self.remaining(now),
        })
    }
}

/// The point `approach_m` short of `target` on the line from `from`.
pub fn approach_point(
    from: (f64, f64),
    target: (f64, f64),
    approach_m: f64,
) -> Result<(f64, f64), String> {
    let (dx, dy) = (target.0 - from.0, target.1 - from.1);
    let distance = dx.hypot(dy);
    if !distance.is_finite() {
        return Err("No position for the rover or the object".into());
    }
    if distance <= approach_m + 0.25 {
        return Err(format!(
            "The rover is already within {approach_m:.1} m of it; hand it to the operator"
        ));
    }
    let scale = (distance - approach_m) / distance;
    Ok((from.0 + dx * scale, from.1 + dy * scale))
}

/// What accepting a card does.
#[derive(Clone, Debug, PartialEq)]
pub enum RecAction {
    SetLevel {
        level: String,
        budget_min: Option<u32>,
    },
    GoTo(AgentGoal),
    Task {
        instruction: String,
        budget_min: u32,
    },
    /// Drive to `approach_m` short of a found bucket, then hand over. Never dumps.
    HandoffBucket {
        object_id: u32,
        approach_m: f64,
    },
    /// A flag for the operator; accepting only acknowledges it.
    Attention,
}

impl RecAction {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::SetLevel { .. } => "set_level",
            Self::GoTo(_) => "go_to",
            Self::Task { .. } => "task",
            Self::HandoffBucket { .. } => "handoff_bucket",
            Self::Attention => "attention",
        }
    }
    /// Short card title, such as "L4 Explore · 5 min".
    pub fn title(&self) -> String {
        match self {
            Self::SetLevel { level, budget_min } => {
                let label = crate::level_label(level);
                match budget_min {
                    Some(minutes) => format!("{label} · {minutes} min"),
                    None => label.to_string(),
                }
            }
            Self::GoTo(AgentGoal::Local { x, y }) => format!("Go to {x:.1}, {y:.1} m"),
            Self::GoTo(AgentGoal::Geographic {
                latitude,
                longitude,
            }) => format!("Go to {latitude:.6}, {longitude:.6}"),
            Self::Task {
                instruction,
                budget_min,
            } => {
                let short: String = instruction.chars().take(40).collect();
                format!("L5 task · {budget_min} min · {short}")
            }
            Self::HandoffBucket { object_id, .. } => {
                format!("Drive up to bucket #{object_id}, then hand over")
            }
            Self::Attention => "Needs you".into(),
        }
    }
    fn to_json(&self) -> Value {
        match self {
            Self::SetLevel { level, budget_min } => {
                json!({"level": level, "budget_min": budget_min})
            }
            Self::GoTo(AgentGoal::Local { x, y }) => json!({"x": x, "y": y}),
            Self::GoTo(AgentGoal::Geographic {
                latitude,
                longitude,
            }) => json!({"latitude": latitude, "longitude": longitude}),
            Self::Task {
                instruction,
                budget_min,
            } => json!({"instruction": instruction, "budget_min": budget_min}),
            Self::HandoffBucket {
                object_id,
                approach_m,
            } => json!({"object_id": object_id, "approach_m": approach_m}),
            Self::Attention => json!({}),
        }
    }
}

/// A recommendation as the agent sent it, after checking.
#[derive(Clone, Debug, PartialEq)]
pub struct Recommendation {
    /// Empty until the book assigns one.
    pub rec_id: String,
    pub rover: u64,
    pub action: RecAction,
    /// 1 FYI, 2 soon, 3 now.
    pub urgency: u8,
    pub reason: String,
    pub evidence: Vec<String>,
    pub confidence: Option<f64>,
    pub engine: String,
    pub ttl_s: f64,
}

/// Check one `propose` item. `action` holds the kind's fields.
pub fn decode_recommendation(value: &Value, engine: &str) -> Result<Recommendation, String> {
    let rec_id = match value["rec_id"].as_str() {
        None => String::new(),
        Some(id) if valid_token(id) => id.to_string(),
        Some(_) => return Err("`rec_id` must be 1-64 letters, digits or ._:-".into()),
    };
    let action = &value["action"];
    let action = match value["kind"].as_str().ok_or("`kind` is required")? {
        "set_level" => {
            let level = text(action, "level", 32)?;
            let budget_min = minutes(action, "budget_min")?;
            if level == "task" {
                return Err("Recommend an L5 task with kind `task`".into());
            }
            check_level(&level, budget_min)?;
            RecAction::SetLevel { level, budget_min }
        }
        "go_to" => RecAction::GoTo(parse_goal(action)?),
        "task" => {
            let budget = minutes(action, "budget_min")?;
            check_budget(budget, TASK_BUDGETS_MIN, "An L5 task")?;
            RecAction::Task {
                instruction: task_text(action["instruction"].as_str().unwrap_or_default())
                    .map_err(|e| e.0)?,
                budget_min: budget.unwrap_or_default(),
            }
        }
        "handoff_bucket" => RecAction::HandoffBucket {
            object_id: action["object_id"]
                .as_u64()
                .and_then(|id| u32::try_from(id).ok())
                .ok_or("`action.object_id` must be a found object's id")?,
            approach_m: action["approach_m"].as_f64().unwrap_or(1.5).clamp(0.5, 5.),
        },
        "attention" => RecAction::Attention,
        other => {
            return Err(format!(
                "Unknown kind `{other}`; use set_level, go_to, task, handoff_bucket or attention"
            ));
        }
    };
    let urgency = match &value["urgency"] {
        Value::Null => 1,
        v => v
            .as_u64()
            .filter(|u| (1..=3).contains(u))
            .ok_or("`urgency` is 1 (FYI), 2 (soon) or 3 (now)")? as u8,
    };
    let evidence = match &value["evidence"] {
        Value::Null => Vec::new(),
        Value::Array(items) if items.len() <= 8 => items
            .iter()
            .map(|item| {
                item.as_str()
                    .filter(|s| !s.is_empty() && s.chars().count() <= 80)
                    .map(str::to_string)
                    .ok_or("each `evidence` entry is a short string")
            })
            .collect::<Result<_, _>>()?,
        _ => return Err("`evidence` is a list of up to 8 strings".into()),
    };
    let confidence = match &value["confidence"] {
        Value::Null => None,
        v => Some(
            v.as_f64()
                .filter(|c| (0. ..=1.).contains(c))
                .ok_or("`confidence` is between 0 and 1")?,
        ),
    };
    Ok(Recommendation {
        rec_id,
        rover: rover(value)?,
        action,
        urgency,
        reason: text(value, "reason", REASON_MAX_CHARS)?,
        evidence,
        confidence,
        engine: engine.chars().take(64).collect(),
        ttl_s: value["expires_s"]
            .as_f64()
            .filter(|s| s.is_finite())
            .unwrap_or(DEFAULT_RECOMMENDATION_TTL_S)
            .clamp(MIN_RECOMMENDATION_TTL_S, MAX_RECOMMENDATION_TTL_S),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecState {
    Open,
    Accepted,
    Rejected,
    /// Nobody answered before it lapsed.
    Expired,
    Withdrawn,
    /// Replaced by a more urgent card, or no longer fits the rover.
    Superseded,
}

impl RecState {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
            Self::Withdrawn => "withdrawn",
            Self::Superseded => "superseded",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecEntry {
    pub rec: Recommendation,
    pub shown_at: f64,
    pub state: RecState,
    pub decided_at: Option<f64>,
    /// Reject reason, or why it was superseded.
    pub reason: Option<String>,
    /// What accepting it did: the window's note, or why it was refused.
    pub dispatch: Option<Result<String, String>>,
}

impl RecEntry {
    pub fn remaining(&self, now: f64) -> f64 {
        (self.shown_at + self.rec.ttl_s - now).max(0.)
    }
    pub fn to_json(&self, now: f64) -> Value {
        json!({
            "rec_id": self.rec.rec_id,
            "rover": self.rec.rover,
            "kind": self.rec.action.kind(),
            "action": self.rec.action.to_json(),
            "urgency": self.rec.urgency,
            "reason": self.rec.reason,
            "evidence": self.rec.evidence,
            "confidence": self.rec.confidence,
            "engine": self.rec.engine,
            "state": self.state.wire(),
            "age_s": now - self.shown_at,
            "remaining_s": if self.state == RecState::Open { Some(self.remaining(now)) } else { None },
            "decision_reason": self.reason,
            "decision_latency_s": self.decided_at.map(|at| at - self.shown_at),
            "dispatch": match &self.dispatch {
                Some(Ok(note)) => json!({"ok": true, "note": note}),
                Some(Err(error)) => json!({"ok": false, "note": error}),
                None => Value::Null,
            },
        })
    }
}

/// Recommendation cards: at most [`MAX_OPEN_RECOMMENDATIONS`] open, plus a
/// short history so the agent can see what the operator decided.
#[derive(Clone, Debug, Default)]
pub struct RecBook {
    entries: Vec<RecEntry>,
    next: u64,
}

const REC_HISTORY: usize = 50;

fn same_idea(a: &Recommendation, b: &Recommendation) -> bool {
    a.rover == b.rover && a.action == b.action
}

impl RecBook {
    /// Put a card up. A duplicate of an open card refreshes it and keeps its id.
    /// Returns the id and whether a card was superseded to make room.
    pub fn propose(
        &mut self,
        mut rec: Recommendation,
        now: f64,
    ) -> Result<(String, Option<String>), String> {
        if let Some(open) = self
            .entries
            .iter_mut()
            .find(|e| e.state == RecState::Open && same_idea(&e.rec, &rec))
        {
            open.rec.reason = rec.reason;
            open.rec.urgency = rec.urgency;
            open.rec.evidence = rec.evidence;
            open.rec.confidence = rec.confidence;
            open.rec.ttl_s = (now - open.shown_at) + rec.ttl_s;
            return Ok((open.rec.rec_id.clone(), None));
        }
        if !rec.rec_id.is_empty() && self.entries.iter().any(|e| e.rec.rec_id == rec.rec_id) {
            return Err(format!("rec_id `{}` was already used", rec.rec_id));
        }
        if let Some(rejected) = self.entries.iter().rev().find(|e| {
            e.state == RecState::Rejected
                && e.rec.rover == rec.rover
                && e.rec.action.kind() == rec.action.kind()
                && e.decided_at.is_some_and(|at| now - at < REJECT_COOLDOWN_S)
        }) {
            return Err(format!(
                "The operator rejected {} for rover {} {:.0} s ago ({}); wait {:.0} s before proposing it again",
                rec.action.kind(),
                rec.rover,
                now - rejected.decided_at.unwrap_or(now),
                rejected.reason.as_deref().unwrap_or("no reason"),
                REJECT_COOLDOWN_S - (now - rejected.decided_at.unwrap_or(now)),
            ));
        }
        let mut superseded = None;
        if self.open().len() >= MAX_OPEN_RECOMMENDATIONS {
            let weakest = self
                .entries
                .iter_mut()
                .filter(|e| e.state == RecState::Open)
                .min_by(|a, b| {
                    a.rec
                        .urgency
                        .cmp(&b.rec.urgency)
                        .then(a.shown_at.total_cmp(&b.shown_at))
                })
                .filter(|weakest| weakest.rec.urgency < rec.urgency)
                .ok_or(
                    "Three recommendations are already open and none is less urgent; withdraw one or wait",
                )?;
            weakest.state = RecState::Superseded;
            weakest.decided_at = Some(now);
            weakest.reason = Some("a more urgent card replaced it".into());
            superseded = Some(weakest.rec.rec_id.clone());
        }
        if rec.rec_id.is_empty() {
            self.next += 1;
            rec.rec_id = format!("rec-{}", self.next);
        }
        let id = rec.rec_id.clone();
        self.entries.push(RecEntry {
            rec,
            shown_at: now,
            state: RecState::Open,
            decided_at: None,
            reason: None,
            dispatch: None,
        });
        self.trim();
        Ok((id, superseded))
    }
    fn trim(&mut self) {
        while self.entries.len() > REC_HISTORY + MAX_OPEN_RECOMMENDATIONS {
            match self.entries.iter().position(|e| e.state != RecState::Open) {
                Some(index) => {
                    self.entries.remove(index);
                }
                None => break,
            }
        }
    }
    /// Open cards, most urgent first, then oldest first.
    pub fn open(&self) -> Vec<&RecEntry> {
        let mut open: Vec<_> = self
            .entries
            .iter()
            .filter(|e| e.state == RecState::Open)
            .collect();
        open.sort_by(|a, b| {
            b.rec
                .urgency
                .cmp(&a.rec.urgency)
                .then(a.shown_at.total_cmp(&b.shown_at))
        });
        open
    }
    /// Decided cards, newest decision first.
    pub fn recent(&self, count: usize) -> Vec<&RecEntry> {
        let mut done: Vec<_> = self
            .entries
            .iter()
            .filter(|e| e.state != RecState::Open)
            .collect();
        done.sort_by(|a, b| {
            b.decided_at
                .unwrap_or_default()
                .total_cmp(&a.decided_at.unwrap_or_default())
        });
        done.truncate(count);
        done
    }
    pub fn get(&self, rec_id: &str) -> Option<&RecEntry> {
        self.entries.iter().find(|e| e.rec.rec_id == rec_id)
    }
    /// Close an open card. Only an open card can be decided.
    pub fn decide(
        &mut self,
        rec_id: &str,
        state: RecState,
        reason: Option<String>,
        now: f64,
    ) -> Result<&RecEntry, String> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.rec.rec_id == rec_id)
            .ok_or_else(|| format!("No recommendation `{rec_id}`"))?;
        if entry.state != RecState::Open {
            return Err(format!("`{rec_id}` is already {}", entry.state.wire()));
        }
        if state == RecState::Open {
            return Err("A card can only be closed".into());
        }
        entry.state = state;
        entry.decided_at = Some(now);
        entry.reason = reason;
        Ok(entry)
    }
    /// Record what accepting a card did.
    pub fn dispatched(&mut self, rec_id: &str, result: Result<String, String>) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.rec.rec_id == rec_id) {
            entry.dispatch = Some(result);
        }
    }
    /// Close lapsed cards; returns their ids.
    pub fn expire(&mut self, now: f64) -> Vec<String> {
        let mut lapsed = Vec::new();
        for entry in &mut self.entries {
            if entry.state == RecState::Open && entry.remaining(now) <= 0. {
                entry.state = RecState::Expired;
                entry.decided_at = Some(now);
                lapsed.push(entry.rec.rec_id.clone());
            }
        }
        lapsed
    }
    /// Close every open card for which `stale` gives a reason.
    pub fn supersede_where(
        &mut self,
        now: f64,
        mut stale: impl FnMut(&Recommendation) -> Option<String>,
    ) -> Vec<String> {
        let mut closed = Vec::new();
        for entry in &mut self.entries {
            if entry.state == RecState::Open
                && let Some(reason) = stale(&entry.rec)
            {
                entry.state = RecState::Superseded;
                entry.decided_at = Some(now);
                entry.reason = Some(reason);
                closed.push(entry.rec.rec_id.clone());
            }
        }
        closed
    }
}

/// Why an open card no longer fits the rover it is about, if it does not.
/// `requested` is the rover's requested level; `present` is false once the
/// rover has left the fleet.
pub fn recommendation_stale(
    rec: &Recommendation,
    present: bool,
    requested: Option<&str>,
) -> Option<String> {
    if !present {
        return Some("the rover left the fleet".into());
    }
    match &rec.action {
        RecAction::SetLevel { level, .. } if requested == Some(level.as_str()) => {
            Some("the rover is already at that level".into())
        }
        RecAction::Task { .. } if requested == Some("task") => {
            Some("the rover is already running a task".into())
        }
        _ => None,
    }
}
