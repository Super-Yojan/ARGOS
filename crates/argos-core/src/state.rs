//! Fleet membership, latest telemetry, and goal acknowledgement tracking.
//! Freshness uses caller-supplied monotonic seconds, keeping this cache deterministic.

use crate::*;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug)]
pub struct RoverSnapshot {
    pub id: u64,
    pub membership: String,
    pub pose: Option<Pose>,
    pub pose_age: Option<f64>,
    pub goal: Option<GoalStatus>,
    pub goal_age: Option<f64>,
    /// Monotonic receipt time, stable until a new status packet is observed.
    pub goal_received_at: Option<f64>,
    pub command_phase: String,
    pub command_token: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub link: String,
    pub fleet_age: Option<f64>,
    pub rovers: Vec<RoverSnapshot>,
    pub notice: Option<String>,
}
#[derive(Default)]
struct Entry {
    pose: Option<(Pose, f64)>,
    goal: Option<(GoalStatus, f64)>,
    command: Option<Command>,
}
struct Command {
    token: Option<String>,
    phase: String,
    sent: f64,
    /// The goal itself, kept so a stale-authority refusal can be resent once.
    request: Option<GoalRequest>,
    retried: bool,
}
/// Terra's refusal reason when a goal carried an older authority revision than
/// the vehicle's: something else (often a repeated level click) was acknowledged
/// first. The goal itself was fine, so one resend with the new revision is safe.
pub const STALE_AUTHORITY_REASON: &str = "current_authority_required";
#[derive(Default)]
pub struct FleetCache {
    fleet: Option<(FleetState, f64)>,
    entries: BTreeMap<u64, Entry>,
    disconnected: bool,
    notice: Option<String>,
    retries: Vec<(u64, GoalRequest)>,
}
impl FleetCache {
    pub fn fleet(&mut self, state: FleetState, now: f64) {
        self.disconnected = false;
        let previous = self
            .fleet
            .as_ref()
            .map(|(fleet, _)| fleet.ids.iter().copied().collect::<BTreeSet<_>>())
            .unwrap_or_default();
        for id in &state.ids {
            let entry = self.entries.entry(*id).or_default();
            if !previous.contains(id) {
                *entry = Entry::default();
            }
        }
        // Terra retires increasing IDs. Preserve current membership plus at most
        // 32 retired rows for last-known inspection, rather than lifetime history.
        let current = state.ids.iter().copied().collect::<BTreeSet<_>>();
        let retired = self
            .entries
            .keys()
            .rev()
            .filter(|id| !current.contains(id))
            .take(32)
            .copied()
            .collect::<BTreeSet<_>>();
        self.entries
            .retain(|id, _| current.contains(id) || retired.contains(id));
        self.notice = (state.count != state.ids.len() as u64)
            .then(|| "Fleet count disagrees with membership".into());
        self.fleet = Some((state, now));
    }
    fn member(&self, id: u64) -> bool {
        self.fleet
            .as_ref()
            .is_some_and(|(fleet, _)| fleet.ids.contains(&id))
            && !self.disconnected
    }
    pub fn pose(&mut self, id: u64, pose: Pose, now: f64) {
        if self.member(id) && pose.rover_id == id {
            self.entries.entry(id).or_default().pose = Some((pose, now));
        }
    }
    pub fn status(&mut self, id: u64, status: GoalStatus, now: f64) {
        if !self.member(id) {
            return;
        }
        let entry = self.entries.entry(id).or_default();
        if let Some(command) = &mut entry.command {
            if command.phase == "cancelling"
                && now > command.sent
                && status.state == GoalState::Idle
            {
                command.phase = "cancelled".into();
            } else if command.token.is_some()
                && command.token == status.token
                && now >= command.sent
            {
                command.phase = match status.state {
                    GoalState::Idle => "superseded",
                    GoalState::Active => "active",
                    GoalState::Arrived => "arrived",
                }
                .into();
            } else if command.phase == "active" {
                command.phase = "superseded".into();
            }
        }
        entry.goal = Some((status, now));
    }
    pub fn notice(&mut self, message: String) {
        self.notice = Some(message);
    }
    pub fn disconnected(&mut self) {
        self.disconnected = true;
        self.fleet = None;
        for entry in self.entries.values_mut() {
            entry.pose = None;
            entry.goal = None;
            if let Some(command) = &mut entry.command
                && (command.phase == "pending"
                    || command.phase == "cancelling"
                    || command.phase == "active")
            {
                command.phase = "unconfirmed".into();
            }
        }
    }
    fn ready(&self, id: u64, now: f64) -> Result<()> {
        if !self.member(id)
            || self
                .fleet
                .as_ref()
                .is_none_or(|(_, at)| now - at >= SUPERVISION_STALE_SECONDS)
        {
            return Err(invalid("rover is absent, stale, or disconnected"));
        }
        Ok(())
    }
    pub fn prepare_goal(
        &mut self,
        id: u64,
        request: &GoalRequest,
        token: &str,
        now: f64,
    ) -> Result<Vec<u8>> {
        self.prepare_goal_attempt(id, request, token, now, false)
    }
    /// `retried` marks the one automatic resend after a stale-authority refusal.
    pub fn prepare_goal_attempt(
        &mut self,
        id: u64,
        request: &GoalRequest,
        token: &str,
        now: f64,
        retried: bool,
    ) -> Result<Vec<u8>> {
        self.ready(id, now)?;
        let bytes = request.encode(token)?;
        let entry = self.entries.entry(id).or_default();
        if entry.command.as_ref().is_some_and(|command| {
            (command.phase == "pending" || command.phase == "cancelling")
                && now - command.sent < GOAL_ACK_SECONDS
        }) {
            return Err(invalid("a command is awaiting acknowledgement"));
        }
        entry.command = Some(Command {
            token: Some(token.into()),
            phase: "pending".into(),
            sent: now,
            request: Some(request.clone()),
            retried,
        });
        Ok(bytes)
    }
    /// Terra refused goal `token` (seen as a rejected receipt in autonomy
    /// status). Clears the pending command at once, so the next goal is not
    /// blocked for `GOAL_ACK_SECONDS`, and queues one resend of a local goal
    /// refused only for stale authority. Returns whether it matched our goal.
    pub fn goal_refused(&mut self, id: u64, token: &str, reason: &str) -> bool {
        let Some(command) = self
            .entries
            .get_mut(&id)
            .and_then(|entry| entry.command.as_mut())
            .filter(|command| {
                command.phase == "pending" && command.token.as_deref() == Some(token)
            })
        else {
            return false;
        };
        command.phase = "refused".into();
        let retry = reason == STALE_AUTHORITY_REASON && !command.retried;
        match command.request.clone() {
            Some(request @ GoalRequest::Local { .. }) if retry => {
                self.retries.push((id, request));
                self.notice = Some(format!(
                    "Rover {id} refused the goal ({reason}) · resending once"
                ));
            }
            _ => {
                self.notice = Some(format!("Rover {id} refused the goal: {reason}"));
            }
        }
        true
    }
    /// Goals queued by `goal_refused` for their single resend.
    pub fn take_retries(&mut self) -> Vec<(u64, GoalRequest)> {
        std::mem::take(&mut self.retries)
    }
    pub fn prepare_cancel(&mut self, id: u64, now: f64) -> Result<()> {
        self.ready(id, now)?;
        let entry = self.entries.entry(id).or_default();
        if entry.command.as_ref().is_some_and(|command| {
            command.phase == "cancelling" && now - command.sent < GOAL_ACK_SECONDS
        }) {
            return Err(invalid("cancellation is awaiting acknowledgement"));
        }
        entry.command = Some(Command {
            token: None,
            phase: "cancelling".into(),
            sent: now,
            request: None,
            retried: false,
        });
        Ok(())
    }
    pub fn publish_failed(&mut self, id: u64, message: String) {
        if let Some(command) = self
            .entries
            .get_mut(&id)
            .and_then(|entry| entry.command.as_mut())
        {
            command.phase = "failed".into();
        }
        self.notice = Some(message);
    }
    pub fn snapshot(&self, now: f64) -> Snapshot {
        let fleet_age = self.fleet.as_ref().map(|(_, at)| (now - at).max(0.));
        let link = if self.disconnected {
            "disconnected"
        } else if fleet_age.is_none() {
            "connecting"
        } else if fleet_age.unwrap() >= SUPERVISION_STALE_SECONDS {
            "stale"
        } else {
            "healthy"
        };
        let rovers = self
            .entries
            .iter()
            .map(|(&id, entry)| {
                let command_phase = entry
                    .command
                    .as_ref()
                    .map(|command| {
                        if (command.phase == "pending" || command.phase == "cancelling")
                            && now - command.sent >= GOAL_ACK_SECONDS
                        {
                            "unconfirmed".into()
                        } else {
                            command.phase.clone()
                        }
                    })
                    .unwrap_or_else(|| "none".into());
                RoverSnapshot {
                    id,
                    membership: if !self.member(id) {
                        "absent"
                    } else if link == "stale" {
                        "stale"
                    } else {
                        "online"
                    }
                    .into(),
                    pose: entry.pose.as_ref().map(|(p, _)| p.clone()),
                    pose_age: entry.pose.as_ref().map(|(_, at)| (now - at).max(0.)),
                    goal: entry.goal.as_ref().map(|(g, _)| g.clone()),
                    goal_age: entry.goal.as_ref().map(|(_, at)| (now - at).max(0.)),
                    goal_received_at: entry.goal.as_ref().map(|(_, at)| *at),
                    command_phase,
                    command_token: entry
                        .command
                        .as_ref()
                        .and_then(|command| command.token.clone()),
                }
            })
            .collect();
        Snapshot {
            link: link.into(),
            fleet_age,
            rovers,
            notice: self.notice.clone(),
        }
    }
}

#[cfg(test)]
mod refusal_tests {
    use super::*;

    fn cache() -> FleetCache {
        let mut cache = FleetCache::default();
        cache.fleet(
            FleetState {
                count: 1,
                max_count: 8,
                ids: vec![0],
            },
            0.,
        );
        cache
    }
    fn local() -> GoalRequest {
        GoalRequest::Local {
            x: 4.4,
            y: 0.8,
            yaw: None,
        }
    }

    #[test]
    fn stale_authority_refusal_frees_the_slot_and_queues_one_resend() {
        let mut cache = cache();
        cache.prepare_goal(0, &local(), "goal-a", 1.).unwrap();
        // Before the fix the second goal waited GOAL_ACK_SECONDS (150 s).
        assert!(cache.prepare_goal(0, &local(), "goal-b", 2.).is_err());
        assert!(cache.goal_refused(0, "goal-a", STALE_AUTHORITY_REASON));
        let retries = cache.take_retries();
        assert_eq!(retries.len(), 1);
        assert!(cache.take_retries().is_empty());
        cache
            .prepare_goal_attempt(0, &retries[0].1, "goal-a2", 3., true)
            .unwrap();
        // The resend is refused too: no second resend, but the slot is free.
        assert!(cache.goal_refused(0, "goal-a2", STALE_AUTHORITY_REASON));
        assert!(cache.take_retries().is_empty());
        assert!(cache.prepare_goal(0, &local(), "goal-c", 4.).is_ok());
    }

    #[test]
    fn other_refusals_are_reported_but_not_resent() {
        let mut cache = cache();
        cache.prepare_goal(0, &local(), "goal-a", 1.).unwrap();
        assert!(cache.goal_refused(0, "goal-a", "authority_required"));
        assert!(cache.take_retries().is_empty());
        let notice = cache.snapshot(1.5).notice.unwrap();
        assert!(notice.contains("authority_required"), "{notice}");
        assert!(cache.prepare_goal(0, &local(), "goal-b", 2.).is_ok());
    }

    #[test]
    fn refusal_for_someone_elses_token_is_ignored() {
        let mut cache = cache();
        cache.prepare_goal(0, &local(), "goal-a", 1.).unwrap();
        assert!(!cache.goal_refused(0, "gpui-0-1", STALE_AUTHORITY_REASON));
        assert!(cache.prepare_goal(0, &local(), "goal-b", 2.).is_err());
    }
}
