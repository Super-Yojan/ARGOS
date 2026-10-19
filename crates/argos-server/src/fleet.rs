use crate::{engine::Fleet, types::*};
use serde_json::Value;
use std::time::{Duration, Instant};

pub struct DemoFleet {
    pub rovers: Vec<Rover>,
    modes: Vec<(String, String, String, f64)>,
    goals: Vec<(String, f64, Pose, Pose)>,
    now: f64,
}
impl Default for DemoFleet {
    fn default() -> Self {
        let mut rovers = vec![
            Rover::demo("1", 0., 0.),
            Rover::demo("2", 8., -6.),
            Rover::demo("18446744073709551615", -5., 4.),
        ];
        rovers[1].hardware.as_mut().unwrap().armed = false;
        rovers[1].hardware.as_mut().unwrap().reason = "Disarmed · observation only".into();
        rovers[2].pose = None;
        rovers[2].pose_age = None;
        Self {
            rovers,
            modes: vec![],
            goals: vec![],
            now: 0.,
        }
    }
}
impl Fleet for DemoFleet {
    fn emergency_stop(&mut self, id: &str) -> Result<(), String> {
        let r = self
            .rovers
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or("Unknown rover")?;
        r.authority.as_mut().ok_or("Missing authority")?.safety = "stop".into();
        Ok(())
    }

    fn drive(&mut self, id: &str, r: &DriveRequest, _: &str) -> Result<(), String> {
        let rover = self
            .rovers
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or("Unknown rover")?;
        if let Some(p) = rover.pose.as_mut() {
            p.yaw += r.angular * 0.2;
            p.x += r.linear * p.yaw.cos() * 0.2;
            p.y += r.linear * p.yaw.sin() * 0.2;
        }
        Ok(())
    }

    fn snapshot(&mut self, now: f64) -> FleetSnapshot {
        self.now = now;
        for (id, level, token, at) in std::mem::take(&mut self.modes) {
            if now >= at {
                if let Some(a) = self
                    .rovers
                    .iter_mut()
                    .find(|r| r.id == id)
                    .and_then(|r| r.authority.as_mut())
                {
                    if a.safety != "stop" && ["teleop", "waypoint_direct"].contains(&level.as_str())
                    {
                        a.safety = "clear".into();
                        a.reason = Some("idle".into());
                    }
                    a.requested_level = level;
                    a.token = Some(token);
                    a.result = Some("accepted".into());
                }
            } else {
                self.modes.push((id, level, token, at));
            }
        }
        for (id, start, from, to) in &self.goals {
            if let Some(r) = self.rovers.iter_mut().find(|r| r.id == *id) {
                let t = ((now - start) / 5.).clamp(0., 1.);
                r.pose = Some(Pose {
                    x: from.x + (to.x - from.x) * t,
                    y: from.y + (to.y - from.y) * t,
                    yaw: 0.,
                });
                if let Some(g) = r.goal.as_mut() {
                    g.distance = ((to.x - from.x).hypot(to.y - from.y)) * (1. - t);
                    g.state = if t >= 1. { "arrived" } else { "active" }.into();
                }
            }
        }
        // The demo synthesizes GPS explicitly; live fleets only use reported fixes.
        for rover in &mut self.rovers {
            rover.gps = rover.pose.as_ref().map(|pose| GpsFix {
                latitude: 38.8297 + pose.x / 111_319.4908,
                longitude: -77.3075 - pose.y / (111_319.4908 * 38.8297_f64.to_radians().cos()),
                accuracy: 4.,
                age: 0.,
            });
        }
        FleetSnapshot {
            link: "healthy".into(),
            rovers: self.rovers.clone(),
            ..Default::default()
        }
    }
    fn navigation(
        &mut self,
        id: &str,
        request: &NavigationRequest,
        token: &str,
    ) -> Result<(), String> {
        let r = self
            .rovers
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or("Unknown rover")?;
        let n = r.navigation.as_mut().ok_or("Unsupported configuration")?;
        n.revision = (n.revision.parse::<u64>().map_err(|_| "Invalid revision")? + 1).to_string();
        n.parameters = request.parameters.clone();
        n.footprint_radius = n.parameters.radius();
        n.token = Some(token.into());
        n.result = Some("accepted".into());
        Ok(())
    }
    fn occupancy(&mut self, id: &str, _now: f64) -> Result<Option<Occupancy>, String> {
        let r = self
            .rovers
            .iter()
            .find(|r| r.id == id)
            .ok_or("Unknown rover")?;
        let mut cells = vec![0; 1600];
        for row in 0..40 {
            for col in 0..40 {
                cells[row * 40 + col] = if row < 5 && col < 10 {
                    -1
                } else if row == 0
                    || col == 0
                    || row == 39
                    || col == 39
                    || ((15..20).contains(&row) && (23..28).contains(&col))
                {
                    100
                } else {
                    0
                };
            }
        }
        Ok(Some(Occupancy {
            rover_id: id.into(),
            run_id: "demo-run".into(),
            sequence: "1".into(),
            frame_id: r.frame_id.clone(),
            width: 40,
            height: 40,
            resolution: 0.5,
            origin_x: -10.,
            origin_y: -10.,
            cells,
            age: 0.,
        }))
    }
    fn select_mode(&mut self, id: &str, level: &str, token: &str) -> Result<(), String> {
        self.modes
            .push((id.into(), level.into(), token.into(), self.now + 0.6));
        Ok(())
    }
    fn send_goal(&mut self, id: &str, goal: &Waypoint) -> Result<String, String> {
        let r = self
            .rovers
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or("Unknown rover")?;
        let (x, y) = match *goal {
            Waypoint::Local { x, y, .. } => (x, y),
            Waypoint::Geographic {
                latitude,
                longitude,
                ..
            } => {
                let a = r.reference.as_ref().ok_or("Missing geographic reference")?;
                argos_core::Anchor {
                    latitude: a.latitude,
                    longitude: a.longitude,
                }
                .local(latitude, longitude)
                .map_err(|e| e.to_string())?
            }
        };
        let from = r.pose.clone().ok_or("Missing pose")?;
        let token = uuid::Uuid::new_v4().to_string();
        r.goal = Some(Goal {
            state: "active".into(),
            token: Some(token.clone()),
            x,
            y,
            distance: (x - from.x).hypot(y - from.y),
        });
        r.goal_age = Some(0.);
        r.goal_observation = Some(uuid::Uuid::new_v4().to_string());
        self.goals.retain(|(i, ..)| i != id);
        self.goals
            .push((id.into(), self.now, from, Pose { x, y, yaw: 0. }));
        Ok(token)
    }
    fn hardware(
        &mut self,
        id: &str,
        action: &str,
        token: &str,
        _run: &str,
        _revision: &str,
    ) -> Result<(), String> {
        let h = self
            .rovers
            .iter_mut()
            .find(|r| r.id == id)
            .and_then(|r| r.hardware.as_mut())
            .ok_or("Unknown hardware")?;
        h.armed = action == "arm";
        h.token = Some(token.into());
        h.result = Some("requested".into());
        h.reason = "Simulated hardware".into();
        h.age = 0.;
        Ok(())
    }
    fn cancel(&mut self, id: &str) -> Result<(), String> {
        self.goals.retain(|(i, ..)| i != id);
        self.modes.retain(|(i, ..)| i != id);
        let r = self
            .rovers
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or("Unknown rover")?;
        r.goal = Some(Goal {
            state: "idle".into(),
            token: None,
            x: 0.,
            y: 0.,
            distance: 0.,
        });
        r.goal_age = Some(0.);
        r.goal_observation = Some(uuid::Uuid::new_v4().to_string());
        Ok(())
    }
}

pub struct LiveFleet {
    endpoint: String,
    prefix: String,
    client: Option<argos_zenoh::Client>,
    retry: Instant,
}
impl LiveFleet {
    pub fn new(endpoint: String, prefix: String) -> Self {
        Self {
            endpoint,
            prefix,
            client: None,
            retry: Instant::now() - Duration::from_secs(6),
        }
    }
    fn client(&self) -> Result<&argos_zenoh::Client, String> {
        self.client.as_ref().ok_or("Fleet disconnected".into())
    }
}
fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().into()
}
fn num(v: &Value, key: &str) -> Option<f64> {
    v[key].as_f64().filter(|x| x.is_finite())
}
// GPS is observation only: never infer a local frame, rotation or drive readiness.
fn gps_fix(v: &Value) -> Option<GpsFix> {
    if v["version"] != 1 {
        return None;
    }
    let received_age = num(v, "age")?;
    let fix_age = num(v, "gpsFixAge")?;
    let fix = GpsFix {
        latitude: num(v, "gpsLatitude")?,
        longitude: num(v, "gpsLongitude")?,
        accuracy: num(v, "gpsAccuracy")?,
        age: received_age + fix_age,
    };
    if received_age < 0.
        || fix_age < 0.
        || fix.age > 3.
        || fix.latitude.abs() > 90.
        || fix.longitude.abs() > 180.
        || fix.accuracy < 0.
    {
        None
    } else {
        Some(fix)
    }
}

impl Fleet for LiveFleet {
    fn emergency_stop(&mut self, id: &str) -> Result<(), String> {
        self.client()?
            .action(
                id.parse().map_err(|_| "Invalid rover ID")?,
                "safety",
                serde_json::json!({"action":"stop"}),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn drive(&mut self, id: &str, r: &DriveRequest, session: &str) -> Result<(), String> {
        self.client()?.operator_command(id.parse().map_err(|_|"Invalid rover ID")?,"teleop",&serde_json::json!({"linear":r.linear,"angular":r.angular,"run_id":r.run_id,"authority_revision":r.revision.parse::<u64>().map_err(|_|"Invalid revision")?,"operator_session_id":session,"sequence":r.sequence}).to_string()).map_err(|e|e.to_string())
    }

    fn snapshot(&mut self, _: f64) -> FleetSnapshot {
        if self
            .client
            .as_ref()
            .is_some_and(|c| c.snapshot().link == "failed")
        {
            self.client = None;
        }
        if self.client.is_none() && self.retry.elapsed() >= Duration::from_secs(5) {
            self.retry = Instant::now();
            match argos_zenoh::Client::connect_authenticated(
                &self.endpoint,
                &self.prefix,
                std::env::var("ARGOS_ZENOH_USER")
                    .ok()
                    .zip(std::env::var("ARGOS_ZENOH_PASSWORD").ok())
                    .as_ref()
                    .map(|(u, p)| (u.as_str(), p.as_str())),
            ) {
                Ok(c) => self.client = Some(c),
                Err(e) => tracing::warn!(error=%e,"fleet connection failed"),
            }
        }
        let Some(c) = &self.client else {
            return FleetSnapshot {
                link: "disconnected".into(),
                ..Default::default()
            };
        };
        let s = c.snapshot();
        let op: Value = serde_json::from_str(&c.operator_state_snapshot()).unwrap_or_default();
        let loc: Value = serde_json::from_str(&c.localization_snapshot()).unwrap_or_default();
        let rovers = s
            .rovers
            .into_iter()
            .map(|r| {
                let id = r.id.to_string();
                let a = &op[&id]["authority"];
                let h = &op[&id]["hardware"];
                let n = &a["navigation"];
                Rover {
                    gps: gps_fix(&loc[&id]),
                    navigation: if n["schema_version"] == 1 {
                        serde_json::from_value::<NavigationParameters>(n["parameters"].clone())
                            .ok()
                            .filter(|p| p.valid())
                            .map(|parameters| NavigationState {
                                schema_version: 1,
                                revision: n["revision"].as_u64().unwrap_or(0).to_string(),
                                parameters,
                                footprint_radius: n["footprint_radius"].as_f64().unwrap_or(-1.),
                                age: num(a, "age").unwrap_or(f64::MAX),
                                token: n["token"].as_str().map(String::from),
                                result: n["result"].as_str().map(String::from),
                            })
                    } else {
                        None
                    },
                    id: id.clone(),
                    membership: r.membership,
                    pose: r.pose.map(|p| Pose {
                        x: p.x,
                        y: p.y,
                        yaw: p.yaw,
                    }),
                    pose_age: r.pose_age,
                    goal: r.goal.map(|g| Goal {
                        state: match g.state {
                            argos_core::GoalState::Idle => "idle",
                            argos_core::GoalState::Active => "active",
                            argos_core::GoalState::Arrived => "arrived",
                        }
                        .into(),
                        token: g.token,
                        x: g.x,
                        y: g.y,
                        distance: g.distance,
                    }),
                    goal_age: r.goal_age,
                    goal_observation: r.goal_received_at.map(|at| at.to_bits().to_string()),
                    frame_id: loc[&id]["frameID"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(String::from),
                    authority: if a.is_object() {
                        Some(Authority {
                            reason: a["reason"].as_str().map(String::from),
                            run_id: text(a, "run_id"),
                            revision: a["revision"].as_u64().unwrap_or(0).to_string(),
                            requested_level: text(a, "requested_level"),
                            supported_levels: a["supported_levels"]
                                .as_array()
                                .map(|v| {
                                    v.iter()
                                        .filter_map(|s| s.as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default(),
                            safety: text(a, "safety"),
                            age: num(a, "age").unwrap_or(f64::MAX),
                            token: a["token"].as_str().map(String::from),
                            result: a["result"].as_str().map(String::from),
                        })
                    } else {
                        None
                    },
                    hardware: if h.is_object() {
                        Some(Hardware {
                            arming: h["arming"].as_bool(),
                            token: h["token"].as_str().map(String::from),
                            result: h["result"].as_str().map(String::from),
                            ready: h["ready"] == true,
                            armed: h["armed"] == true,
                            reason: text(h, "reason"),
                            age: num(h, "age").unwrap_or(f64::MAX),
                        })
                    } else {
                        None
                    },
                    reference: None,
                }
            })
            .collect();
        FleetSnapshot {
            link: s.link,
            rovers,
            ..Default::default()
        }
    }
    fn navigation(
        &mut self,
        id: &str,
        request: &NavigationRequest,
        token: &str,
    ) -> Result<(), String> {
        self.client()?.operator_command(id.parse().map_err(|_|"Invalid rover ID")?,"navigation/settings",&serde_json::json!({"token":token,"run_id":request.run_id,"authority_revision":request.revision.parse::<u64>().map_err(|_|"Invalid authority revision")?,"settings_revision":request.settings_revision.parse::<u64>().map_err(|_|"Invalid settings revision")?,"parameters":request.parameters}).to_string()).map_err(|e|e.to_string())
    }
    fn occupancy(&mut self, id: &str, now: f64) -> Result<Option<Occupancy>, String> {
        let r = self
            .snapshot(now)
            .rovers
            .into_iter()
            .find(|r| r.id == id)
            .ok_or("Unknown rover")?;
        let Some((g, age, frame_id)) = self
            .client()?
            .occupancy_grid(id.parse().map_err(|_| "Invalid rover ID")?)
        else {
            return Ok(None);
        };
        if frame_id != r.frame_id || !r.authority.as_ref().is_some_and(|a| a.run_id == g.run_id) {
            return Ok(None);
        }
        Ok(Some(Occupancy {
            rover_id: g.rover_id.to_string(),
            run_id: g.run_id,
            sequence: g.sequence.to_string(),
            frame_id,
            width: g.width,
            height: g.height,
            resolution: g.resolution,
            origin_x: g.origin_x,
            origin_y: g.origin_y,
            cells: g.occupancy,
            age,
        }))
    }
    fn select_mode(&mut self, id: &str, level: &str, token: &str) -> Result<(), String> {
        self.client()?
            .operator_command(
                id.parse().map_err(|_| "Invalid rover ID")?,
                "autonomy",
                &serde_json::json!({"level":level,"token":token}).to_string(),
            )
            .map_err(|e| e.to_string())
    }
    fn send_goal(&mut self, id: &str, goal: &Waypoint) -> Result<String, String> {
        self.client()?
            .send_goal(id.parse().map_err(|_| "Invalid rover ID")?, goal.core())
            .map_err(|e| e.to_string())
    }
    fn hardware(
        &mut self,
        id: &str,
        action: &str,
        token: &str,
        run: &str,
        revision: &str,
    ) -> Result<(), String> {
        let revision = if action == "arm" {
            revision
                .parse::<u64>()
                .map_err(|_| "Invalid authority revision")?
        } else {
            0
        };
        self.client()?.operator_command(id.parse().map_err(|_| "Invalid rover ID")?, "hardware", &serde_json::json!({"action":action,"token":token,"run_id":run,"authority_revision":revision}).to_string()).map_err(|e| e.to_string())
    }
    fn cancel(&mut self, id: &str) -> Result<(), String> {
        self.client()?
            .cancel_goal(id.parse().map_err(|_| "Invalid rover ID")?)
            .map_err(|e| e.to_string())
    }
}
impl Fleet for Box<dyn Fleet> {
    fn emergency_stop(&mut self, id: &str) -> Result<(), String> {
        self.as_mut().emergency_stop(id)
    }

    fn drive(&mut self, id: &str, r: &DriveRequest, session: &str) -> Result<(), String> {
        self.as_mut().drive(id, r, session)
    }

    fn snapshot(&mut self, now: f64) -> FleetSnapshot {
        self.as_mut().snapshot(now)
    }
    fn navigation(
        &mut self,
        id: &str,
        request: &NavigationRequest,
        token: &str,
    ) -> Result<(), String> {
        self.as_mut().navigation(id, request, token)
    }
    fn occupancy(&mut self, id: &str, now: f64) -> Result<Option<Occupancy>, String> {
        self.as_mut().occupancy(id, now)
    }
    fn select_mode(&mut self, id: &str, level: &str, token: &str) -> Result<(), String> {
        self.as_mut().select_mode(id, level, token)
    }
    fn send_goal(&mut self, id: &str, goal: &Waypoint) -> Result<String, String> {
        self.as_mut().send_goal(id, goal)
    }
    fn hardware(
        &mut self,
        id: &str,
        action: &str,
        token: &str,
        run: &str,
        revision: &str,
    ) -> Result<(), String> {
        self.as_mut().hardware(id, action, token, run, revision)
    }
    fn cancel(&mut self, id: &str) -> Result<(), String> {
        self.as_mut().cancel(id)
    }
}

#[cfg(test)]
mod gps_tests {
    use super::gps_fix;
    use serde_json::json;
    #[test]
    fn raw_gps_ignores_heading_tracking_and_alignment_but_checks_measurement_age() {
        let report = json!({"version":1,"mode":"local","tracking":"limited",
            "age":0.2,"gpsFixAge":0.3,"gpsLatitude":38.8,"gpsLongitude":-77.3,
            "gpsAccuracy":45.,"headingAccuracy":-1});
        let fix = gps_fix(&report).unwrap();
        assert_eq!(fix.latitude, 38.8);
        assert_eq!(fix.accuracy, 45.);
        assert_eq!(fix.age, 0.5);
        for (key, value) in [
            ("gpsFixAge", json!(2.9)),
            ("age", json!(-1.)),
            ("gpsLatitude", json!(91.)),
            ("gpsLongitude", json!(181.)),
            ("gpsAccuracy", json!(-1.)),
            ("gpsAccuracy", json!(null)),
        ] {
            let mut invalid = report.clone();
            invalid[key] = value;
            assert!(gps_fix(&invalid).is_none(), "invalid {key}");
        }
    }
}
