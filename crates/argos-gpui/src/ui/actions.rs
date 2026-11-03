//! What the operator's buttons and keys do: select, stop, drive, study
//! condition, L4 explore and L5 task. Every send goes through `FleetFeed`.

use super::levels::STUDY_SENDS;
use super::levels::gps_blocks;
use super::task::task_agent_choice;
use crate::Dashboard;

use argos_core::{
    EXPLORE_BUDGETS_MIN, TASK_BUDGETS_MIN, explore_refusal, level_label, level_permitted,
    task_refusal,
};
use argos_gpui::RoverMark;
use argos_gpui::console::{self, StopCause, StopOrigin};
use gpui_kit::{prelude::*, *};

impl Dashboard {
    /// The condition that locks the level picker: this window's choice, else
    /// what the rover reports.
    pub(super) fn study_lock<'a>(&self, mark: &'a RoverMark) -> Option<&'a str> {
        match self.study {
            Some(want) => want,
            None => mark.assigned_level.as_deref(),
        }
    }
    /// True once a rover has ignored every resend of the condition, which an
    /// older Terra build does.
    pub(super) fn study_refused(&self, mark: &RoverMark) -> bool {
        self.study
            .is_some_and(|want| mark.assigned_level.as_deref() != want)
            && self
                .study_sends
                .get(&mark.id)
                .is_some_and(|(at, count)| *count >= STUDY_SENDS && self.now - at >= 2.)
    }
    pub(super) fn set_study(&mut self, level: Option<&'static str>) {
        self.study = Some(level);
        self.study_sends.clear();
        self.note = match level {
            Some(level) => format!(
                "Study condition: every rover fixed at {} · L0 takeover and Stop stay available",
                level_label(level)
            ),
            None => "Study condition off · every level is available".into(),
        };
        self.sync_study();
    }
    pub(super) fn select(&mut self, id: u64) {
        if !self.marks.iter().any(|mark| mark.id == id) {
            return;
        }
        if self.selected != Some(id) {
            self.stop_drive();
            self.drag = None;
            self.selected = Some(id);
        }
        self.selection_hold.clear();
        self.claim.take(self.now);
        self.note = format!("Rover {id} selected · control active in this window");
    }
    pub(super) fn reset_safety(&mut self, id: u64) {
        self.stop_drive();
        match self.feed.safety_reset(id) {
            Ok(()) => {
                self.latched.remove(&id);
                self.note = format!("Reset stop sent to rover {id}");
            }
            Err(error) => self.note = error,
        }
    }
    pub(super) fn stop_rover(&mut self, id: u64, origin: StopOrigin) {
        self.end_agent_drive("stop");
        self.stop_drive();
        self.drag = None;
        self.latched.insert(id, (origin, self.now));
        self.note = match self.feed.emergency_stop(id, origin.name()) {
            Ok(()) => format!("Stop sent to rover {id}"),
            Err(error) => error,
        };
    }
    pub(super) fn begin_drive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.end_agent_drive("operator took the wheel");
        let Some(id) = self.selected else {
            self.note = "Select a rover before driving".into();
            return;
        };
        if self.latched.contains_key(&id)
            || self
                .selected_mark()
                .is_some_and(|mark| mark.safety == "emergency_stop")
        {
            self.note = "Stop is active · use Reset stop before driving".into();
            return;
        }
        self.drag = None;
        // Selecting manual control does not send motor commands.
        if self
            .selected_mark()
            .is_some_and(|mark| mark.requested_level.as_deref() != Some("teleop"))
        {
            self.feed.publish_level(id, "teleop");
            self.note = "Selecting manual control · waiting for L0 confirmation".into();
            return;
        }
        match self.feed.begin_drive(id) {
            Ok(()) => {
                self.driving = true;
                self.note = "Drive is manual. In-place spin is off.".into();
                self.focus.focus(window, cx);
            }
            Err(error) => self.note = error,
        }
    }
    pub(super) fn stop_drive(&mut self) {
        if let Some(id) = self.selected {
            for key in ["w", "a", "s", "d"] {
                let _ = self.feed.drive_key(id, key, false);
            }
        }
        self.feed.end_drive();
        self.driving = false;
        self.held.clear();
    }
    pub(super) fn estop(&mut self) {
        if let Some(id) = self.selected {
            self.stop_rover(id, StopOrigin::Space);
        } else {
            self.stop_drive();
        }
    }
    /// Safety stop for every fleet member, located or not. Each rover then
    /// needs its own Reset stop; nothing here re-arms or resumes.
    pub(super) fn stop_all(&mut self, origin: StopOrigin) {
        self.end_agent_drive("stop");
        self.stop_drive();
        self.drag = None;
        let ids = console::fleet_ids(&self.marks, &self.presence);
        let mut failed = Vec::new();
        for id in &ids {
            self.latched.insert(*id, (origin, self.now));
            if let Err(error) = self.feed.emergency_stop(*id, origin.name()) {
                failed.push(format!("rover {id} ({error})"));
            }
        }
        self.note = if ids.is_empty() {
            "No rovers in the fleet to stop".into()
        } else if failed.is_empty() {
            format!("Stop sent to all {} rovers", ids.len())
        } else {
            format!("Stop not sent to {}", failed.join(", "))
        };
    }
    pub(super) fn is_stopped(&self, mark: &RoverMark) -> bool {
        self.latched.contains_key(&mark.id) || mark.safety == "emergency_stop"
    }
    /// Who stopped this rover, if it is stopped.
    pub(super) fn stop_cause(&self, mark: &RoverMark) -> Option<StopCause> {
        console::stop_cause(mark, self.latched.get(&mark.id).copied(), self.now)
    }
    pub(super) fn start_explore(&mut self, id: u64) {
        let Some(mark) = self.marks.iter().find(|mark| mark.id == id) else {
            self.note = "Select a rover that is in the fleet".into();
            return;
        };
        if !level_permitted(self.study_lock(mark), "explore") {
            self.note = "The study condition locks L4 in this trial".into();
            return;
        }
        if let Some(reason) = explore_refusal(self.is_stopped(mark), gps_blocks(mark, "explore")) {
            self.note = reason.into();
            return;
        }
        let minutes = EXPLORE_BUDGETS_MIN
            .get(self.explore_budget)
            .copied()
            .unwrap_or(EXPLORE_BUDGETS_MIN[0]);
        self.stop_drive();
        self.feed.publish_explore(id, f64::from(minutes) * 60.);
        self.note = format!("Sending L4 Explore · {minutes} min");
    }
    pub(super) fn start_task(&mut self, id: u64, cx: &App) {
        let Some(mark) = self.marks.iter().find(|mark| mark.id == id) else {
            self.note = "Select a rover that is in the fleet".into();
            return;
        };
        if !level_permitted(self.study_lock(mark), "task") {
            self.note = "The study condition locks L5 in this trial".into();
            return;
        }
        if let Some(reason) = task_refusal(
            self.is_stopped(mark),
            gps_blocks(mark, "task"),
            mark.task_supported,
        ) {
            self.note = reason.into();
            return;
        }
        let text = self.task_input.read(cx).value().to_string();
        let instruction = match argos_core::task_text(&text) {
            Ok(text) => text,
            Err(error) => {
                self.note = error.0;
                return;
            }
        };
        let minutes = TASK_BUDGETS_MIN
            .get(self.task_budget)
            .copied()
            .unwrap_or(TASK_BUDGETS_MIN[0]);
        let agent = task_agent_choice(self.task_agent, mark);
        self.stop_drive();
        self.feed
            .publish_task(id, instruction, f64::from(minutes) * 60., agent);
        self.note = format!("Sending L5 task to the {} · {minutes} min", agent.label());
    }
    pub(super) fn reply_task(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task_id) = self
            .marks
            .iter()
            .find(|mark| mark.id == id)
            .and_then(|mark| mark.task.as_ref())
            .filter(|task| task.running())
            .map(|task| task.task_id.clone())
        else {
            self.note = "No running task to reply to".into();
            return;
        };
        let text = self.task_reply.read(cx).value().to_string();
        match argos_core::task_text(&text) {
            Ok(text) => {
                self.feed.publish_task_reply(id, task_id, text);
                self.task_reply
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.note = "Sending reply to the rover's model".into();
            }
            Err(error) => self.note = error.0,
        }
    }
    pub(super) fn hold(&mut self, key: &str, down: bool) {
        let Some(id) = self.selected else {
            return;
        };
        if !self.driving {
            if down {
                self.note = "Click Drive this rover, then hold W/A/S/D in this window".into();
            }
            return;
        }
        match self.feed.drive_key(id, key, down) {
            Ok(()) => {
                if down {
                    self.held.insert(key.to_string());
                    self.feed.refresh_drive(id);
                } else {
                    self.held.remove(key);
                }
                // Clearing the note would show the last background command
                // instead, such as an old "Cancel sent", while driving.
                self.note = if self.held.is_empty() {
                    format!("Driving rover {id} · hold W/A/S/D")
                } else {
                    let keys: Vec<_> = self.held.iter().map(|key| key.to_uppercase()).collect();
                    format!("Driving rover {id} · {}", keys.join(" "))
                };
            }
            Err(error) => {
                self.held.remove(key);
                self.note = error;
            }
        }
    }
}
