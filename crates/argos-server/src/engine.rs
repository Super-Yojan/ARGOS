//! Single-owner command state machine. Time is caller-supplied monotonic seconds.
use crate::types::*;
use std::collections::BTreeMap;

pub trait Fleet: Send + 'static {
    fn snapshot(&mut self, now: f64) -> FleetSnapshot;
    fn emergency_stop(&mut self, _id: &str) -> Result<(), String> {
        Err("Emergency stop unavailable".into())
    }
    fn drive(&mut self, _id: &str, _request: &DriveRequest, _session: &str) -> Result<(), String> {
        Err("Manual driving unavailable".into())
    }

    fn navigation(
        &mut self,
        _id: &str,
        _request: &NavigationRequest,
        _token: &str,
    ) -> Result<(), String> {
        Err("Terra does not support remote navigation settings".into())
    }
    fn occupancy(&mut self, _id: &str, _now: f64) -> Result<Option<Occupancy>, String> {
        Ok(None)
    }
    fn select_mode(&mut self, id: &str, level: &str, token: &str) -> Result<(), String>;
    fn send_goal(&mut self, id: &str, goal: &Waypoint) -> Result<String, String>;
    fn cancel(&mut self, id: &str) -> Result<(), String>;
    fn hardware(
        &mut self,
        _id: &str,
        _action: &str,
        _token: &str,
        _run: &str,
        _revision: &str,
    ) -> Result<(), String> {
        Err("Hardware control unavailable".into())
    }
}
type Journal = Box<dyn Fn(&Operation) -> Result<(), String> + Send>;
pub struct Engine<F: Fleet> {
    pub fleet: F,
    pub operations: BTreeMap<String, Operation>,
    pub claim: Option<Claim>,
    journal: Journal,
    held: BTreeMap<String, (String, DriveRequest, f64)>,
    drive_order: BTreeMap<(String, String), u64>,
    drive_session: String,
    wire_sequence: u64,
}
impl<F: Fleet> Engine<F> {
    pub fn new(fleet: F) -> Self {
        Self {
            fleet,
            operations: BTreeMap::new(),
            claim: None,
            journal: Box::new(|_| Ok(())),
            wire_sequence: 0,
            held: BTreeMap::new(),
            drive_order: BTreeMap::new(),
            drive_session: uuid::Uuid::new_v4().to_string(),
        }
    }
    pub fn journal(&mut self, journal: Journal) {
        self.journal = journal;
    }
    fn save(&mut self, op: Operation) -> Result<Operation, String> {
        (self.journal)(&op)?;
        self.operations.insert(op.id.clone(), op.clone());
        Ok(op)
    }
    fn drive_ready(s: &FleetSnapshot, id: &str, request: &DriveRequest) -> bool {
        let recent = |v: f64| v.is_finite() && (0.0..0.5).contains(&v);
        s.link == "healthy"
            && s.rovers.iter().any(|r| {
                r.id == id
                    && r.ready()
                    && r.pose_age.is_some_and(recent)
                    && r.hardware.as_ref().is_some_and(|h| recent(h.age))
                    && r.authority.as_ref().is_some_and(|a| {
                        recent(a.age)
                            && a.run_id == request.run_id
                            && a.revision == request.revision
                            && ["teleop", "assisted_teleop"].contains(&a.requested_level.as_str())
                    })
            })
    }
    fn stop_drive(&mut self, id: &str) {
        if let Some((_, mut request, _)) = self.held.remove(id) {
            request.linear = 0.;
            request.angular = 0.;
            self.wire_sequence += 1;
            request.sequence = self.wire_sequence;
            if let Err(e) = self.fleet.drive(id, &request, &self.drive_session) {
                tracing::warn!(error=%e,"neutral publication failed; vehicle deadman remains required");
            }
        }
    }
    pub fn drive(
        &mut self,
        user: &Identity,
        owner: &str,
        id: &str,
        mut request: DriveRequest,
        now: f64,
    ) -> Result<(), String> {
        self.allowed(user, owner, now)?;
        let key = (owner.to_string(), id.to_string());
        if self
            .drive_order
            .get(&key)
            .is_some_and(|s| *s >= request.sequence)
        {
            return Err("Outdated drive input".into());
        }
        self.drive_order.insert(key, request.sequence);
        let moving = request.linear != 0. || request.angular != 0.;
        if !request.linear.is_finite()
            || !request.angular.is_finite()
            || request.linear.abs() > 0.5
            || request.angular.abs() > 1.
        {
            self.stop_drive(id);
            return Err("Invalid drive velocity".into());
        }
        if moving
            && (!request.observed_at.is_finite()
                || !(0.0..0.5).contains(&(now - request.observed_at))
                || !Self::drive_ready(&self.fleet.snapshot(now), id, &request))
        {
            self.stop_drive(id);
            return Err("Fresh armed manual authority required".into());
        }
        if moving && self.held.get(id).is_some_and(|(_, _, at)| now - at < 0.09) {
            return Err("Drive rate exceeded".into());
        }
        if moving {
            self.held
                .insert(id.into(), (owner.into(), request.clone(), now));
        } else {
            self.held.remove(id);
        }
        self.wire_sequence += 1;
        request.sequence = self.wire_sequence;
        if let Err(e) = self.fleet.drive(id, &request, &self.drive_session) {
            self.stop_drive(id);
            return Err(e);
        }
        Ok(())
    }
    pub fn emergency_stop(
        &mut self,
        user: &Identity,
        owner: &str,
        id: &str,
        now: f64,
    ) -> Result<(), String> {
        self.allowed(user, owner, now)?;
        let stop = self.fleet.emergency_stop(id);
        self.release(owner);
        // Retire pending commands before attempting all stop paths, even if a publication fails.
        for op in self
            .operations
            .values_mut()
            .filter(|o| o.rover_id == id && !o.terminal())
        {
            op.phase = "interrupted".into();
            op.message = "Emergency stop requested".into();
        }
        let cancel = self.fleet.cancel(id);
        let disarm = self
            .fleet
            .hardware(id, "disarm", &uuid::Uuid::new_v4().to_string(), "", "0");
        let result = stop.and(cancel).and(disarm);
        let operation = Operation {
            id: uuid::Uuid::new_v4().to_string(),
            request_id: uuid::Uuid::new_v4().to_string(),
            user_id: user.id.clone(),
            owner: owner.into(),
            rover_id: id.into(),
            kind: "stop".into(),
            waypoint: None,
            phase: "unconfirmed".into(),
            message: match &result {
                Ok(()) => {
                    "Published safety stop, cancel and disarm; verify vehicle acknowledgement"
                        .into()
                }
                Err(e) => format!("Emergency stop outcome uncertain: {e}"),
            },
            mode_token: None,
            goal_token: None,
            run_id: String::new(),
            goal_observation: None,
            started: now,
            created_at: crate::auth::epoch() as f64,
            mode_level: String::new(),
            revision: String::new(),
            frame_id: None,
            settings: None,
            settings_revision: None,
        };
        if let Err(e) = self.save(operation.clone()) {
            self.operations.insert(operation.id.clone(), operation);
            return Err(format!(
                "Stop attempted; audit storage failed: {e}. Verify vehicle status"
            ));
        }
        result
    }
    pub fn claim(&mut self, user: &Identity, owner: &str, now: f64) -> Result<Claim, String> {
        if user.role != "operator" {
            return Err("Operator role required".into());
        }
        if self
            .claim
            .as_ref()
            .is_some_and(|c| c.expires_at > now && c.owner != owner)
        {
            return Err("Another operator holds control".into());
        }
        if let Some(expired) = self
            .claim
            .as_ref()
            .filter(|c| c.expires_at <= now)
            .map(|c| c.owner.clone())
        {
            self.release(&expired);
        }
        let claim = Claim {
            owner: owner.into(),
            login: user.login.clone(),
            expires_at: now + 30.,
        };
        self.claim = Some(claim.clone());
        Ok(claim)
    }
    pub fn release(&mut self, owner: &str) {
        if self.claim.as_ref().is_some_and(|c| c.owner == owner) {
            self.claim = None;
            let ids: Vec<_> = self.held.keys().cloned().collect();
            for id in ids {
                self.stop_drive(&id);
            }
            let pending = self
                .operations
                .values()
                .filter(|o| {
                    o.owner == owner
                        && ["queued", "ready", "mode_pending"].contains(&o.phase.as_str())
                })
                .cloned()
                .collect::<Vec<_>>();
            for mut op in pending {
                op.phase = "interrupted".into();
                op.message = "Control released or expired; unpublished goal discarded".into();
                if let Err(e) = self.save(op.clone()) {
                    tracing::error!(error=%e,"release audit failed");
                    self.operations.insert(op.id.clone(), op);
                }
            }
        }
    }
    fn allowed(&self, user: &Identity, owner: &str, now: f64) -> Result<(), String> {
        if user.role != "operator"
            || !self
                .claim
                .as_ref()
                .is_some_and(|c| c.owner == owner && c.expires_at > now)
        {
            return Err("Current operator control required".into());
        }
        Ok(())
    }
    fn duplicate(
        &self,
        user: &Identity,
        id: &str,
        request: &str,
        kind: &str,
        waypoint: &Option<Waypoint>,
    ) -> Result<Option<Operation>, String> {
        if uuid::Uuid::parse_str(request).is_err() {
            return Err("request_id must be a UUID".into());
        }
        if let Some(op) = self.operations.values().find(|o| o.request_id == request) {
            if op.user_id != user.id
                || op.rover_id != id
                || op.kind != kind
                || &op.waypoint != waypoint
            {
                return Err("Request ID already used for a different operation".into());
            }
            return Ok(Some(op.clone()));
        }
        Ok(None)
    }
    pub fn submit(
        &mut self,
        user: &Identity,
        owner: &str,
        id: &str,
        request: WaypointRequest,
        now: f64,
    ) -> Result<Operation, String> {
        self.allowed(user, owner, now)?;
        if let Some(op) = self.duplicate(
            user,
            id,
            &request.request_id,
            "waypoint",
            &Some(request.waypoint.clone()),
        )? {
            if op.run_id != request.run_id || op.frame_id != request.frame_id {
                return Err("Request ID already used for another rover frame".into());
            }
            return Ok(op);
        }
        if matches!(request.waypoint, Waypoint::Geographic { .. }) {
            return Err("GPS is for observation only; send waypoints in local coordinates".into());
        }
        request
            .waypoint
            .core()
            .encode("validate")
            .map_err(|e| e.to_string())?;
        let snap = self.fleet.snapshot(now);
        let rover = snap
            .rovers
            .iter()
            .find(|r| r.id == id)
            .ok_or("Unknown rover")?;
        if snap.link != "healthy" || !rover.ready() {
            return Err("Fresh pose, authority and armed, ready hardware required".into());
        }
        if rover.frame_id != request.frame_id
            || !rover
                .authority
                .as_ref()
                .is_some_and(|a| a.run_id == request.run_id)
        {
            return Err("Rover run or coordinate frame changed; review a new waypoint".into());
        }
        if matches!(request.waypoint, Waypoint::Geographic { .. }) && rover.reference.is_none() {
            return Err("Usable geographic reference required".into());
        }
        if self
            .operations
            .values()
            .any(|o| o.rover_id == id && !o.terminal())
        {
            return Err("Rover already has an active operation; cancel it first".into());
        }
        let a = rover.authority.as_ref().ok_or("Missing authority")?;
        let level = if a.requested_level == "waypoint_direct" {
            "waypoint_direct"
        } else {
            "waypoint"
        };
        if !a.supported_levels.iter().any(|s| s == level) {
            return Err("Waypoint mode not supported".into());
        }
        let op = Operation {
            settings: None,
            settings_revision: None,
            id: uuid::Uuid::new_v4().to_string(),
            request_id: request.request_id,
            user_id: user.id.clone(),
            owner: owner.into(),
            rover_id: id.into(),
            kind: "waypoint".into(),
            waypoint: Some(request.waypoint),
            phase: "queued".into(),
            message: "Waiting to request waypoint authority".into(),
            mode_token: None,
            goal_token: None,
            run_id: a.run_id.clone(),
            goal_observation: None,
            started: now,
            created_at: crate::auth::epoch() as f64,
            mode_level: level.into(),
            revision: a.revision.clone(),
            frame_id: rover.frame_id.clone(),
        };
        let op = self.save(op)?;
        // Requesting mode is not goal publication. Persist the intent before sending.
        let mut next = op.clone();
        next.mode_token = Some(uuid::Uuid::new_v4().to_string());
        next.phase = if a.requested_level == level {
            "ready"
        } else {
            "mode_pending"
        }
        .into();
        next.message = "Waiting for matching waypoint authority".into();
        let next = self.save(next)?;
        if next.phase == "mode_pending"
            && let Err(e) =
                self.fleet
                    .select_mode(id, level, next.mode_token.as_deref().unwrap_or_default())
        {
            let mut failed = next;
            failed.phase = "rejected".into();
            failed.message = e;
            return self.save(failed);
        }
        Ok(next)
    }
    pub fn navigation(
        &mut self,
        user: &Identity,
        owner: &str,
        id: &str,
        request: NavigationRequest,
        now: f64,
    ) -> Result<Operation, String> {
        let began = std::time::Instant::now();
        self.allowed(user, owner, now)?;
        if !request.parameters.valid() {
            return Err("Invalid vehicle dimensions or avoidance limits".into());
        }
        if let Some(op) = self.duplicate(user, id, &request.request_id, "navigation", &None)? {
            if op.run_id != request.run_id
                || op.revision != request.revision
                || op.settings_revision.as_ref() != Some(&request.settings_revision)
                || op.settings.as_ref() != Some(&request.parameters)
            {
                return Err("Request ID already used for different settings".into());
            }
            return Ok(op);
        }
        let eligible = |s: &FleetSnapshot| {
            s.link == "healthy"
                && s.rovers.iter().any(|r| {
                    r.id == id
                        && navigation_ready(r)
                        && r.authority.as_ref().is_some_and(|a| {
                            a.run_id == request.run_id && a.revision == request.revision
                        })
                        && r.navigation
                            .as_ref()
                            .is_some_and(|n| n.revision == request.settings_revision)
                })
        };
        if !eligible(&self.fleet.snapshot(now)) {
            return Err("Settings require fresh disarmed hardware, idle goal and current Terra configuration".into());
        }
        request
            .settings_revision
            .parse::<u64>()
            .ok()
            .and_then(|r| r.checked_add(1))
            .ok_or("Invalid settings revision")?;
        if self
            .operations
            .values()
            .any(|o| o.rover_id == id && !o.terminal())
        {
            return Err("Wait for the active command or cancel it before changing settings".into());
        }
        let token = uuid::Uuid::new_v4().to_string();
        let mut op = Operation {
            settings: Some(request.parameters.clone()),
            settings_revision: Some(request.settings_revision.clone()),
            id: uuid::Uuid::new_v4().to_string(),
            request_id: request.request_id.clone(),
            user_id: user.id.clone(),
            owner: owner.into(),
            rover_id: id.into(),
            kind: "navigation".into(),
            waypoint: None,
            phase: "settings_pending".into(),
            message: "Navigation settings sent; awaiting Terra acknowledgement".into(),
            mode_token: Some(token.clone()),
            goal_token: None,
            run_id: request.run_id.clone(),
            goal_observation: None,
            started: now,
            created_at: crate::auth::epoch() as f64,
            mode_level: String::new(),
            revision: request.revision.clone(),
            frame_id: None,
        };
        self.save(op.clone())?;
        let at = now + began.elapsed().as_secs_f64();
        if self.allowed(user, owner, at).is_err() || !eligible(&self.fleet.snapshot(at)) {
            op.phase = "interrupted".into();
            op.message = "Settings not sent: hardware, authority or configuration changed".into();
            return self.save(op);
        }
        if let Err(e) = self.fleet.navigation(id, &request, &token) {
            op.phase = "unconfirmed".into();
            op.message = format!("Settings publication failed or uncertain: {e}; not retried");
            return self.save(op);
        }
        Ok(op)
    }
    pub fn autonomy(
        &mut self,
        user: &Identity,
        owner: &str,
        id: &str,
        request: AutonomyRequest,
        now: f64,
    ) -> Result<Operation, String> {
        let began = std::time::Instant::now();
        self.allowed(user, owner, now)?;
        if ![
            "teleop",
            "assisted_teleop",
            "waypoint_direct",
            "waypoint",
            "target_search",
        ]
        .contains(&request.level.as_str())
        {
            return Err("Unsupported autonomy level".into());
        }
        let kind = format!("autonomy:{}", request.level);
        if let Some(op) = self.duplicate(user, id, &request.request_id, &kind, &None)? {
            if op.run_id != request.run_id || op.revision != request.revision {
                return Err("Request ID already used for another authority".into());
            }
            return Ok(op);
        }
        let snap = self.fleet.snapshot(now);
        let r = snap
            .rovers
            .iter()
            .find(|r| r.id == id)
            .ok_or("Unknown rover")?;
        let recent = |age: f64| age.is_finite() && (0.0..0.5).contains(&age);
        let a = r
            .authority
            .as_ref()
            .ok_or("Vehicle authority unavailable")?;
        if snap.link != "healthy"
            || r.membership != "online"
            || r.pose.is_none()
            || !r.pose_age.is_some_and(recent)
            || !recent(a.age)
            || a.run_id.is_empty()
            || a.run_id != request.run_id
            || a.revision != request.revision
        {
            return Err("Fresh pose and current vehicle authority required to change mode".into());
        }
        if !a.supported_levels.contains(&request.level) {
            return Err("Vehicle does not support this autonomy level".into());
        }
        // Explicit mode selection supersedes unpublished intent; never revive a previous goal later.
        let pending = self
            .operations
            .values()
            .filter(|o| o.rover_id == id && !o.terminal())
            .cloned()
            .collect::<Vec<_>>();
        for mut op in pending {
            op.phase = "interrupted".into();
            op.message = "Superseded by explicit autonomy selection".into();
            self.save(op)?;
        }
        let token = uuid::Uuid::new_v4().to_string();
        let mut op = Operation {
            settings: None,
            settings_revision: None,
            id: uuid::Uuid::new_v4().to_string(),
            request_id: request.request_id,
            user_id: user.id.clone(),
            owner: owner.into(),
            rover_id: id.into(),
            kind,
            waypoint: None,
            phase: "autonomy_pending".into(),
            message: "Mode requested; waiting for vehicle acknowledgement".into(),
            mode_token: Some(token.clone()),
            goal_token: None,
            run_id: request.run_id,
            goal_observation: None,
            started: now,
            created_at: crate::auth::epoch() as f64,
            mode_level: request.level.clone(),
            revision: request.revision,
            frame_id: None,
        };
        self.save(op.clone())?;
        // Storage can block while the transport continues receiving new authority.
        let publish_now = now + began.elapsed().as_secs_f64();
        let current = self.fleet.snapshot(publish_now);
        if self.allowed(user, owner, publish_now).is_err()
            || current.link != "healthy"
            || !current.rovers.iter().any(|r| {
                r.id == id
                    && r.membership == "online"
                    && r.pose.is_some()
                    && r.pose_age.is_some_and(recent)
                    && r.authority.as_ref().is_some_and(|a| {
                        recent(a.age)
                            && a.run_id == op.run_id
                            && a.revision == op.revision
                            && a.supported_levels.contains(&request.level)
                    })
            })
        {
            op.phase = "interrupted".into();
            op.message = "Mode not sent: vehicle authority or control changed".into();
            return self.save(op);
        }
        if let Err(e) = self.fleet.select_mode(id, &request.level, &token) {
            op.phase = "unconfirmed".into();
            op.message = format!("Mode publication failed or uncertain: {e}; not retried");
            return self.save(op);
        }
        Ok(op)
    }
    pub fn hardware(
        &mut self,
        user: &Identity,
        owner: &str,
        id: &str,
        request: HardwareRequest,
        now: f64,
    ) -> Result<Operation, String> {
        self.allowed(user, owner, now)?;
        if !["arm", "disarm"].contains(&request.action.as_str()) {
            return Err("Invalid hardware action".into());
        }
        if let Some(op) = self.duplicate(user, id, &request.request_id, &request.action, &None)? {
            if op.run_id != request.run_id || op.revision != request.revision {
                return Err("Request ID already used for another authority".into());
            }
            return Ok(op);
        }
        let snap = self.fleet.snapshot(now);
        let rover = snap
            .rovers
            .iter()
            .find(|r| r.id == id)
            .ok_or("Unknown rover")?;
        if request.action == "arm" {
            let recent = |age: f64| age.is_finite() && (0.0..0.5).contains(&age);
            if snap.link != "healthy"
                || rover.membership != "online"
                || rover.pose.is_none()
                || !rover.pose_age.is_some_and(recent)
                || !rover
                    .hardware
                    .as_ref()
                    .is_some_and(|h| h.ready && !h.armed && recent(h.age))
                || !rover.authority.as_ref().is_some_and(|a| {
                    !a.run_id.is_empty()
                        && a.run_id == request.run_id
                        && a.revision == request.revision
                        && a.safety == "clear"
                        && recent(a.age)
                })
            {
                return Err(
                    "Arming requires fresh pose, clear authority and ready, disarmed hardware"
                        .into(),
                );
            }
            if self
                .operations
                .values()
                .any(|o| o.rover_id == id && !o.terminal())
            {
                return Err("Rover has an active operation".into());
            }
        } else {
            // A disarm must retire unpublished goals so a later rearm cannot revive them.
            let pending = self
                .operations
                .values()
                .filter(|o| o.rover_id == id && !o.terminal())
                .cloned()
                .collect::<Vec<_>>();
            for mut op in pending {
                op.phase = "interrupted".into();
                op.message = "Superseded by explicit disarm".into();
                self.save(op)?;
            }
        }
        let token = uuid::Uuid::new_v4().to_string();
        let mut op = Operation {
            settings: None,
            settings_revision: None,
            id: uuid::Uuid::new_v4().to_string(),
            request_id: request.request_id,
            user_id: user.id.clone(),
            owner: owner.into(),
            rover_id: id.into(),
            kind: request.action.clone(),
            waypoint: None,
            phase: "hardware_pending".into(),
            message: "Hardware request published; awaiting rover confirmation".into(),
            mode_token: None,
            goal_token: Some(token.clone()),
            run_id: request.run_id.clone(),
            goal_observation: None,
            started: now,
            created_at: crate::auth::epoch() as f64,
            mode_level: String::new(),
            revision: request.revision.clone(),
            frame_id: None,
        };
        self.save(op.clone())?;
        if let Err(e) = self.fleet.hardware(
            id,
            &request.action,
            &token,
            &request.run_id,
            &request.revision,
        ) {
            op.phase = "unconfirmed".into();
            op.message = format!("Hardware publication failed or uncertain: {e}; not retried");
            return self.save(op);
        }
        Ok(op)
    }
    pub fn cancel(
        &mut self,
        user: &Identity,
        owner: &str,
        id: &str,
        request: &str,
        now: f64,
    ) -> Result<Operation, String> {
        self.allowed(user, owner, now)?;
        if let Some(op) = self.duplicate(user, id, request, "cancel", &None)? {
            return Ok(op);
        }
        let observed = self
            .fleet
            .snapshot(now)
            .rovers
            .into_iter()
            .find(|r| r.id == id)
            .ok_or("Unknown rover")?;
        let receipt = observed.goal_observation;
        let pending = self
            .operations
            .values()
            .filter(|o| o.rover_id == id && !o.terminal())
            .cloned()
            .collect::<Vec<_>>();
        for mut op in pending {
            op.phase = "cancelled".into();
            op.message = "Superseded by explicit cancellation".into();
            self.save(op)?;
        }
        let mut op = Operation {
            settings: None,
            settings_revision: None,
            id: uuid::Uuid::new_v4().to_string(),
            request_id: request.into(),
            user_id: user.id.clone(),
            owner: owner.into(),
            rover_id: id.into(),
            kind: "cancel".into(),
            waypoint: None,
            phase: "cancelling".into(),
            message: "Cancellation published; awaiting idle".into(),
            mode_token: None,
            goal_token: None,
            run_id: String::new(),
            goal_observation: receipt,
            started: now,
            created_at: crate::auth::epoch() as f64,
            mode_level: String::new(),
            revision: String::new(),
            frame_id: None,
        };
        self.save(op.clone())?;
        if let Err(e) = self.fleet.cancel(id) {
            op.phase = "rejected".into();
            op.message = e;
        }
        self.save(op)
    }
    pub fn tick(&mut self, now: f64) {
        if self.claim.as_ref().is_some_and(|c| c.expires_at <= now) {
            let owner = self
                .claim
                .as_ref()
                .map(|c| c.owner.clone())
                .unwrap_or_default();
            self.release(&owner);
        }
        let snap = self.fleet.snapshot(now);
        let expired: Vec<_> = self
            .held
            .iter()
            .filter(|(id, (owner, r, at))| {
                now - at >= 0.45
                    || !self
                        .claim
                        .as_ref()
                        .is_some_and(|c| c.owner == *owner && c.expires_at > now)
                    || !Self::drive_ready(&snap, id, r)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.stop_drive(&id);
        }
        let ops = self
            .operations
            .values()
            .filter(|o| !o.terminal())
            .cloned()
            .collect::<Vec<_>>();
        for mut op in ops {
            let rover = snap.rovers.iter().find(|r| r.id == op.rover_id);
            let before = op.clone();
            if op.kind == "navigation" {
                if !rover
                    .is_some_and(|r| r.authority.as_ref().is_some_and(|a| a.run_id == op.run_id))
                {
                    op.phase = "interrupted".into();
                    op.message = "Vehicle run changed; configuration outcome uncertain".into()
                } else if let Some(n) = rover.and_then(|r| r.navigation.as_ref()).filter(|n| {
                    n.age.is_finite() && (0.0..0.5).contains(&n.age) && n.token == op.mode_token
                }) {
                    if n.result.as_deref() == Some("accepted")
                        && op.settings.as_ref() == Some(&n.parameters)
                        && op
                            .settings_revision
                            .as_ref()
                            .and_then(|v| v.parse::<u64>().ok())
                            .and_then(|v| v.checked_add(1))
                            .map(|v| v.to_string())
                            .as_ref()
                            == Some(&n.revision)
                        && (n.footprint_radius - n.parameters.radius()).abs() < 1e-6
                    {
                        op.phase = "confirmed".into();
                        op.message =
                            "Terra applied vehicle dimensions and avoidance settings".into()
                    } else if n
                        .result
                        .as_deref()
                        .is_some_and(|v| v.starts_with("rejected"))
                    {
                        op.phase = "rejected".into();
                        op.message = n.result.clone().unwrap_or_default()
                    }
                }
                if !op.terminal() && now - op.started >= 10. {
                    op.phase = "unconfirmed".into();
                    op.message = "Navigation settings unconfirmed; not retried".into()
                }
                if (op.phase != before.phase || op.message != before.message)
                    && let Err(e) = self.save(op.clone())
                {
                    tracing::error!(error=%e,"navigation audit failed");
                    self.operations.insert(op.id.clone(), op);
                }
                continue;
            }
            if op.kind.starts_with("autonomy:") {
                if !rover
                    .is_some_and(|r| r.authority.as_ref().is_some_and(|a| a.run_id == op.run_id))
                {
                    op.phase = "interrupted".into();
                    op.message = "Vehicle run changed; mode outcome uncertain".into();
                } else if let Some(a) = rover
                    .and_then(|r| r.authority.as_ref())
                    .filter(|a| fresh(Some(a.age)) && a.token == op.mode_token)
                {
                    if a.result.as_deref() == Some("accepted") && a.requested_level == op.mode_level
                    {
                        op.phase = "confirmed".into();
                        op.message = format!("Vehicle acknowledged {}", op.mode_level);
                    } else if a.result.as_deref().is_some_and(|r| r != "accepted") {
                        op.phase = "rejected".into();
                        op.message = "Vehicle rejected autonomy selection".into();
                    }
                }
                if !op.terminal() && now - op.started >= 10. {
                    op.phase = "unconfirmed".into();
                    op.message = "Mode acknowledgement missing; not retried".into();
                }
                if (op.phase != before.phase || op.message != before.message)
                    && let Err(e) = self.save(op.clone())
                {
                    tracing::error!(error=%e,"autonomy audit failed");
                    self.operations.insert(op.id.clone(), op);
                }
                continue;
            }
            if op.kind == "arm" || op.kind == "disarm" {
                if op.kind == "arm"
                    && !rover.is_some_and(|r| {
                        r.authority.as_ref().is_some_and(|a| a.run_id == op.run_id)
                    })
                {
                    op.phase = "interrupted".into();
                    op.message = "Rover run changed; arming outcome uncertain".into();
                } else if let Some(h) = rover.and_then(|r| r.hardware.as_ref()).filter(|h| {
                    h.age.is_finite() && (0.0..0.5).contains(&h.age) && h.token == op.goal_token
                }) {
                    if h.result
                        .as_deref()
                        .is_some_and(|r| r.starts_with("rejected"))
                    {
                        op.phase = "rejected".into();
                        op.message = h.result.clone().unwrap_or_default();
                    } else if h.armed == (op.kind == "arm") {
                        op.phase = "confirmed".into();
                        op.message = format!(
                            "Rover confirms {}",
                            if h.armed { "armed" } else { "disarmed" }
                        );
                    }
                }
                if !op.terminal() && now - op.started >= 10. {
                    op.phase = "unconfirmed".into();
                    op.message =
                        "Hardware outcome unconfirmed; not retried. Check live hardware status."
                            .into();
                }
                if (op.phase != before.phase || op.message != before.message)
                    && let Err(e) = self.save(op.clone())
                {
                    tracing::error!(error=%e,"hardware audit failed");
                    self.operations.insert(op.id.clone(), op);
                }
                continue;
            }
            let pending = ["queued", "ready", "mode_pending"].contains(&op.phase.as_str());
            if pending {
                let held = self
                    .claim
                    .as_ref()
                    .is_some_and(|c| c.owner == op.owner && c.expires_at > now);
                if !held
                    || snap.link != "healthy"
                    || !rover.is_some_and(|r| {
                        r.ready()
                            && r.authority.as_ref().is_some_and(|a| a.run_id == op.run_id)
                            && r.frame_id == op.frame_id
                            && (!matches!(op.waypoint, Some(Waypoint::Geographic { .. }))
                                || r.reference.is_some())
                    })
                {
                    op.phase = "interrupted".into();
                    op.message =
                        "Unpublished goal discarded: control or rover readiness changed".into();
                } else if now - op.started >= argos_core::GOAL_ACK_SECONDS {
                    op.phase = "unconfirmed".into();
                    op.message = "Waypoint mode was not acknowledged; no goal sent".into();
                } else if let Some(a) = rover.and_then(|r| r.authority.as_ref()) {
                    if op.phase == "ready"
                        && (a.requested_level != op.mode_level || a.revision != op.revision)
                    {
                        op.phase = "interrupted".into();
                        op.message = "Waypoint authority changed before publication".into();
                    } else if op.phase == "ready"
                        || (op.mode_token.is_some()
                            && a.token == op.mode_token
                            && a.result.as_deref() == Some("accepted")
                            && a.requested_level == op.mode_level)
                    {
                        // Journal before publication: a crash in this window is uncertain, never retried.
                        op.phase = "publishing".into();
                        op.message = "Publishing waypoint once".into();
                        if self.save(op.clone()).is_err() {
                            continue;
                        }
                        match self.fleet.send_goal(
                            &op.rover_id,
                            op.waypoint.as_ref().expect("waypoint operation"),
                        ) {
                            Ok(token) => {
                                op.goal_token = Some(token);
                                op.phase = "published".into();
                                op.message = "Published; awaiting rover acceptance".into();
                                op.started = now;
                            }
                            Err(e) => {
                                op.phase = "unconfirmed".into();
                                op.message = format!("Publication failed or uncertain: {e}");
                            }
                        }
                    } else if a.token == op.mode_token
                        && a.result.as_deref().is_some_and(|r| r != "accepted")
                    {
                        op.phase = "rejected".into();
                        op.message = "Rover rejected waypoint authority".into();
                    }
                }
            } else if op.kind == "cancel" {
                if rover.is_some_and(|r| {
                    fresh(r.goal_age)
                        && r.goal_observation.is_some()
                        && r.goal_observation != op.goal_observation
                        && r.goal.as_ref().is_some_and(|g| g.state == "idle")
                }) {
                    op.phase = "cancelled".into();
                    op.message = "Rover reports idle".into();
                } else if now - op.started >= argos_core::GOAL_ACK_SECONDS {
                    op.phase = "unconfirmed".into();
                    op.message = "Cancellation unconfirmed; not retried".into();
                }
            } else if let Some(goal) = rover
                .filter(|r| fresh(r.goal_age))
                .and_then(|r| r.goal.as_ref())
                .filter(|g| g.token == op.goal_token)
            {
                if goal.state == "arrived" {
                    op.phase = "arrived".into();
                    op.message = "Rover reports arrival".into();
                } else if goal.state == "active" {
                    op.phase = "accepted".into();
                    op.message = "Rover accepted waypoint".into();
                }
            }
            if op.phase == "accepted"
                && (!rover.is_some_and(|r| {
                    r.membership == "online"
                        && r.authority.as_ref().is_some_and(|a| a.run_id == op.run_id)
                }) || rover.is_some_and(|r| {
                    fresh(r.goal_age)
                        && r.goal
                            .as_ref()
                            .is_some_and(|g| g.state == "idle" || g.token != op.goal_token)
                }))
            {
                op.phase = "interrupted".into();
                op.message = "Rover authority or goal changed; previous outcome uncertain".into();
            }
            if op.phase == "published" && now - op.started >= argos_core::GOAL_ACK_SECONDS {
                op.phase = "unconfirmed".into();
                op.message = "Goal unconfirmed; not retried".into();
            }
            if op.phase != before.phase || op.message != before.message {
                // Keep the final in-memory result even if audit storage fails after publication.
                if let Err(e) = self.save(op.clone()) {
                    tracing::error!(error=%e,"operation audit failed");
                    self.operations.insert(op.id.clone(), op);
                }
            }
        }
    }
    pub fn snapshot(&mut self, now: f64) -> FleetSnapshot {
        let mut s = self.fleet.snapshot(now);
        s.now = now;
        s.claim = self.claim.clone();
        let mut ops = self.operations.values().cloned().collect::<Vec<_>>();
        ops.sort_by(|a, b| {
            b.created_at
                .total_cmp(&a.created_at)
                .then_with(|| b.started.total_cmp(&a.started))
        });
        s.operations = ops.into_iter().take(100).collect();
        s
    }
}
