//! Every command the window sends to a rover: levels, study condition,
//! explore, task, dock, goals, navigation source, teleop, stops, and 3D
//! capture. Each send runs on its own short thread and writes its result
//! to `FleetPicture::command`.

use super::config::capture_dir;
use super::feed::FleetFeed;
use super::feed::unix_millis;
use argos_core::{
    APP_UPDATE_REQUIRED, GoalRequest, NavigationEdit, OutdoorAdmission, admit_outdoor_goal,
    drive_block, encode_navigation_settings, encode_source_request, goal_blocked,
    source_change_refusal,
};
use std::path::PathBuf;

impl FleetFeed {
    pub fn publish_level(&self, id: u64, level: &str) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        let level = level.to_string();
        let source = self.token_source();
        std::thread::spawn(move || {
            let message = (|| {
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                let token = argos_core::command_token(source, id, unix_millis());
                let bytes = argos_core::encode_level_request(&level, &token)
                    .map_err(|error| error.to_string())?;
                let payload = String::from_utf8(bytes).map_err(|_| "level payload".to_string())?;
                client
                    .operator_command(id, "autonomy", &payload)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(format!(
                    "Requested {level} for rover {id} · waiting for vehicle acknowledgement"
                ))
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    /// Set (`Some`) or clear (`None`) rover `id`'s study condition. Terra then
    /// rejects every other level except L0 takeover.
    pub fn publish_assignment(&self, id: u64, level: Option<&str>) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        let level = level.map(str::to_string);
        std::thread::spawn(move || {
            let message = (|| {
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                let token = format!("gpui-study-{id}-{}", unix_millis());
                let bytes = argos_core::encode_assignment_request(level.as_deref(), &token)
                    .map_err(|error| error.to_string())?;
                let payload = String::from_utf8(bytes).map_err(|_| "level payload".to_string())?;
                client
                    .operator_command(id, "autonomy", &payload)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(match &level {
                    Some(level) => format!(
                        "Study condition {} sent to rover {id}",
                        argos_core::level_label(level)
                    ),
                    None => format!("Study condition cleared on rover {id}"),
                })
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    /// Copy the current session log to a new timestamped file next to it, with
    /// a closing snapshot record. Returns the file written.
    pub fn export_session(&self, label: &str) -> Result<PathBuf, String> {
        let guard = self.client.lock().unwrap();
        let client = guard
            .as_ref()
            .ok_or("Not connected, so there is no session log")?;
        let live = client.session_log_path();
        let stem = live
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("argos-session");
        let destination =
            live.with_file_name(format!("{stem}-{label}-export-{}.jsonl", unix_millis()));
        let text = destination.to_string_lossy().into_owned();
        client
            .export_session(&text)
            .map_err(|error| format!("Export failed: {error}"))?;
        Ok(destination)
    }
    /// Start L4 frontier exploration for `budget_seconds`. Terra picks and
    /// drives the frontiers; this only sends the level with its budget.
    pub fn publish_explore(&self, id: u64, budget_seconds: f64) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        let source = self.token_source();
        std::thread::spawn(move || {
            let message = (|| {
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                let token = argos_core::command_token(source, id, unix_millis());
                let bytes = argos_core::encode_explore_request(&token, budget_seconds)
                    .map_err(|error| error.to_string())?;
                let payload = String::from_utf8(bytes).map_err(|_| "level payload".to_string())?;
                client
                    .operator_command(id, "autonomy", &payload)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(format!(
                    "Requested L4 Explore for {:.0} min on rover {id} · waiting for vehicle acknowledgement",
                    budget_seconds / 60.
                ))
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    /// Select L5 and give the task in one message. Terra's phone model picks the
    /// waypoints; Terra checks and drives them.
    pub fn publish_task(
        &self,
        id: u64,
        instruction: String,
        budget_seconds: f64,
        agent: argos_core::TaskAgent,
    ) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        let source = self.token_source();
        std::thread::spawn(move || {
            let message = (|| {
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                let stamp = unix_millis();
                let token = argos_core::command_token(source, id, stamp);
                let task_id = format!("task-{id}-{stamp}");
                let bytes = argos_core::encode_task_level_request(
                    &token,
                    &task_id,
                    &instruction,
                    budget_seconds,
                    agent,
                )
                .map_err(|error| error.to_string())?;
                let payload = String::from_utf8(bytes).map_err(|_| "task payload".to_string())?;
                client
                    .operator_command(id, "autonomy", &payload)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(format!(
                    "Sent L5 task to rover {id} · waiting for vehicle acknowledgement"
                ))
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    /// Reply to the model's question, or add guidance, on `task/action`.
    pub fn publish_task_reply(&self, id: u64, task_id: String, text: String) {
        self.task_action(
            id,
            move |token| argos_core::encode_task_reply(&task_id, &text, token),
            "Reply sent to the rover's model",
        );
    }
    /// Approve or deny the agent's raw-drive request. Deny also ends a running grant.
    pub fn publish_task_drive(&self, id: u64, task_id: String, approve: bool) {
        self.task_action(
            id,
            move |token| argos_core::encode_task_drive(&task_id, approve, token),
            if approve {
                "Raw drive approved for the agent"
            } else {
                "Raw drive denied · the rover stops"
            },
        );
    }
    /// Cancel the task. The rover holds at L5.
    pub fn cancel_task(&self, id: u64, task_id: String) {
        self.task_action(
            id,
            move |token| argos_core::encode_task_cancel(&task_id, token),
            "Task cancel requested",
        );
    }
    /// Line the rover up over found bucket `object_id`.
    pub fn start_dock(&self, id: u64, object_id: u32) {
        self.operator_action(
            id,
            "dock",
            move |token| argos_core::encode_dock_start(object_id, token),
            "Dock requested · Terra lines up over the bucket",
        );
    }
    /// Approve the dump: Terra opens the gate.
    pub fn approve_dump(&self, id: u64, dock_id: u64) {
        self.operator_action(
            id,
            "dock",
            move |token| argos_core::encode_dock_dump(dock_id, token),
            "Dump approved · the gate opens",
        );
    }
    /// End the dock. An open gate closes.
    pub fn cancel_dock(&self, id: u64) {
        self.operator_action(
            id,
            "dock",
            argos_core::encode_dock_cancel,
            "Dock cancel requested",
        );
    }
    fn task_action(
        &self,
        id: u64,
        encode: impl FnOnce(&str) -> argos_core::Result<Vec<u8>> + Send + 'static,
        done: &'static str,
    ) {
        self.operator_action(id, "task/action", encode, done);
    }
    fn operator_action(
        &self,
        id: u64,
        kind: &'static str,
        encode: impl FnOnce(&str) -> argos_core::Result<Vec<u8>> + Send + 'static,
        done: &'static str,
    ) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        let source = self.token_source();
        std::thread::spawn(move || {
            let message = (|| {
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                let token = argos_core::command_token(source, id, unix_millis());
                let bytes = encode(&token).map_err(|error| error.to_string())?;
                let payload = String::from_utf8(bytes).map_err(|_| format!("{kind} payload"))?;
                client
                    .operator_command(id, kind, &payload)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(done.to_string())
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    /// Publish a confirmed goal. A geographic goal is admitted only while outdoor
    /// readiness still matches the reviewed run and navigation revision.
    pub fn publish_goal(&self, id: u64, goal: GoalRequest, arrival_radius_m: Option<f64>) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        std::thread::spawn(move || {
            let message = (|| {
                let mut review = None;
                {
                    let current = picture.lock().unwrap();
                    if current.link != "healthy" {
                        return Err("Link is down".to_string());
                    }
                    let mark = current
                        .marks
                        .iter()
                        .find(|mark| mark.id == id)
                        .ok_or("Select a rover that is in the fleet")?;
                    // Terra refuses every goal while a safety stop is held.
                    if mark.safety == "emergency_stop" {
                        return Err(format!("Rover {id}: {}", argos_core::GOAL_WHILE_STOPPED));
                    }
                    if let GoalRequest::Geographic {
                        latitude,
                        longitude,
                        ..
                    } = goal
                    {
                        if mark.app_update_required {
                            return Err(APP_UPDATE_REQUIRED.into());
                        }
                        let level = mark
                            .effective_level
                            .as_deref()
                            .or(mark.requested_level.as_deref());
                        admit_outdoor_goal(OutdoorAdmission {
                            report: mark.outdoor.as_ref(),
                            latitude,
                            longitude,
                            arrival_radius_m: arrival_radius_m.unwrap_or(mark.arrival_radius),
                            level,
                            run_id: &mark.run_id,
                            navigation_revision: mark.navigation_revision,
                        })
                        .map_err(str::to_string)?;
                    }
                    if matches!(goal, GoalRequest::Geographic { .. }) {
                        review = Some((
                            mark.run_id.clone(),
                            mark.navigation_revision,
                            arrival_radius_m.unwrap_or(mark.arrival_radius),
                        ));
                    }
                    if let Some(reason) = goal_blocked(mark.degraded) {
                        return Err(format!("{reason} ({})", mark.degrade_reason));
                    }
                    if let Some(reason) = drive_block(&mark.hardware) {
                        return Err(reason.into());
                    }
                }
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                client
                    .send_reviewed_goal(id, goal, review)
                    .map_err(|error| error.to_string())?;
                Ok("Goal sent".to_string())
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    pub fn select_source(&self, id: u64, source: &str) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        let source = source.to_string();
        std::thread::spawn(move || {
            let message = (|| {
                let (run_id, revision, ready_and_neutral, goal_active, mission_active, outdoor) = {
                    let current = picture.lock().unwrap();
                    let mark = current
                        .marks
                        .iter()
                        .find(|mark| mark.id == id)
                        .ok_or("Select a rover that is in the fleet")?;
                    (
                        mark.run_id.clone(),
                        mark.navigation_revision,
                        mark.hardware.ready_and_neutral(),
                        mark.goal_active,
                        mark.mission_active,
                        mark.outdoor.clone(),
                    )
                };
                if let Some(reason) = source_change_refusal(
                    ready_and_neutral,
                    goal_active,
                    mission_active,
                    &source,
                    outdoor.as_ref(),
                ) {
                    return Err(reason.to_string());
                }
                let token = format!("src-{id}-{}", unix_millis());
                let bytes = encode_source_request(&source, &token, &run_id, revision)
                    .map_err(|error| error.to_string())?;
                let payload =
                    String::from_utf8(bytes).map_err(|_| "navigation source".to_string())?;
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                client
                    .operator_command(id, "navigation/source", &payload)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(format!("Requested {source} navigation"))
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    /// Ask Terra for new planner settings, always with CrabTrapBot's body
    /// size. Terra applies them only with neutral hardware, no active goal,
    /// and current authority. `done` is the note shown once sent.
    pub fn configure_navigation(&self, id: u64, edit: NavigationEdit, done: String) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        std::thread::spawn(move || {
            let message = (|| {
                let (navigation, run_id, authority_revision) = {
                    let current = picture.lock().unwrap();
                    let mark = current
                        .marks
                        .iter()
                        .find(|mark| mark.id == id)
                        .ok_or("Select a rover that is in the fleet")?;
                    if !mark.hardware.ready_and_neutral() {
                        return Err("Settings need ready hardware with neutral output".into());
                    }
                    if mark.goal_active {
                        return Err("Cancel the active goal before changing settings".into());
                    }
                    (
                        mark.navigation_settings.clone(),
                        mark.run_id.clone(),
                        mark.authority_revision,
                    )
                };
                let token = format!("nav-{id}-{}", unix_millis());
                let bytes = encode_navigation_settings(
                    &navigation,
                    edit,
                    &token,
                    &run_id,
                    authority_revision,
                )
                .map_err(|error| error.to_string())?;
                let payload =
                    String::from_utf8(bytes).map_err(|_| "navigation settings".to_string())?;
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                client
                    .operator_command(id, "navigation/settings", &payload)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(done)
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    pub fn resume_outdoor(&self, id: u64) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let current = picture.lock().unwrap();
                let mark = current
                    .marks
                    .iter()
                    .find(|m| m.id == id)
                    .ok_or("Rover unavailable")?;
                let payload=serde_json::json!({"schema_version":1,"token":format!("resume-{id}-{}",unix_millis()),"run_id":mark.run_id,"navigation_revision":mark.navigation_revision}).to_string();
                drop(current);
                let guard = client.lock().unwrap();
                let c = guard.as_ref().ok_or("Not connected")?;
                c.operator_command(id, "navigation/resume", &payload)
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>("Outdoor resume requested".into())
            })();
            picture.lock().unwrap().command = result.unwrap_or_else(|e| e);
        });
    }
    pub fn cancel_goal(&self, id: u64) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        std::thread::spawn(move || {
            let message = (|| {
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                client.cancel_goal(id).map_err(|error| error.to_string())?;
                Ok::<_, String>(format!("Cancel sent to rover {id}"))
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    pub fn begin_drive(&self, id: u64) -> Result<(), String> {
        {
            let current = self.picture.lock().unwrap();
            if current.link != "healthy" {
                return Err("Link is down".into());
            }
            let mark = current
                .marks
                .iter()
                .find(|mark| mark.id == id)
                .ok_or("Rover is not in the fleet")?;
            if !mark.control_pose_ready() {
                return Err("Pose is stale".into());
            }
            if let Some(reason) = drive_block(&mark.hardware) {
                return Err(reason.into());
            }
        }
        let client = self.connected()?;
        client.set_teleop_guard(*self.limits.lock().unwrap());
        Ok(())
    }
    pub fn end_drive(&self) {
        if let Ok(client) = self.connected() {
            client.clear_input();
            client.clear_teleop_guard();
        }
    }
    pub fn drive_key(&self, id: u64, key: &str, down: bool) -> Result<(), String> {
        if down {
            let current = self.picture.lock().unwrap();
            if current.link != "healthy" {
                return Err("Link is down".into());
            }
            let mark = current
                .marks
                .iter()
                .find(|mark| mark.id == id)
                .ok_or("Rover is not in the fleet")?;
            if !mark.control_pose_ready() {
                return Err("Pose is stale".into());
            }
            if let Some(reason) = drive_block(&mark.hardware) {
                return Err(reason.into());
            }
        }
        let client = self.connected()?;
        client
            .input(id, key, down)
            .map_err(|error| error.to_string())
    }
    pub fn refresh_drive(&self, id: u64) {
        if let Ok(client) = self.connected() {
            client.refresh_drive(id);
        }
    }
    pub fn safety_reset(&self, id: u64) -> Result<(), String> {
        let client = self.connected()?;
        client
            .action(id, "safety", serde_json::json!({"action": "reset"}))
            .map_err(|error| error.to_string())?;
        self.picture.lock().unwrap().command = format!("Reset sent to rover {id}");
        Ok(())
    }
    /// Safety stop one rover. `origin` names the control that sent it
    /// (see `console::StopOrigin::name`); the session log gets a
    /// `stop_origin` record with `sent`, and the `action_sent` token when it
    /// went out. With no client connected there is no log to write.
    pub fn emergency_stop(&self, id: u64, origin: &str) -> Result<(), String> {
        let client = self.connected()?;
        client.clear_teleop_guard();
        let sent = client
            .action(id, "safety", serde_json::json!({"action": "stop"}))
            .map_err(|error| error.to_string());
        // A failed send is logged too, so the record never claims a stop the
        // rover did not get. `sent: false` has no token.
        client.record_event(serde_json::json!({
            "kind": "stop_origin",
            "rover_id": id,
            "origin": origin,
            "sent": sent.is_ok(),
            "token": sent.as_ref().ok(),
            "error": sent.as_ref().err(),
        }));
        sent?;
        self.picture.lock().unwrap().command = format!("Stop sent to rover {id}");
        Ok(())
    }
    /// Ask the rover for a full labelled point capture (10 s at 1 Hz) and save
    /// it as PLY under [`capture_dir`] once it is complete.
    pub fn capture_3d(&self, id: u64) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        std::thread::spawn(move || {
            let message = (|| {
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                // The rover runs one capture at a time; a recording replaces live.
                let _ = client.stop_live_points(id);
                client
                    .start_point_capture(id, capture_dir())
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(format!("Capturing 3D from rover {id} · 10 s at 1 Hz"))
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    /// Turn rover `id`'s live labelled point stream on or off.
    pub fn set_live_points(&self, id: u64, on: bool) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        std::thread::spawn(move || {
            let message = (|| {
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                if on {
                    client
                        .start_live_points(id)
                        .map_err(|error| error.to_string())?;
                    Ok::<_, String>(format!(
                        "Live 3D from rover {id} · {} Hz, every {}th pixel",
                        argos_zenoh::LIVE_RATE_HZ,
                        argos_zenoh::LIVE_STRIDE
                    ))
                } else {
                    client
                        .stop_live_points(id)
                        .map_err(|error| error.to_string())?;
                    Ok(format!("Live 3D from rover {id} stopped"))
                }
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    /// Rover `id`'s live point stream, while one is wanted.
    pub fn live_points(&self, id: u64) -> Option<argos_zenoh::LiveStatus> {
        self.connected().ok()?.live_points(id)
    }
    /// The rover's latest point capture; also finishes and saves a complete one.
    pub fn point_capture(&self, id: u64) -> Option<argos_zenoh::CaptureStatus> {
        self.connected().ok()?.point_capture(id)
    }
}
