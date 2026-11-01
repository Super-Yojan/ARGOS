//! Agents in the window: bridge requests, recommendation cards, and the
//! raw-drive grant. Rules and wire format are in [`argos_core::agent`].
//!
//! Every command an agent sends, and every card the operator accepts, goes
//! through the same checks as the buttons (study lock, stop, Outdoor GPS,
//! L5 support, the goal gate, hardware) and the same `FleetFeed` calls. Tokens
//! mark the source, and each step is a session-log event.
use super::{
    Dashboard, caution, gps_blocks, hint, interaction::geographic_gate, task_agent_choice,
};
use argos_core::{
    AGENT_PROTOCOL, AgentCommand, AgentEnvelope, AgentGoal, AgentMode, AgentRequest, CommandSource,
    DragAnchor, DragEnd, DragGate, DragSpace, DriveGrant, EXPLORE_BUDGETS_MIN, GoalDrag,
    PlanarPoint, REJECT_REASONS, RecAction, RecBook, RecEntry, RecState, TASK_BUDGETS_MIN,
    agent_permitted, approach_point, decode_recommendation, drive_block, drive_keys,
    explore_refusal, level_label, level_permitted, recommendation_stale, task_refusal,
};
use argos_gpui::{AGENT_PORT, AgentBridge, RoverMark};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    ActiveTheme, Selectable, Sizable, StyledExt, ThemeStyled,
    button::{Button, ButtonGroup},
    tag::Tag,
};
use gpui_kit::{prelude::*, *};
use serde_json::{Value, json};

/// How long an accepted goal card waits for the rover to reach L3.
const LEVEL_WAIT_S: f64 = 5.;
/// How long an accepted goal is watched for arrival.
const OUTCOME_WAIT_S: f64 = 300.;

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

/// What the window keeps for agents.
pub(super) struct AgentDesk {
    pub(super) mode: AgentMode,
    bridge: Option<AgentBridge>,
    bridge_note: String,
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
}

impl AgentDesk {
    /// Open the bridge unless `ARGOS_AGENT=0`. `ARGOS_AGENT_PORT` picks the port.
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
        Self {
            mode: if disabled {
                AgentMode::Off
            } else {
                AgentMode::default()
            },
            bridge,
            bridge_note,
            engine: None,
            last_call: None,
            book: RecBook::default(),
            grant: None,
            burst: None,
            rejecting: None,
            queued: Vec::new(),
            watched: Vec::new(),
        }
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
                self.stop_rover(*rover);
                self.feed.swap_source(previous);
                Ok(format!(
                    "Stop sent to rover {rover} · the operator must press Reset stop"
                ))
            }
            AgentCommand::StopAll => {
                self.end_agent_drive("stop");
                let previous = self.feed.swap_source(source);
                self.stop_all();
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
            self.note = "Take control to accept this card".into();
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
        }
    }

    fn set_agent_mode(&mut self, mode: AgentMode) {
        if mode == self.agent.mode {
            return;
        }
        if mode != AgentMode::Operator {
            self.end_agent_drive("agent mode changed");
            self.agent.queued.clear();
        }
        self.feed.record(
            json!({"kind": "agent_mode", "from": self.agent.mode.wire(), "to": mode.wire()}),
        );
        self.agent.mode = mode;
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

/// The agent panel at the lower left of the map: mode, drive grant, cards.
pub(super) fn agent_panel(dashboard: &Dashboard, cx: &mut Context<Dashboard>) -> AnyElement {
    let desk = &dashboard.agent;
    let now = dashboard.now;
    let mode = desk.mode;
    let status = match (&desk.engine, desk.last_call) {
        (Some(engine), Some(at)) => format!("{engine} · last call {:.0} s ago", now - at),
        _ if desk.bridge.is_some() => format!("{} · no agent yet", desk.bridge_note),
        _ => desk.bridge_note.clone(),
    };
    let grant = desk.grant.clone();
    let rejecting = desk.rejecting.clone();
    let cards: Vec<RecEntry> = desk.book.open().into_iter().cloned().collect();
    v_flex()
        .popover_style(cx)
        .id("agent-panel")
        .absolute()
        .bottom_3()
        .left_3()
        .w(px(340.))
        .max_h(px(520.))
        .p_2()
        .gap_2()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            h_flex()
                .gap_2()
                .child(div().text_xs().font_semibold().child("Agent"))
                .child(
                    ButtonGroup::new("agent-mode")
                        .outline()
                        .small()
                        .children(AgentMode::ALL.iter().enumerate().map(|(index, each)| {
                            Button::new(("agent-mode", index as u64))
                                .label(each.label())
                                .selected(*each == mode)
                        }))
                        .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                            if let Some(mode) = clicks.first().and_then(|index| AgentMode::ALL.get(*index)) {
                                this.set_agent_mode(*mode);
                            }
                            cx.notify();
                        })),
                ),
        )
        .child(hint(status, cx))
        .when_some(grant, |panel, grant| {
            let pending = grant.pending();
            panel.child(
                v_flex()
                    .gap_1()
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
                    ),
            )
        })
        .when(cards.is_empty() && mode != AgentMode::Off, |panel| {
            panel.child(hint("No recommendations right now.", cx))
        })
        .children(cards.into_iter().map(|entry| {
            let rec_id = entry.rec.rec_id.clone();
            let picking = rejecting.as_deref() == Some(rec_id.as_str());
            let accept_id = rec_id.clone();
            let reject_id = rec_id.clone();
            let refused = match &entry.dispatch {
                Some(Err(error)) => Some(error.clone()),
                _ => None,
            };
            v_flex()
                .gap_1()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(if entry.rec.urgency == 3 {
                    cx.theme().danger
                } else {
                    cx.theme().border
                })
                .child(
                    h_flex()
                        .gap_2()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .font_semibold()
                                .min_w_0()
                                .truncate()
                                .child(format!("R{} · {}", entry.rec.rover, entry.rec.action.title())),
                        )
                        .child(urgency_tag(entry.rec.urgency)),
                )
                .child(div().text_xs().child(entry.rec.reason.clone()))
                .when_some(refused, |card, error| card.child(caution(error, cx)))
                .child(if picking {
                    h_flex()
                        .gap_1()
                        .flex_wrap()
                        .children(REJECT_REASONS.iter().enumerate().map(|(index, (wire, label))| {
                            let rec_id = reject_id.clone();
                            Button::new(SharedString::from(format!("agent-reason-{rec_id}-{index}")))
                                .outline()
                                .xsmall()
                                .label(*label)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.reject_card(&rec_id, wire);
                                    cx.notify();
                                }))
                        }))
                        .into_any_element()
                } else {
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new(SharedString::from(format!("agent-accept-{accept_id}")))
                                .small()
                                .label(if entry.rec.action == RecAction::Attention { "Got it" } else { "Accept" })
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
        }))
        .into_any_element()
}
