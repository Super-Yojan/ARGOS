//! Agents in the window: bridge requests, recommendation cards, and the
//! raw-drive grant. Rules and wire format are in [`argos_core::agent`].
//!
//! Every command an agent sends, and every card the operator accepts, goes
//! through the same checks as the buttons (study lock, stop, Outdoor GPS,
//! L5 support, the goal gate, hardware) and the same `FleetFeed` calls. Tokens
//! mark the source, and each step is a session-log event.
use super::interaction::geographic_gate;
use super::levels::gps_blocks;
use super::task::task_agent_choice;
use super::widgets::{caution, hint};
use crate::Dashboard;
use argos_core::{
    AGENT_CONNECT_GRACE_S, AGENT_PROTOCOL, AgentCommand, AgentEnvelope, AgentGoal, AgentLink,
    AgentMode, AgentRequest, CommandSource, DragAnchor, DragEnd, DragGate, DragSpace, DriveGrant,
    EXPLORE_BUDGETS_MIN, GoalDrag, PlanarPoint, REJECT_REASONS, RecAction, RecBook, RecEntry,
    RecState, TASK_BUDGETS_MIN, agent_permitted, agent_status, approach_point,
    decode_recommendation, drive_block, drive_keys, explore_refusal, level_label, level_permitted,
    recommendation_stale, task_refusal,
};
use argos_core::{AGENT_EARLY_TICK_MIN_S, AGENT_RUN_MAX_S, AGENT_TICK_S, tick_reasons};
use argos_gpui::agent_mcp::McpProfile;
use argos_gpui::agent_runner::{AgentCli, AgentRun, TICK_MARK, tick_reply};
use argos_gpui::console::StopOrigin;
use argos_gpui::voice::{
    MAX_RECORD_S, MIN_RECORD_S, Recording, Transcriber, clip_seconds, download_model,
    missing_model, transcribe_clip,
};
use argos_gpui::{AGENT_PORT, AgentBridge, AgentProcess, RoverMark};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::Input;
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonRounded, ButtonVariants},
    menu::{DropdownMenu, PopupMenuItem},
    tag::Tag,
};
use gpui_kit::{prelude::*, *};
use serde_json::{Value, json};
use std::cell::Cell;
use std::sync::mpsc;

/// How long an accepted goal card waits for the rover to reach L3.
const LEVEL_WAIT_S: f64 = 5.;
/// How long an accepted goal is watched for arrival.
const OUTCOME_WAIT_S: f64 = 300.;
/// A spoken task longer than this is cut; a minute of speech fits.
const OPERATOR_TASK_MAX_CHARS: usize = 1_000;

/// An accepted card whose goal waits for the rover to switch to L3.
struct QueuedGoal {
    rec_id: String,
    rover: u64,
    goal: AgentGoal,
    deadline: f64,
}

/// An accepted goal card, watched until the rover arrives.
struct Watched {
    rec_id: String,
    rover: u64,
    handoff: Option<u32>,
    deadline: f64,
    seen_active: bool,
}

/// How often a picked engine looks at the field for early-tick reasons.
const FIELD_CHECK_S: f64 = 5.;

/// The coding-agent CLI the operator picked, run once per tick.
struct CliRunner {
    cli: AgentCli,
    log: std::path::PathBuf,
    run: Option<(AgentProcess, f64)>,
    last_tick: Option<f64>,
    last_check: f64,
    last_state: Option<Value>,
    ticks: u32,
    /// Why the last run failed, or why it could not start.
    error: Option<String>,
    /// The last line of the last good run: usually its answer.
    answer: Option<String>,
    /// The operator task the last run was given, so a new one runs at once.
    task_sent: Option<u32>,
}

impl CliRunner {
    fn new(cli: AgentCli) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        Self {
            cli,
            log: argos_gpui::session_dir().join(format!(
                "{}{}-{stamp}.log",
                argos_gpui::AGENT_LOG_PREFIX,
                cli.engine()
            )),
            run: None,
            last_tick: None,
            last_check: f64::NEG_INFINITY,
            last_state: None,
            ticks: 0,
            error: None,
            answer: None,
            task_sent: None,
        }
    }
}

/// What the operator asked the fleet to do. It stays in every tick's field
/// state until the operator clears or replaces it.
#[derive(Clone, Debug)]
pub(super) struct OperatorTask {
    id: u32,
    text: String,
    /// `voice` or `typed`.
    source: &'static str,
    at: f64,
}

/// Who said a line in the Agent panel's conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChatKind {
    /// The operator's task, `typed` or `voice`.
    Operator(&'static str),
    /// The engine's answer at the end of a run, by engine label.
    Agent(&'static str),
    /// Something failed: an engine run, a card dispatch.
    Problem,
    /// A quiet note: mode change, card accepted, task cleared.
    Event,
}

/// One line of the conversation. It is display only; every step is already
/// a session-log event.
#[derive(Clone, Debug)]
pub(super) struct ChatLine {
    kind: ChatKind,
    text: String,
    at: f64,
    /// Small line above an engine reply, like "Ran Claude · 14 s · tick 3".
    meta: Option<String>,
}

/// The conversation keeps this many lines; older ones drop off the top.
const CHAT_MAX_LINES: usize = 200;

/// Transcription result: the text and where the clip was saved.
type Heard = Result<(String, std::path::PathBuf), String>;

/// Talk button state: the microphone while recording, then the transcriber.
#[derive(Default)]
pub(super) struct VoiceDesk {
    recording: Option<Recording>,
    /// Transcription running off the UI thread, and when it started.
    pending: Option<(mpsc::Receiver<Heard>, f64)>,
    /// Last problem or result line for the panel, and whether it is a warning.
    note: Option<(String, bool)>,
    clips: u32,
    /// The Whisper model downloading on first Talk.
    download: Option<mpsc::Receiver<Result<(), String>>>,
}

/// What the window keeps for agents.
pub(crate) struct AgentDesk {
    pub(super) mode: AgentMode,
    bridge: Option<AgentBridge>,
    bridge_note: String,
    /// The agent ARGOS started from `ARGOS_AGENT_CMD`, if any.
    process: Option<AgentProcess>,
    /// The engine the operator picked (`ARGOS_AGENT_CLI` at start).
    runner: Option<CliRunner>,
    /// The operator let the engine command rovers this session. Off at
    /// start and whenever the mode leaves Operator or the engine changes.
    engine_commands: bool,
    /// When the bridge opened, on the window's clock.
    opened: f64,
    /// Whether the session log already says the agent is missing / stopped.
    missing_logged: bool,
    exit_logged: bool,
    engine: Option<String>,
    last_call: Option<f64>,
    pub(super) book: RecBook,
    grant: Option<DriveGrant>,
    /// The rover the agent is driving, the keys it holds, and when they release.
    burst: Option<(u64, Vec<&'static str>, f64)>,
    /// Card whose reject reasons are showing.
    rejecting: Option<String>,
    queued: Vec<QueuedGoal>,
    watched: Vec<Watched>,
    /// The operator's current task for the engine (spoken or typed).
    pub(super) task: Option<OperatorTask>,
    next_task: u32,
    pub(super) voice: VoiceDesk,
    /// The Agent panel's conversation, oldest first.
    chat: Vec<ChatLine>,
    chat_scroll: ScrollHandle,
    /// Rows drawn last frame, so the view follows new ones to the bottom.
    chat_rows: Cell<usize>,
}

impl AgentDesk {
    /// Open the bridge unless `ARGOS_AGENT=0`. `ARGOS_AGENT_PORT` picks the port.
    /// With the bridge up, `ARGOS_AGENT_CMD` names an agent command to start.
    pub(super) fn start() -> Self {
        let disabled = std::env::var("ARGOS_AGENT").ok().as_deref() == Some("0");
        let port = std::env::var("ARGOS_AGENT_PORT")
            .ok()
            .and_then(|port| port.parse().ok())
            .unwrap_or(AGENT_PORT);
        let (bridge, bridge_note) = if disabled {
            (None, "Agent bridge off (ARGOS_AGENT=0)".to_string())
        } else {
            match AgentBridge::start(&argos_gpui::session_dir(), port) {
                Ok(bridge) => {
                    let note = format!("Agent bridge on 127.0.0.1:{}", bridge.port());
                    (Some(bridge), note)
                }
                Err(error) => (None, format!("Agent bridge not started: {error}")),
            }
        };
        let mut bridge_note = bridge_note;
        let process = bridge.as_ref().and_then(|bridge| {
            let command = std::env::var("ARGOS_AGENT_CMD").ok()?;
            let command = command.trim();
            if command.is_empty() {
                return None;
            }
            match AgentProcess::spawn(command, bridge.file(), &argos_gpui::session_dir()) {
                Ok(process) => Some(process),
                Err(error) => {
                    bridge_note = format!("{bridge_note} · could not start `{command}`: {error}");
                    None
                }
            }
        });
        Self {
            mode: if disabled {
                AgentMode::Off
            } else {
                AgentMode::default()
            },
            bridge,
            bridge_note,
            process,
            runner: std::env::var("ARGOS_AGENT_CLI")
                .ok()
                .and_then(|name| AgentCli::parse(&name))
                .map(CliRunner::new),
            engine_commands: false,
            opened: 0.,
            missing_logged: false,
            exit_logged: false,
            engine: None,
            last_call: None,
            book: RecBook::default(),
            grant: None,
            burst: None,
            rejecting: None,
            queued: Vec::new(),
            watched: Vec::new(),
            task: None,
            next_task: 0,
            voice: VoiceDesk::default(),
            chat: Vec::new(),
            chat_scroll: ScrollHandle::new(),
            chat_rows: Cell::new(0),
        }
    }
}

impl AgentDesk {
    /// Add a line to the conversation. An engine answer or problem that
    /// repeats the line before it is skipped, so a quiet fleet does not fill
    /// the panel with the same report every minute.
    fn say(&mut self, kind: ChatKind, text: impl Into<String>, at: f64) {
        let text = text.into();
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let repeat = self
            .chat
            .iter()
            .rev()
            .find(|line| line.kind != ChatKind::Event);
        if matches!(kind, ChatKind::Agent(_) | ChatKind::Problem)
            && repeat.is_some_and(|line| line.kind == kind && line.text == text)
        {
            return;
        }
        self.chat.push(ChatLine {
            kind,
            text: text.to_string(),
            at,
            meta: None,
        });
        let extra = self.chat.len().saturating_sub(CHAT_MAX_LINES);
        self.chat.drain(..extra);
    }

    /// The panel's status line: who is connected, or what the operator should do.
    fn status(&self, now: f64) -> argos_core::AgentStatus {
        agent_status(&AgentLink {
            bridge: self.bridge.as_ref().map(|_| self.bridge_note.as_str()),
            bridge_error: &self.bridge_note,
            since_open: now - self.opened,
            engine: self.engine.as_deref(),
            since_call: self.last_call.map(|at| now - at),
            bad_tokens: self.bridge.as_ref().map_or(0, AgentBridge::bad_tokens),
            command: self
                .runner
                .as_ref()
                .map(|runner| runner.cli.label())
                .or_else(|| self.process.as_ref().map(AgentProcess::command)),
            command_exit: match &self.runner {
                Some(runner) => runner.error.as_deref(),
                None => self.process.as_ref().and_then(AgentProcess::exited),
            },
        })
    }
}

fn rover_json(dashboard: &Dashboard, mark: &RoverMark) -> Value {
    json!({
        "id": mark.id,
        "status": mark.status,
        "pose": {
            "x": mark.local_x, "y": mark.local_y, "yaw": mark.yaw,
            "latitude": mark.latitude, "longitude": mark.longitude, "heading_deg": mark.heading,
            "degraded": mark.degraded, "degrade_reason": mark.degrade_reason,
        },
        "level": {
            "requested": mark.requested_level,
            "effective": mark.effective_level,
            "study_condition": dashboard.study_lock(mark),
        },
        "safety": mark.safety,
        "stopped": dashboard.is_stopped(mark),
        "hold": mark.hold,
        "reason": mark.autonomy_reason,
        "armed": mark.hardware.output_active,
        "drive_block": drive_block(&mark.hardware),
        "navigation": if mark.outdoor_gps { "outdoor_gps" } else { "local" },
        "goal": mark.goal.as_ref().map(|goal| json!({
            "x": goal.x, "y": goal.y, "latitude": goal.latitude, "longitude": goal.longitude, "state": goal.state,
        })),
        "goal_active": mark.goal_active,
        "mission_active": mark.mission_active,
        "exploration": mark.exploration.as_ref().map(|run| json!({
            "phase": run.phase, "elapsed_s": run.elapsed_s, "remaining_s": run.remaining_s,
            "coverage_m2": run.coverage_m2, "explored_ratio": run.explored_ratio,
            "target": run.target, "end_reason": run.end_reason,
        })),
        "task": mark.task.as_ref().map(|task| json!({
            "task_id": task.task_id, "instruction": task.instruction, "phase": task.phase,
            "running": task.running(), "question": task.question, "step": task.step,
            "remaining_s": task.remaining_s, "agent": task.agent.as_str(),
        })),
        "task_supported": mark.task_supported,
        "task_agents": mark.task_agents.iter().map(|agent| agent.as_str()).collect::<Vec<_>>(),
        "objects": mark.objects.iter().map(|found| {
            let object = &found.object;
            json!({
                "id": object.id, "class": object.class, "color": object.color,
                "x": object.x, "y": object.y,
                "frame": "local",
                "gps": object.latitude.zip(object.longitude).map(|(latitude, longitude)| json!({
                    "latitude": latitude, "longitude": longitude, "accuracy_m": object.geo_accuracy_m,
                })),
                "confidence": object.confidence, "sightings": object.sightings,
            })
        }).collect::<Vec<_>>(),
    })
}

fn entries_json(entries: Vec<&RecEntry>, now: f64) -> Vec<Value> {
    entries
        .into_iter()
        .map(|entry| entry.to_json(now))
        .collect()
}

impl Dashboard {
    /// Called from the poll loop: housekeeping, then answer queued requests.
    pub(super) fn serve_agents(&mut self) {
        self.agent_upkeep();
        let calls = self
            .agent
            .bridge
            .as_ref()
            .map(AgentBridge::take_calls)
            .unwrap_or_default();
        for call in calls {
            let result = self.agent_call(&call.envelope);
            call.answer(result);
        }
    }

    fn agent_call(&mut self, envelope: &AgentEnvelope) -> Result<Value, String> {
        let request = envelope.request.clone()?;
        agent_permitted(self.agent.mode, &request).map_err(str::to_string)?;
        self.agent.last_call = Some(self.now);
        match request {
            AgentRequest::Hello { engine } => {
                self.feed.record(json!({"kind": "agent_hello", "engine": engine, "mode": self.agent.mode.wire()}));
                self.agent.engine = Some(engine);
                Ok(json!({
                    "protocol": AGENT_PROTOCOL,
                    "mode": self.agent.mode.wire(),
                    "fleet": self.fleet_mode.label(),
                    "explore_budgets_min": EXPLORE_BUDGETS_MIN,
                    "task_budgets_min": TASK_BUDGETS_MIN,
                }))
            }
            AgentRequest::State => Ok(self.field_state()),
            AgentRequest::Rover { rover } => {
                if !self.marks.iter().any(|mark| mark.id == rover) {
                    return Err(format!("Rover {rover} is not on the map"));
                }
                Ok(json!({"rover": rover, "telemetry": self.feed.rover_telemetry(rover)}))
            }
            AgentRequest::Propose(items) => Ok(self.propose(items)),
            AgentRequest::Recommendations => Ok(json!({
                "open": entries_json(self.agent.book.open(), self.now),
                "recent": entries_json(self.agent.book.recent(10), self.now),
            })),
            AgentRequest::Withdraw { rec_id } => {
                self.agent
                    .book
                    .decide(&rec_id, RecState::Withdrawn, None, self.now)?;
                self.log_decision(&rec_id);
                Ok(json!({"rec_id": rec_id, "state": "withdrawn"}))
            }
            AgentRequest::Command(command) => {
                let result = self.agent_command(&command);
                self.feed.record(json!({
                    "kind": "agent_command",
                    "op": command.op(),
                    "rover_id": command.rover(),
                    "engine": self.agent.engine,
                    "ok": result.is_ok(),
                    "note": match &result { Ok(note) | Err(note) => note },
                }));
                if let Ok(note) = &result {
                    self.note = format!("Agent: {note}");
                }
                result.map(|note| json!({ "note": note }))
            }
        }
    }

    fn field_state(&self) -> Value {
        json!({
            "protocol": AGENT_PROTOCOL,
            "mode": self.agent.mode.wire(),
            "fleet": self.fleet_mode.label(),
            "link": self.link,
            "time_s": self.now,
            "operator": {
                "has_control": self.claim.held(self.now),
                "selected": self.selected,
                "driving": self.driving,
                "note": self.note,
                "last_command": self.command,
            },
            "rovers": self.marks.iter().map(|mark| rover_json(self, mark)).collect::<Vec<_>>(),
            "unlocated": self.presence.iter()
                .filter(|presence| !self.marks.iter().any(|mark| mark.id == presence.id()))
                .map(|presence| json!({"id": presence.id(), "membership": presence.membership(), "reason": presence.position_reason()}))
                .collect::<Vec<_>>(),
            "recommendations": {
                "open": entries_json(self.agent.book.open(), self.now),
                "recent": entries_json(self.agent.book.recent(5), self.now),
            },
            "drive_grant": self.agent.grant.as_ref().map(|grant| grant.to_json(self.now)),
            "operator_task": self.agent.task.as_ref().map(|task| json!({
                "id": task.id,
                "text": task.text,
                "source": task.source,
                "age_s": (self.now - task.at).max(0.),
            })),
        })
    }

    fn propose(&mut self, items: Vec<Value>) -> Value {
        let engine = self.agent.engine.clone().unwrap_or_else(|| "agent".into());
        let results: Vec<Value> = items
            .iter()
            .map(|item| {
                let result = decode_recommendation(item, &engine).and_then(|rec| {
                    if !self.marks.iter().any(|mark| mark.id == rec.rover) {
                        return Err(format!("Rover {} is not on the map", rec.rover));
                    }
                    if let RecAction::HandoffBucket { object_id, .. } = rec.action {
                        self.bucket(rec.rover, object_id)?;
                    }
                    self.agent.book.propose(rec, self.now)
                });
                match result {
                    Ok((rec_id, superseded)) => {
                        if let Some(old) = &superseded {
                            self.log_decision(old);
                        }
                        if let Some(entry) = self.agent.book.get(&rec_id) {
                            self.feed.record(
                                json!({"kind": "rec_shown", "rec": entry.to_json(self.now)}),
                            );
                        }
                        json!({"ok": true, "rec_id": rec_id, "superseded": superseded})
                    }
                    Err(error) => json!({"ok": false, "error": error}),
                }
            })
            .collect();
        json!({ "results": results })
    }

    fn log_decision(&self, rec_id: &str) {
        if let Some(entry) = self.agent.book.get(rec_id) {
            self.feed
                .record(json!({"kind": "rec_decided", "rec": entry.to_json(self.now)}));
        }
    }

    /// The found bucket `object_id` on `rover`'s map.
    fn bucket(&self, rover: u64, object_id: u32) -> Result<(f64, f64), String> {
        self.marks
            .iter()
            .find(|mark| mark.id == rover)
            .and_then(|mark| {
                mark.objects
                    .iter()
                    .find(|found| found.object.id == object_id && found.object.class == "bucket")
            })
            .map(|found| (found.object.x, found.object.y))
            .ok_or_else(|| format!("Rover {rover} has not found a bucket #{object_id}"))
    }

    fn agent_mark(&self, rover: u64) -> Result<RoverMark, String> {
        let mark = self
            .marks
            .iter()
            .find(|mark| mark.id == rover)
            .cloned()
            .ok_or_else(|| format!("Rover {rover} is not on the map"))?;
        if self.driving && self.selected == Some(rover) {
            return Err(format!("The operator is driving rover {rover}"));
        }
        Ok(mark)
    }

    fn agent_command(&mut self, command: &AgentCommand) -> Result<String, String> {
        let source = CommandSource::Agent;
        // Commands only arrive in Operator mode, which is the operator's grant.
        // The agent takes this window's lease itself, as selecting a rover does.
        if !matches!(command, AgentCommand::Stop { .. } | AgentCommand::StopAll) {
            self.claim.take(self.now);
        }
        match command {
            AgentCommand::SetLevel {
                rover,
                level,
                budget_min,
            } => self.level_command(*rover, level, *budget_min, source),
            AgentCommand::GoTo { rover, goal } => self.goal_command(*rover, goal, source),
            AgentCommand::CancelGoal { rover } => {
                self.agent_mark(*rover)?;
                self.feed.as_source(source, |feed| feed.cancel_goal(*rover));
                Ok(format!("Cancel sent to rover {rover}"))
            }
            AgentCommand::StartTask {
                rover,
                instruction,
                budget_min,
                agent,
            } => self.task_command(*rover, instruction, *budget_min, *agent, source),
            AgentCommand::ReplyTask { rover, text } => {
                let task_id = self.running_task(*rover)?;
                self.feed.as_source(source, |feed| {
                    feed.publish_task_reply(*rover, task_id, text.clone())
                });
                Ok(format!("Reply sent to rover {rover}'s task"))
            }
            AgentCommand::CancelTask { rover } => {
                let task_id = self.running_task(*rover)?;
                self.feed
                    .as_source(source, |feed| feed.cancel_task(*rover, task_id));
                Ok(format!("Task cancel sent to rover {rover}"))
            }
            AgentCommand::Stop { rover } => {
                // Stop is allowed even while the operator drives; it stops them too.
                if !self.marks.iter().any(|mark| mark.id == *rover) {
                    return Err(format!("Rover {rover} is not on the map"));
                }
                self.end_agent_drive("stop");
                let previous = self.feed.swap_source(source);
                self.stop_rover(*rover, StopOrigin::Agent);
                self.feed.swap_source(previous);
                Ok(format!(
                    "Stop sent to rover {rover} · the operator must press Reset stop"
                ))
            }
            AgentCommand::StopAll => {
                self.end_agent_drive("stop");
                let previous = self.feed.swap_source(source);
                self.stop_all(StopOrigin::Agent);
                self.feed.swap_source(previous);
                Ok(self.note.clone())
            }
            AgentCommand::RequestDrive {
                rover,
                seconds,
                reason,
            } => {
                self.agent_mark(*rover)?;
                if let Some(grant) = &self.agent.grant {
                    return Err(format!(
                        "A drive grant for rover {} is already {}",
                        grant.rover,
                        if grant.pending() {
                            "waiting for the operator"
                        } else {
                            "running"
                        }
                    ));
                }
                let grant = DriveGrant::request(*rover, *seconds, reason.clone(), self.now);
                self.feed.record(json!({"kind": "drive_grant", "event": "requested", "grant": grant.to_json(self.now), "engine": self.agent.engine}));
                self.agent.grant = Some(grant);
                Ok(format!(
                    "Asked the operator to approve {seconds:.0} s of raw drive on rover {rover}; read drive_grant in state"
                ))
            }
            AgentCommand::Drive {
                rover,
                linear,
                angular,
                seconds,
            } => self.agent_drive(*rover, *linear, *angular, *seconds),
            AgentCommand::EndDrive { rover } => {
                if self
                    .agent
                    .grant
                    .as_ref()
                    .is_some_and(|grant| grant.rover == *rover)
                {
                    self.end_agent_drive("agent ended it");
                    Ok(format!("Drive ended on rover {rover}"))
                } else {
                    Err(format!("No drive grant for rover {rover}"))
                }
            }
        }
    }

    fn running_task(&self, rover: u64) -> Result<String, String> {
        self.agent_mark(rover)?
            .task
            .filter(|task| task.running())
            .map(|task| task.task_id)
            .ok_or_else(|| format!("Rover {rover} has no running task"))
    }

    /// The level buttons' checks, for an agent or an accepted card.
    fn level_command(
        &mut self,
        rover: u64,
        level: &str,
        budget_min: Option<u32>,
        source: CommandSource,
    ) -> Result<String, String> {
        argos_core::check_level(level, budget_min)?;
        let mark = self.agent_mark(rover)?;
        if !level_permitted(self.study_lock(&mark), level) {
            return Err(format!("The study condition locks {}", level_label(level)));
        }
        if level == "explore" {
            if let Some(reason) =
                explore_refusal(self.is_stopped(&mark), gps_blocks(&mark, "explore"))
            {
                return Err(reason.into());
            }
            let minutes = budget_min.unwrap_or(EXPLORE_BUDGETS_MIN[0]);
            self.feed.as_source(source, |feed| {
                feed.publish_explore(rover, f64::from(minutes) * 60.)
            });
            return Ok(format!(
                "Sending L4 Explore · {minutes} min to rover {rover}; read its level in state"
            ));
        }
        self.feed
            .as_source(source, |feed| feed.publish_level(rover, level));
        Ok(format!(
            "Sending {} to rover {rover}; read its level in state",
            level_label(level)
        ))
    }

    /// The Task · L5 section's checks.
    fn task_command(
        &mut self,
        rover: u64,
        instruction: &str,
        budget_min: u32,
        agent: Option<argos_core::TaskAgent>,
        source: CommandSource,
    ) -> Result<String, String> {
        let mark = self.agent_mark(rover)?;
        if !level_permitted(self.study_lock(&mark), "task") {
            return Err("The study condition locks L5 in this trial".into());
        }
        if let Some(reason) = task_refusal(
            self.is_stopped(&mark),
            gps_blocks(&mark, "task"),
            mark.task_supported,
        ) {
            return Err(reason.into());
        }
        let instruction = argos_core::task_text(instruction).map_err(|error| error.0)?;
        if !TASK_BUDGETS_MIN.contains(&budget_min) {
            return Err(format!(
                "An L5 task budget is one of {TASK_BUDGETS_MIN:?} min"
            ));
        }
        let agent = task_agent_choice(agent, &mark);
        self.feed.as_source(source, |feed| {
            feed.publish_task(rover, instruction, f64::from(budget_min) * 60., agent)
        });
        Ok(format!(
            "Sending an L5 task to rover {rover}'s {} · {budget_min} min",
            agent.label()
        ))
    }

    /// The goal drag's checks: the same gate, as if the operator dropped the
    /// ghost on this point with the rover selected.
    fn goal_command(
        &mut self,
        rover: u64,
        goal: &AgentGoal,
        _source: CommandSource,
    ) -> Result<String, String> {
        let mark = self.agent_mark(rover)?;
        if let Some(reason) = drive_block(&mark.hardware) {
            return Err(reason.into());
        }
        let level = mark
            .effective_level
            .clone()
            .or_else(|| mark.requested_level.clone());
        let (space, from, to, radius, gate) = match *goal {
            AgentGoal::Local { x, y } => {
                if mark.outdoor_gps {
                    return Err(
                        "Outdoor GPS is this rover's source; give latitude and longitude".into(),
                    );
                }
                let gate = DragGate {
                    selected: true,
                    link_healthy: self.link == "healthy",
                    pose_fresh: mark.status == "online",
                    degraded: mark.degraded,
                    level: level.as_deref(),
                    geographic: false,
                    outdoor: false,
                    admission: None,
                };
                let from = PlanarPoint {
                    x: mark.local_x,
                    y: mark.local_y,
                };
                (DragSpace::Local, from, PlanarPoint { x, y }, None, gate)
            }
            AgentGoal::Geographic {
                latitude,
                longitude,
            } => {
                if !mark.outdoor_gps {
                    return Err("This rover navigates in its local frame; give x and y".into());
                }
                let from = PlanarPoint {
                    x: mark.latitude,
                    y: mark.longitude,
                };
                let to = PlanarPoint {
                    x: latitude,
                    y: longitude,
                };
                (
                    DragSpace::Geographic,
                    from,
                    to,
                    Some(mark.arrival_radius),
                    geographic_gate(&mark, &self.link, true),
                )
            }
        };
        let mut drag = GoalDrag::start(rover, DragAnchor::Goal, space, from, from, radius, gate)?;
        drag.move_to(to);
        let radius = drag.arrival_radius_m;
        match drag.release(gate) {
            DragEnd::Cancelled(reason) => Err(reason.into()),
            DragEnd::Send(request) => {
                self.feed.publish_goal(rover, request, radius);
                Ok(format!(
                    "Goal sent to rover {rover}; read goal.state in state"
                ))
            }
        }
    }

    fn agent_drive(
        &mut self,
        rover: u64,
        linear: f64,
        angular: f64,
        seconds: f64,
    ) -> Result<String, String> {
        let remaining = match &self.agent.grant {
            Some(grant) if grant.rover == rover && grant.active(self.now) => {
                grant.remaining(self.now)
            }
            Some(grant) if grant.rover == rover && grant.pending() => {
                return Err("The operator has not approved drive yet".into());
            }
            _ => {
                return Err(format!(
                    "No approved drive grant for rover {rover}; call request_drive"
                ));
            }
        };
        if self.driving {
            return Err("The operator is driving; wait until they stop".into());
        }
        let mark = self.agent_mark(rover)?;
        if self.is_stopped(&mark) {
            return Err("Stop is active · the operator must press Reset stop".into());
        }
        if mark.requested_level.as_deref() != Some("teleop") {
            return Err("Raw drive needs L0 Manual; call set_level with teleop first".into());
        }
        let keys = drive_keys(linear, angular)?;
        self.release_burst();
        self.feed.begin_drive(rover)?;
        for key in &keys {
            if let Err(error) = self.feed.drive_key(rover, key, true) {
                self.release_burst_keys(rover, &keys);
                self.feed.end_drive();
                return Err(error);
            }
        }
        self.feed.refresh_drive(rover);
        let seconds = seconds.min(remaining);
        self.agent.burst = Some((rover, keys.clone(), self.now + seconds));
        self.feed.record(json!({
            "kind": "agent_drive", "rover_id": rover, "linear": linear, "angular": angular,
            "seconds": seconds, "keys": keys,
        }));
        let limits = self.feed.limits();
        Ok(format!(
            "Driving rover {rover} {} for {seconds:.1} s at the operator's limits ({:.2} m/s, {:.2} rad/s)",
            keys.join("+").to_uppercase(),
            limits.linear,
            limits.yaw_rate,
        ))
    }

    fn release_burst_keys(&self, rover: u64, keys: &[&'static str]) {
        for key in keys {
            let _ = self.feed.drive_key(rover, key, false);
        }
    }

    /// Let go of the agent's keys. The operator's own drive is left alone.
    fn release_burst(&mut self) {
        if let Some((rover, keys, _)) = self.agent.burst.take() {
            self.release_burst_keys(rover, &keys);
            if !self.driving {
                self.feed.end_drive();
            }
        }
    }

    /// End the agent's raw drive and its grant. Called by Stop, by the
    /// operator taking the wheel, and by the cards' Deny/End buttons.
    pub(super) fn end_agent_drive(&mut self, why: &str) {
        self.release_burst();
        if let Some(grant) = self.agent.grant.take() {
            let event = if grant.pending() { "denied" } else { "ended" };
            self.feed.record(json!({"kind": "drive_grant", "event": event, "why": why, "grant": grant.to_json(self.now)}));
        }
    }

    fn approve_drive(&mut self) {
        if let Some(grant) = self.agent.grant.as_mut().filter(|grant| grant.pending()) {
            grant.approve(self.now);
            let json = grant.to_json(self.now);
            self.note = format!(
                "Agent may drive rover {} for {:.0} s · Stop or End drive ends it",
                grant.rover, grant.seconds
            );
            self.feed
                .record(json!({"kind": "drive_grant", "event": "approved", "grant": json}));
        }
    }

    fn agent_upkeep(&mut self) {
        let now = self.now;
        self.voice_upkeep(now);
        self.agent_link_log(now);
        self.agent_cli_tick(now);
        for rec_id in self.agent.book.expire(now) {
            self.log_decision(&rec_id);
        }
        let marks = &self.marks;
        let stale = self.agent.book.supersede_where(now, |rec| {
            let mark = marks.iter().find(|mark| mark.id == rec.rover);
            recommendation_stale(
                rec,
                mark.is_some(),
                mark.and_then(|mark| mark.requested_level.as_deref()),
            )
        });
        for rec_id in stale {
            self.log_decision(&rec_id);
        }
        if let Some((rover, _, until)) = self.agent.burst.clone() {
            let granted = self
                .agent
                .grant
                .as_ref()
                .is_some_and(|grant| grant.rover == rover && grant.active(now));
            if now >= until || !granted {
                self.release_burst();
            } else {
                self.feed.refresh_drive(rover);
            }
        }
        if self
            .agent
            .grant
            .as_ref()
            .is_some_and(|grant| grant.lapsed(now))
        {
            let pending = self.agent.grant.as_ref().is_some_and(DriveGrant::pending);
            self.end_agent_drive(if pending {
                "nobody answered"
            } else {
                "time ran out"
            });
        }
        self.agent_queued(now);
        self.agent_outcomes(now);
    }

    /// Put a missing or stopped agent in the session log once, so a study
    /// run shows whether the advisor was there.
    fn agent_link_log(&mut self, now: f64) {
        if self.agent.bridge.is_none() {
            return;
        }
        let command = self.agent.process.as_ref().map(|p| p.command().to_string());
        let log = self
            .agent
            .process
            .as_ref()
            .map(|p| p.log().display().to_string());
        let exit = self
            .agent
            .process
            .as_mut()
            .and_then(|process| process.poll().map(str::to_string));
        if let Some(exit) = exit
            && !self.agent.exit_logged
        {
            self.agent.exit_logged = true;
            self.feed.record(json!({"kind": "agent_process", "event": "exited", "command": command, "exit": exit, "log": log}));
        }
        if self.agent.last_call.is_none()
            && !self.agent.missing_logged
            && now - self.agent.opened >= AGENT_CONNECT_GRACE_S
        {
            self.agent.missing_logged = true;
            self.feed.record(json!({
                "kind": "agent_missing",
                "after_s": now - self.agent.opened,
                "command": command,
                "bad_tokens": self.agent.bridge.as_ref().map_or(0, AgentBridge::bad_tokens),
            }));
        }
    }

    /// Let the picked engine use the operator tools. Needs the operator's own
    /// click each session; logged with who was signed in to this computer.
    fn set_engine_commands(&mut self, on: bool) {
        if self.agent.engine_commands == on {
            return;
        }
        self.agent.engine_commands = on;
        let engine = self.agent.runner.as_ref().map(|runner| runner.cli.engine());
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_default();
        self.feed.record(json!({
            "kind": "agent_engine_commands",
            "enabled": on,
            "engine": engine,
            "by": "operator (Agent panel)",
            "os_user": user,
        }));
        self.note = if on {
            format!(
                "{} may now command rovers, through the same checks as you",
                engine.unwrap_or("The engine")
            )
        } else {
            "The engine only advises".into()
        };
        let note = self.note.clone();
        self.agent.say(ChatKind::Event, note, self.now);
    }

    /// Pick the engine ARGOS runs each tick, or none.
    fn set_agent_cli(&mut self, cli: Option<AgentCli>) {
        if self.agent.runner.as_ref().map(|runner| runner.cli) == cli {
            return;
        }
        if let Some((mut process, _)) = self.agent.runner.take().and_then(|runner| runner.run) {
            process.kill();
        }
        self.feed
            .record(json!({"kind": "agent_engine", "engine": cli.map(AgentCli::engine)}));
        self.agent.runner = cli.map(CliRunner::new);
        self.agent.say(
            ChatKind::Event,
            format!("Engine: {}", cli.map_or("none", AgentCli::label)),
            self.now,
        );
        self.set_engine_commands(false);
        self.agent.missing_logged = false;
        self.agent.opened = self.now;
        self.note = match cli {
            Some(cli) => format!(
                "ARGOS runs {} every {AGENT_TICK_S:.0} s, sooner when something changes",
                cli.label()
            ),
            None => "No engine: connect an MCP client to `argos-gpui mcp` yourself".into(),
        };
    }

    /// Run the picked engine once per tick: every 60 s, or after 15 s when
    /// the field changed. One run at a time; a run over 180 s is stopped.
    fn agent_cli_tick(&mut self, now: f64) {
        let Some(bridge_file) = self.agent.bridge.as_ref().map(|b| b.file().to_path_buf()) else {
            return;
        };
        let Some(runner) = self.agent.runner.as_mut() else {
            return;
        };
        if let Some((process, started)) = runner.run.as_mut() {
            let seconds = now - *started;
            let ended = if process.poll().is_some() {
                Some(process.succeeded() == Some(true))
            } else if seconds > AGENT_RUN_MAX_S {
                process.kill();
                Some(false)
            } else {
                None
            };
            let Some(ok) = ended else {
                return;
            };
            let exit = process.exited().unwrap_or_default().to_string();
            let answer = exit.split_once(" · ").map(|(_, last)| last.to_string());
            let label = runner.cli.label();
            let said = if ok {
                runner.error = None;
                runner.answer = answer.clone();
                let reply = std::fs::read_to_string(&runner.log)
                    .ok()
                    .and_then(|log| tick_reply(&log))
                    .or(answer);
                reply.map(|reply| (ChatKind::Agent(label), reply))
            } else {
                runner.error = Some(if seconds > AGENT_RUN_MAX_S {
                    format!("{label} took over {AGENT_RUN_MAX_S:.0} s and was stopped")
                } else {
                    exit.clone()
                });
                runner.error.clone().map(|error| (ChatKind::Problem, error))
            };
            let engine = runner.cli.engine();
            runner.run = None;
            let ticks = runner.ticks;
            if let Some((kind, text)) = said {
                let before = self.agent.chat.len();
                self.agent.say(kind, text, now);
                if self.agent.chat.len() > before
                    && let Some(line) = self.agent.chat.last_mut()
                    && matches!(line.kind, ChatKind::Agent(_))
                {
                    line.meta = Some(format!("Ran {label} · {seconds:.0} s · tick {ticks}"));
                }
            }
            self.feed.record(json!({"kind": "agent_run", "event": "ended", "engine": engine, "ok": ok, "seconds": seconds, "exit": exit}));
            return;
        }
        if self.agent.mode == AgentMode::Off || now - runner.last_check < FIELD_CHECK_S {
            return;
        }
        runner.last_check = now;
        let state = self.field_state();
        let runner = self.agent.runner.as_mut().expect("checked above");
        let mut why = tick_reasons(runner.last_state.as_ref(), &state);
        runner.last_state = Some(state);
        let task = self.agent.task.clone();
        let fresh_task = task
            .as_ref()
            .is_some_and(|task| runner.task_sent != Some(task.id));
        if fresh_task {
            why.insert(0, "operator task".into());
        }
        let since = runner.last_tick.map_or(f64::INFINITY, |at| now - at);
        let due = fresh_task
            || since >= AGENT_TICK_S
            || (!why.is_empty() && since >= AGENT_EARLY_TICK_MIN_S);
        if !due {
            return;
        }
        if why.is_empty() {
            why.push("timer".into());
        }
        runner.last_tick = Some(now);
        runner.ticks += 1;
        runner.task_sent = task.as_ref().map(|task| task.id);
        let profile = if self.agent.mode == AgentMode::Operator && self.agent.engine_commands {
            McpProfile::Operator
        } else {
            McpProfile::Advise
        };
        let dir = argos_gpui::session_dir();
        let started = std::env::current_exe().and_then(|exe| {
            let command = runner.cli.command(&AgentRun {
                exe: &exe,
                bridge: &bridge_file,
                profile,
                dir: &dir,
                task: task.as_ref().map(|task| task.text.as_str()),
            })?;
            if let Ok(mut log) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&runner.log)
            {
                use std::io::Write;
                let _ = writeln!(
                    log,
                    "\n{TICK_MARK}{} ({}) ---",
                    runner.ticks,
                    why.join(", ")
                );
            }
            AgentProcess::start(command, runner.cli.label(), runner.log.clone())
        });
        let engine = runner.cli.engine();
        match started {
            Ok(process) => {
                runner.run = Some((process, now));
                self.feed.record(json!({"kind": "agent_run", "event": "started", "engine": engine, "tick": runner.ticks, "why": why, "profile": profile.wire(), "task_id": task.as_ref().map(|task| task.id)}));
            }
            Err(error) => {
                runner.error = Some(if error.kind() == std::io::ErrorKind::NotFound {
                    format!("`{}` is not installed or not on PATH", runner.cli.program())
                } else {
                    format!("could not start {}: {error}", runner.cli.label())
                });
                let error = runner.error.clone().unwrap_or_default();
                self.feed.record(json!({"kind": "agent_run", "event": "failed", "engine": engine, "error": error}));
                self.agent.say(ChatKind::Problem, error, now);
            }
        }
    }

    /// Give the engine a task in the operator's words. It runs at once (or
    /// right after the run in progress) and stays in the briefing until the
    /// operator clears or replaces it. The engine's moves still come back as
    /// cards; nothing here commands a rover.
    pub(super) fn set_operator_task(&mut self, text: &str, source: &'static str) {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            self.note = "Say or type what the fleet should do".into();
            return;
        }
        let text: String = text.chars().take(OPERATOR_TASK_MAX_CHARS).collect();
        self.agent.next_task += 1;
        let task = OperatorTask {
            id: self.agent.next_task,
            text,
            source,
            at: self.now,
        };
        let replaced = self.agent.task.as_ref().map(|old| old.id);
        self.feed.record(json!({
            "kind": "operator_task",
            "event": "set",
            "id": task.id,
            "text": task.text,
            "source": source,
            "replaced": replaced,
            "engine": self.agent.runner.as_ref().map(|runner| runner.cli.engine()),
        }));
        self.note = match (&self.agent.runner, self.agent.mode) {
            (_, AgentMode::Off) => {
                "Task saved. Set the Agent panel to Advise so the engine can plan it".into()
            }
            (Some(runner), _) => format!("{} is planning: {}", runner.cli.label(), task.text),
            (None, _) => "Task saved. Pick Claude or Codex as the engine to plan it".into(),
        };
        if let Some(runner) = self.agent.runner.as_mut() {
            runner.last_check = f64::NEG_INFINITY;
        }
        self.agent
            .say(ChatKind::Operator(source), task.text.clone(), self.now);
        if self.agent.mode == AgentMode::Off || self.agent.runner.is_none() {
            let note = self.note.clone();
            self.agent.say(ChatKind::Event, note, self.now);
        }
        self.agent.task = Some(task);
    }

    pub(super) fn clear_operator_task(&mut self, why: &str) {
        if let Some(task) = self.agent.task.take() {
            self.feed.record(json!({
                "kind": "operator_task", "event": "cleared", "id": task.id, "why": why,
                "age_s": self.now - task.at,
            }));
            self.note = "Task cleared; the engine goes back to watching the fleet".into();
            let note = self.note.clone();
            self.agent.say(ChatKind::Event, note, self.now);
        }
    }

    /// Talk button: start recording, or stop and transcribe.
    pub(super) fn toggle_voice(&mut self) {
        if self.agent.voice.recording.is_some() {
            self.finish_voice();
            return;
        }
        if self.agent.voice.pending.is_some() {
            self.note = "Still working out what you said".into();
            return;
        }
        if self.agent.voice.download.is_some() {
            self.note = "Still downloading the speech model".into();
            return;
        }
        if let Err(error) = Transcriber::from_env() {
            // whisper.cpp from Homebrew ships without a model: fetch it once.
            if let Some((_, dest)) = missing_model() {
                let (send, receive) = mpsc::channel();
                let path = dest.clone();
                std::thread::spawn(move || {
                    let _ = send.send(download_model(&path));
                });
                self.agent.voice.download = Some(receive);
                self.agent.voice.note = None;
                self.note = format!(
                    "Downloading the Whisper speech model (~150 MB, once) to {}",
                    dest.display()
                );
                self.feed.record(json!({"kind": "operator_voice", "event": "model_download", "to": dest.display().to_string()}));
                return;
            }
            self.agent.voice.note = Some((error, true));
            return;
        }
        match Recording::start() {
            Ok(recording) => {
                self.agent.voice.recording = Some(recording);
                self.agent.voice.note = None;
                self.note = "Listening. Press Talk again (or Cmd/Ctrl+Shift+V) when done".into();
            }
            Err(error) => self.agent.voice.note = Some((error, true)),
        }
    }

    fn finish_voice(&mut self) {
        let Some(recording) = self.agent.voice.recording.take() else {
            return;
        };
        let driver_error = recording.error();
        let samples = recording.finish();
        let seconds = clip_seconds(&samples);
        if let Some(error) = driver_error.filter(|_| seconds < MIN_RECORD_S) {
            self.agent.voice.note = Some((format!("Microphone error: {error}"), true));
            return;
        }
        if seconds < MIN_RECORD_S {
            self.agent.voice.note = Some((
                "Too short. Press Talk, speak, then press it again".into(),
                true,
            ));
            return;
        }
        let transcriber = match Transcriber::from_env() {
            Ok(transcriber) => transcriber,
            Err(error) => {
                self.agent.voice.note = Some((error, true));
                return;
            }
        };
        self.agent.voice.clips += 1;
        let name = format!(
            "{}voice-{}.wav",
            argos_gpui::AGENT_LOG_PREFIX,
            self.agent.voice.clips
        );
        let (send, receive) = mpsc::channel();
        let label = transcriber.label();
        std::thread::spawn(move || {
            let dir = argos_gpui::session_dir();
            let _ = send.send(transcribe_clip(&transcriber, &samples, &dir, &name));
        });
        self.agent.voice.pending = Some((receive, self.now));
        self.note = format!("Transcribing {seconds:.0} s with {label}");
    }

    fn voice_upkeep(&mut self, now: f64) {
        if self
            .agent
            .voice
            .recording
            .as_ref()
            .is_some_and(|recording| recording.seconds() >= MAX_RECORD_S)
        {
            self.finish_voice();
        }
        if let Some(receive) = self.agent.voice.download.as_ref() {
            let done = match receive.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("The model download stopped".into()))
                }
            };
            if let Some(result) = done {
                self.agent.voice.download = None;
                match result {
                    Ok(()) => {
                        self.note = "Speech model ready. Press Talk and speak".into();
                        self.agent.voice.note = None;
                    }
                    Err(error) => self.agent.voice.note = Some((error, true)),
                }
            }
        }
        let Some((receive, started)) = self.agent.voice.pending.as_ref() else {
            return;
        };
        let started = *started;
        let heard = match receive.try_recv() {
            Ok(heard) => heard,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("The transcriber stopped".into()),
        };
        self.agent.voice.pending = None;
        match heard {
            Ok((text, clip)) => {
                self.feed.record(json!({
                    "kind": "operator_voice", "text": text, "clip": clip.display().to_string(),
                    "transcribe_s": now - started,
                }));
                self.agent.voice.note = Some((format!("Heard: \"{text}\""), false));
                self.set_operator_task(&text, "voice");
            }
            Err(error) => {
                self.feed
                    .record(json!({"kind": "operator_voice", "error": error}));
                self.agent.voice.note = Some((error, true));
            }
        }
    }

    /// Send what is typed in the Agent panel's task box as the task.
    pub(super) fn send_typed_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.agent_task_input.read(cx).value().to_string();
        if text.trim().is_empty() {
            self.note = "Type or say what the fleet should do".into();
            return;
        }
        self.set_operator_task(&text, "typed");
        self.agent_task_input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    /// Send accepted goals once their rover reaches L2 or L3.
    fn agent_queued(&mut self, now: f64) {
        let queued = std::mem::take(&mut self.agent.queued);
        for item in queued {
            let level = self
                .marks
                .iter()
                .find(|mark| mark.id == item.rover)
                .and_then(|mark| mark.effective_level.clone());
            if matches!(level.as_deref(), Some("waypoint" | "waypoint_direct")) {
                let result =
                    self.goal_command(item.rover, &item.goal, CommandSource::Recommendation);
                self.finish_dispatch(&item.rec_id, item.rover, result);
            } else if now >= item.deadline {
                self.finish_dispatch(
                    &item.rec_id,
                    item.rover,
                    Err("The rover did not reach L3 in time; goal not sent".into()),
                );
            } else {
                self.agent.queued.push(item);
            }
        }
    }

    fn agent_outcomes(&mut self, now: f64) {
        let watched = std::mem::take(&mut self.agent.watched);
        for mut item in watched {
            let state = self
                .marks
                .iter()
                .find(|mark| mark.id == item.rover)
                .and_then(|mark| mark.goal.as_ref())
                .map(|goal| goal.state.clone());
            item.seen_active |= state.as_deref() == Some("active");
            let arrived = item.seen_active && state.as_deref() == Some("arrived");
            if arrived || now >= item.deadline {
                let outcome = if arrived { "arrived" } else { "unknown" };
                self.feed.record(json!({"kind": "rec_outcome", "rec_id": item.rec_id, "rover_id": item.rover, "outcome": outcome}));
                if arrived && let Some(bucket) = item.handoff {
                    self.note = format!(
                        "Rover {} is at bucket #{bucket} · your turn: line it up and dump by hand",
                        item.rover
                    );
                }
            } else {
                self.agent.watched.push(item);
            }
        }
    }

    fn finish_dispatch(&mut self, rec_id: &str, rover: u64, result: Result<String, String>) {
        if result.is_ok() {
            let handoff = match self.agent.book.get(rec_id).map(|entry| &entry.rec.action) {
                Some(RecAction::HandoffBucket { object_id, .. }) => Some(Some(*object_id)),
                Some(RecAction::GoTo(_)) => Some(None),
                _ => None,
            };
            if let Some(handoff) = handoff {
                self.agent.watched.push(Watched {
                    rec_id: rec_id.to_string(),
                    rover,
                    handoff,
                    deadline: self.now + OUTCOME_WAIT_S,
                    seen_active: false,
                });
            }
        }
        self.note = match &result {
            Ok(note) | Err(note) => note.clone(),
        };
        self.feed.record(json!({
            "kind": "rec_dispatched", "rec_id": rec_id, "rover_id": rover,
            "ok": result.is_ok(), "note": self.note,
        }));
        let title = self
            .agent
            .book
            .get(rec_id)
            .map(|entry| format!("R{rover} · {}", entry.rec.action.title()))
            .unwrap_or_else(|| format!("R{rover}"));
        match &result {
            Ok(note) => self
                .agent
                .say(ChatKind::Event, format!("{title}: {note}"), self.now),
            Err(error) => self
                .agent
                .say(ChatKind::Problem, format!("{title}: {error}"), self.now),
        }
        self.agent.book.dispatched(rec_id, result);
    }

    /// A goal from an accepted card: sent now from L2/L3, otherwise L3 first.
    fn card_goal(
        &mut self,
        rec_id: &str,
        rover: u64,
        goal: AgentGoal,
    ) -> Option<Result<String, String>> {
        let level = self
            .marks
            .iter()
            .find(|mark| mark.id == rover)
            .and_then(|mark| mark.effective_level.clone());
        if matches!(level.as_deref(), Some("waypoint" | "waypoint_direct")) {
            return Some(self.goal_command(rover, &goal, CommandSource::Recommendation));
        }
        if let Err(error) =
            self.level_command(rover, "waypoint", None, CommandSource::Recommendation)
        {
            return Some(Err(error));
        }
        self.agent.queued.push(QueuedGoal {
            rec_id: rec_id.to_string(),
            rover,
            goal,
            deadline: self.now + LEVEL_WAIT_S,
        });
        self.note = format!("Switching rover {rover} to L3, then sending the goal");
        None
    }

    pub(super) fn accept_card(&mut self, rec_id: &str) {
        let Some(entry) = self.agent.book.get(rec_id).cloned() else {
            return;
        };
        // Like the buttons, a card that sends something needs the operator lease.
        if !matches!(entry.rec.action, RecAction::Attention) && !self.claim.held(self.now) {
            self.note = "Control the fleet to accept this card".into();
            return;
        }
        if self
            .agent
            .book
            .decide(rec_id, RecState::Accepted, None, self.now)
            .is_err()
        {
            return;
        }
        self.agent.rejecting = None;
        self.log_decision(rec_id);
        self.agent.say(
            ChatKind::Event,
            format!(
                "You accepted R{} · {}",
                entry.rec.rover,
                entry.rec.action.title()
            ),
            self.now,
        );
        let rover = entry.rec.rover;
        let source = CommandSource::Recommendation;
        let result = match entry.rec.action {
            RecAction::SetLevel { level, budget_min } => {
                Some(self.level_command(rover, &level, budget_min, source))
            }
            RecAction::Task {
                instruction,
                budget_min,
            } => Some(self.task_command(rover, &instruction, budget_min, None, source)),
            RecAction::GoTo(goal) => self.card_goal(rec_id, rover, goal),
            RecAction::HandoffBucket {
                object_id,
                approach_m,
            } => {
                let target = self.bucket(rover, object_id).and_then(|bucket| {
                    let mark = self.agent_mark(rover)?;
                    approach_point((mark.local_x, mark.local_y), bucket, approach_m)
                });
                match target {
                    Ok((x, y)) => self.card_goal(rec_id, rover, AgentGoal::Local { x, y }),
                    Err(error) => Some(Err(error)),
                }
            }
            RecAction::Attention => Some(Ok(format!("Noted for rover {rover}"))),
        };
        if let Some(result) = result {
            self.finish_dispatch(rec_id, rover, result);
        }
    }

    fn reject_card(&mut self, rec_id: &str, reason: &str) {
        self.agent.rejecting = None;
        if self
            .agent
            .book
            .decide(
                rec_id,
                RecState::Rejected,
                Some(reason.to_string()),
                self.now,
            )
            .is_ok()
        {
            self.log_decision(rec_id);
            self.note = "Recommendation rejected".into();
            let label = REJECT_REASONS
                .iter()
                .find(|(wire, _)| *wire == reason)
                .map_or(reason, |(_, label)| *label);
            let title = self
                .agent
                .book
                .get(rec_id)
                .map_or_else(String::new, |entry| {
                    format!(" R{} · {}", entry.rec.rover, entry.rec.action.title())
                });
            self.agent.say(
                ChatKind::Event,
                format!("You rejected{title} ({label})"),
                self.now,
            );
        }
    }

    fn set_agent_mode(&mut self, mode: AgentMode) {
        if mode == self.agent.mode {
            return;
        }
        if mode != AgentMode::Operator {
            self.end_agent_drive("agent mode changed");
            self.agent.queued.clear();
            self.set_engine_commands(false);
        }
        self.feed.record(
            json!({"kind": "agent_mode", "from": self.agent.mode.wire(), "to": mode.wire()}),
        );
        self.agent.mode = mode;
        self.agent.say(
            ChatKind::Event,
            format!("Agent set to {}", mode.label()),
            self.now,
        );
        self.note = match mode {
            AgentMode::Off => "Agent access off".into(),
            AgentMode::Advise => "Agent may read the field and propose recommendations".into(),
            AgentMode::Operator => {
                "Agent may command rovers through the same checks as you · raw drive still needs your approval".into()
            }
        };
    }
}

fn urgency_tag(urgency: u8) -> Tag {
    match urgency {
        3 => Tag::danger().small().child("NOW"),
        2 => Tag::warning().small().child("SOON"),
        _ => Tag::secondary().small().child("FYI"),
    }
}

/// "12 s ago", "3 min ago" for a conversation time stamp.
fn ago(now: f64, at: f64) -> String {
    let seconds = (now - at).max(0.);
    if seconds < 60. {
        format!("{seconds:.0} s ago")
    } else {
        format!("{:.0} min ago", seconds / 60.)
    }
}

/// Ideas shown in an empty conversation. Clicking one fills the composer.
const STARTERS: [&str; 3] = [
    "Rover 1 explores the left side, rover 2 goes home",
    "Which rover needs my attention?",
    "Bring the nearest bucket back to base",
];

/// The Agent panel, laid out like a coding-agent chat: the conversation
/// fills the panel and sits on the bottom edge, engine replies are plain
/// text, the operator's tasks are pill bubbles, cards sit inline, and the
/// composer card holds the engine/mode picker, Talk and a round Send. A
/// one-line status sits under the composer. Accept/Reject on cards, Let it
/// command, and the raw-drive grant work as before; only the layout changed.
pub(super) fn agent_panel(dashboard: &Dashboard, cx: &mut Context<Dashboard>) -> AnyElement {
    v_flex()
        .id("agent-panel")
        .size_full()
        .min_h_0()
        .bg(cx.theme().background)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .when_some(command_strip(dashboard, cx), |panel, strip| {
            panel.child(strip)
        })
        .child(conversation(dashboard, cx))
        .child(composer(dashboard, cx))
        .child(status_footer(dashboard, cx))
        .into_any_element()
}

/// Operator mode only: whether the engine may command, and the switch.
fn command_strip(dashboard: &Dashboard, cx: &mut Context<Dashboard>) -> Option<AnyElement> {
    let desk = &dashboard.agent;
    if desk.mode != AgentMode::Operator {
        return None;
    }
    let (label, on) = desk
        .runner
        .as_ref()
        .map(|runner| (runner.cli.label(), desk.engine_commands))?;
    Some(
        h_flex()
            .mx_3()
            .mt_3()
            .gap_2()
            .px_3()
            .py_1p5()
            .rounded_lg()
            .border_1()
            .border_color(if on {
                cx.theme().warning
            } else {
                cx.theme().border
            })
            .child(div().flex_1().min_w_0().child(if on {
                caution(format!("{label} may command rovers this session"), cx)
            } else {
                hint(format!("{label} only advises, even in Operator mode"), cx)
            }))
            .child(
                Button::new("agent-engine-commands")
                    .xsmall()
                    .outline()
                    .label(if on { "Advise only" } else { "Let it command" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_engine_commands(!on);
                        cx.notify();
                    })),
            )
            .into_any_element(),
    )
}

/// The scrolling conversation: past lines, what the engine is doing now,
/// the drive request, and the open cards. Rows sit on the bottom edge and
/// the view follows new ones.
fn conversation(dashboard: &Dashboard, cx: &mut Context<Dashboard>) -> AnyElement {
    let now = dashboard.now;
    let desk = &dashboard.agent;
    let mut rows: Vec<AnyElement> = Vec::new();
    let cards: Vec<RecEntry> = desk.book.open().into_iter().cloned().collect();
    let working = desk.runner.as_ref().and_then(|runner| {
        runner
            .run
            .as_ref()
            .map(|(_, started)| (runner.cli.label(), now - started))
    });
    let empty =
        desk.chat.is_empty() && cards.is_empty() && working.is_none() && desk.grant.is_none();
    if empty {
        rows.push(empty_chat(dashboard, cx));
    }
    for line in &desk.chat {
        rows.push(chat_line(line, now, cx));
    }
    if let Some((label, seconds)) = working {
        let doing = if desk.task.is_some() {
            "planning your task"
        } else {
            "checking the field"
        };
        rows.push(fold_line(format!("{label} is {doing}… {seconds:.0} s"), cx).into_any_element());
    }
    if let Some(grant) = desk.grant.clone() {
        rows.push(drive_card(&grant, now, cx));
    }
    let rejecting = desk.rejecting.clone();
    for entry in cards {
        rows.push(rec_card(entry, rejecting.as_deref(), now, cx));
    }
    // Follow the bottom when a row is added (a new line, card, or run).
    let count = desk.chat.len() * 4 + rows.len();
    if desk.chat_rows.replace(count) != count {
        desk.chat_scroll.scroll_to_bottom();
    }
    div()
        .id("agent-chat")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&desk.chat_scroll)
        .child(
            v_flex()
                .min_h_full()
                .when(!empty, |column| column.justify_end())
                .px_4()
                .py_4()
                .gap_4()
                .children(rows),
        )
        .into_any_element()
}

/// The small "› Ran Claude · 14 s" line above an engine reply.
fn fold_line(text: String, cx: &App) -> Div {
    h_flex()
        .gap_1p5()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child("›")
        .child(text)
}

/// An engine reply as plain text. Lines starting with `- ` or `* ` become
/// bullets and `**` markers are dropped, so a Markdown-ish answer reads cleanly.
fn reply_body(text: &str) -> Div {
    v_flex()
        .gap_1p5()
        .text_sm()
        .children(
            text.lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    let line = line.replace("**", "");
                    let trimmed = line.trim_start();
                    match trimmed
                        .strip_prefix("- ")
                        .or_else(|| trimmed.strip_prefix("* "))
                    {
                        Some(item) => h_flex()
                            .items_start()
                            .gap_2()
                            .pl_2()
                            .child("•")
                            .child(div().flex_1().min_w_0().child(item.to_string()))
                            .into_any_element(),
                        None => div().child(line.clone()).into_any_element(),
                    }
                }),
        )
}

fn chat_line(line: &ChatLine, now: f64, cx: &App) -> AnyElement {
    match line.kind {
        ChatKind::Operator(source) => v_flex()
            .w_full()
            .items_end()
            .gap_1()
            .child(
                div()
                    .max_w(relative(0.85))
                    .px_4()
                    .py_2p5()
                    .rounded_2xl()
                    .border_1()
                    .border_color(cx.theme().primary.opacity(0.25))
                    .bg(cx.theme().primary.opacity(0.07))
                    .text_sm()
                    .child(line.text.clone()),
            )
            .when(source == "voice", |column| {
                column.child(hint(format!("Spoken · {}", ago(now, line.at)), cx))
            })
            .into_any_element(),
        ChatKind::Agent(_) => v_flex()
            .w_full()
            .gap_2()
            .when_some(line.meta.clone(), |column, meta| {
                column.child(fold_line(meta, cx))
            })
            .child(reply_body(&line.text))
            .into_any_element(),
        ChatKind::Problem => div()
            .w_full()
            .pl_3()
            .py_1()
            .border_l_2()
            .border_color(cx.theme().warning)
            .child(caution(line.text.clone(), cx))
            .into_any_element(),
        ChatKind::Event => div()
            .w_full()
            .text_center()
            .child(hint(format!("{} · {}", line.text, ago(now, line.at)), cx))
            .into_any_element(),
    }
}

/// First view: what the panel is for and a few things to ask.
fn empty_chat(dashboard: &Dashboard, cx: &mut Context<Dashboard>) -> AnyElement {
    let lead = match dashboard.agent.runner.as_ref() {
        Some(runner) => format!(
            "Type or say a task. {} plans it and answers with cards you accept or reject.",
            runner.cli.label()
        ),
        None => "Pick an engine below, then type or say a task. It answers with cards you accept or reject.".to_string(),
    };
    v_flex()
        .size_full()
        .justify_center()
        .items_center()
        .gap_2()
        .child(
            div()
                .text_base()
                .font_semibold()
                .child("What should the fleet do?"),
        )
        .child(div().text_center().child(hint(lead, cx)))
        .child(
            v_flex()
                .w_full()
                .pt_3()
                .gap_1p5()
                .children(STARTERS.iter().enumerate().map(|(index, text)| {
                    Button::new(("agent-starter", index as u64))
                        .outline()
                        .small()
                        .w_full()
                        .rounded(ButtonRounded::Large)
                        .label(*text)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.agent_task_input
                                .update(cx, |input, cx| input.set_value(*text, window, cx));
                            this.agent_task_input
                                .read(cx)
                                .focus_handle(cx)
                                .focus(window, cx);
                            cx.notify();
                        }))
                })),
        )
        .into_any_element()
}
/// The engine asks to drive, or is driving: Approve / Deny / End.
fn drive_card(grant: &DriveGrant, now: f64, cx: &mut Context<Dashboard>) -> AnyElement {
    let pending = grant.pending();
    v_flex()
        .gap_2()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().warning)
        .child(caution(
            if pending {
                format!(
                    "Agent asks to drive rover {} for {:.0} s: {}. It drives with your W/A/S/D limits and stops when you press Stop.",
                    grant.rover, grant.seconds, grant.reason
                )
            } else {
                format!(
                    "Agent is allowed to drive rover {} · {:.0} s left",
                    grant.rover,
                    grant.remaining(now)
                )
            },
            cx,
        ))
        .child(
            h_flex()
                .gap_2()
                .when(pending, |row| {
                    row.child(
                        Button::new("agent-drive-approve")
                            .small()
                            .primary()
                            .label("Approve drive")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.approve_drive();
                                cx.notify();
                            })),
                    )
                })
                .child(
                    Button::new("agent-drive-deny")
                        .outline()
                        .small()
                        .label(if pending { "Deny drive" } else { "End drive" })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.end_agent_drive("operator");
                            this.note = "Agent drive ended".into();
                            cx.notify();
                        })),
                ),
        )
        .into_any_element()
}

/// One open recommendation, inline in the conversation, with Accept/Reject.
fn rec_card(
    entry: RecEntry,
    rejecting: Option<&str>,
    now: f64,
    cx: &mut Context<Dashboard>,
) -> AnyElement {
    let rec_id = entry.rec.rec_id.clone();
    let picking = rejecting == Some(rec_id.as_str());
    let accept_id = rec_id.clone();
    let reject_id = rec_id.clone();
    let refused = match &entry.dispatch {
        Some(Err(error)) => Some(error.clone()),
        _ => None,
    };
    v_flex()
        .gap_1p5()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(if entry.rec.urgency == 3 {
            cx.theme().danger
        } else {
            cx.theme().border
        })
        .bg(cx.theme().background)
        .shadow_xs()
        .child(
            h_flex()
                .gap_2()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .min_w_0()
                        .truncate()
                        .child(format!(
                            "R{} · {}",
                            entry.rec.rover,
                            entry.rec.action.title()
                        )),
                )
                .child(urgency_tag(entry.rec.urgency)),
        )
        .child(div().text_xs().child(entry.rec.reason.clone()))
        .when_some(refused, |card, error| card.child(caution(error, cx)))
        .child(if picking {
            h_flex()
                .gap_1()
                .flex_wrap()
                .children(
                    REJECT_REASONS
                        .iter()
                        .enumerate()
                        .map(|(index, (wire, label))| {
                            let rec_id = reject_id.clone();
                            Button::new(SharedString::from(format!(
                                "agent-reason-{rec_id}-{index}"
                            )))
                            .outline()
                            .xsmall()
                            .label(*label)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.reject_card(&rec_id, wire);
                                    cx.notify();
                                },
                            ))
                        }),
                )
                .into_any_element()
        } else {
            h_flex()
                .gap_2()
                .child(
                    Button::new(SharedString::from(format!("agent-accept-{accept_id}")))
                        .small()
                        .primary()
                        .label(if entry.rec.action == RecAction::Attention {
                            "Got it"
                        } else {
                            "Accept"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.accept_card(&accept_id);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("agent-reject-{reject_id}")))
                        .outline()
                        .small()
                        .label("Reject")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.agent.rejecting = Some(reject_id.clone());
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .text_right()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{:.0} s", entry.remaining(now))),
                )
                .into_any_element()
        })
        .into_any_element()
}

/// Engine and mode in one compact picker, like a model picker in a chat app.
fn engine_picker(dashboard: &Dashboard, cx: &mut Context<Dashboard>) -> impl IntoElement {
    let desk = &dashboard.agent;
    let engine = desk.runner.as_ref().map(|runner| runner.cli);
    let mode = desk.mode;
    let weak = cx.entity().downgrade();
    Button::new("agent-engine-menu")
        .ghost()
        .small()
        .label(format!(
            "✳ {}  {} ▾",
            engine.map_or("No engine", AgentCli::label),
            mode.label()
        ))
        .tooltip("Engine ARGOS runs each tick, and what it may do")
        .dropdown_menu(move |menu, _, _| {
            let mut menu = menu.label("Engine");
            for choice in std::iter::once(None).chain(AgentCli::ALL.iter().copied().map(Some)) {
                let weak = weak.clone();
                menu = menu.item(
                    PopupMenuItem::new(choice.map_or("None", AgentCli::label))
                        .checked(choice == engine)
                        .on_click(move |_, _, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.set_agent_cli(choice);
                                cx.notify();
                            });
                        }),
                );
            }
            menu = menu.separator().label("Agent access");
            for each in AgentMode::ALL {
                let weak = weak.clone();
                menu = menu.item(
                    PopupMenuItem::new(each.label())
                        .checked(each == mode)
                        .on_click(move |_, _, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.set_agent_mode(each);
                                cx.notify();
                            });
                        }),
                );
            }
            menu
        })
}

/// Bottom composer card: current task with Clear, the task box, then the
/// engine/mode picker, Talk and a round Send.
fn composer(dashboard: &Dashboard, cx: &mut Context<Dashboard>) -> AnyElement {
    let now = dashboard.now;
    let voice = &dashboard.agent.voice;
    let talk_label = match (&voice.recording, &voice.pending) {
        (Some(recording), _) => format!("● Stop · {:.0} s", recording.seconds()),
        (None, Some(_)) => "Transcribing…".to_string(),
        (None, None) if voice.download.is_some() => "Getting model…".to_string(),
        (None, None) => "Talk".to_string(),
    };
    let recording = voice.recording.is_some();
    let busy = voice.pending.is_some() || voice.download.is_some();
    // A heard phrase already shows as the operator's bubble; only problems show here.
    let voice_problem = voice
        .note
        .clone()
        .and_then(|(note, warn)| warn.then_some(note));
    let task = dashboard.agent.task.clone();
    v_flex()
        .px_3()
        .gap_1p5()
        .when_some(voice_problem, |column, note| {
            column.child(caution(note, cx))
        })
        .child(
            v_flex()
                .gap_2()
                .px_3()
                .pt_3()
                .pb_2()
                .rounded_2xl()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .shadow_sm()
                .when_some(task, |column, task| {
                    column.child(
                        h_flex()
                            .gap_1()
                            .pl_2()
                            .rounded_md()
                            .bg(cx.theme().muted)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "Current task · {} · {}",
                                        ago(now, task.at),
                                        task.text
                                    )),
                            )
                            .child(
                                Button::new("agent-task-clear")
                                    .ghost()
                                    .xsmall()
                                    .label("Clear")
                                    .tooltip("Drop the task; the engine goes back to watching")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.clear_operator_task("operator");
                                        cx.notify();
                                    })),
                            ),
                    )
                })
                .child(
                    Input::new(&dashboard.agent_task_input)
                        .appearance(false)
                        .aria_label("Task for the fleet"),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(engine_picker(dashboard, cx))
                        .child(div().flex_1())
                        .child(
                            Button::new("agent-talk")
                                .small()
                                .when(recording, |button| button.danger())
                                .when(!recording, |button| button.ghost())
                                .disabled(busy)
                                .label(talk_label)
                                .tooltip("Say the task (Cmd/Ctrl+Shift+V)")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_voice();
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("agent-task-send")
                                .primary()
                                .small()
                                .rounded(ButtonRounded::Size(px(999.)))
                                .label("↑")
                                .tooltip("Send the typed task (Enter)")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.send_typed_task(window, cx);
                                    cx.notify();
                                })),
                        ),
                ),
        )
        .into_any_element()
}

/// One quiet line under the composer: link status and engine activity.
fn status_footer(dashboard: &Dashboard, cx: &mut Context<Dashboard>) -> AnyElement {
    let now = dashboard.now;
    let desk = &dashboard.agent;
    let status = desk.status(now);
    let runs = desk
        .runner
        .as_ref()
        .map(|runner| format!(" · {} runs", runner.ticks))
        .unwrap_or_default();
    let text = format!("{}{runs}", status.text);
    div()
        .px_4()
        .pt_1p5()
        .pb_2()
        .w_full()
        .text_center()
        .child(if status.warn {
            caution(text, cx)
        } else {
            hint(text, cx)
        })
        .into_any_element()
}
