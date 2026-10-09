use std::sync::{Arc, Mutex};
uniffi::setup_scaffolding!();
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ClientError {
    #[error("{message}")]
    Operation { message: String },
}
impl From<argos_core::Error> for ClientError {
    fn from(e: argos_core::Error) -> Self {
        Self::Operation {
            message: e.to_string(),
        }
    }
}
#[derive(uniffi::Record)]
pub struct ConnectionConfig {
    pub endpoint: String,
    pub prefix: String,
}
#[derive(uniffi::Enum)]
pub enum Waypoint {
    Local {
        x: f64,
        y: f64,
        yaw: Option<f64>,
    },
    Geographic {
        latitude: f64,
        longitude: f64,
        yaw: Option<f64>,
    },
}
#[derive(uniffi::Record, Clone)]
pub struct RoverPose {
    pub x: f64,
    pub y: f64,
    pub yaw: f64,
}
#[derive(uniffi::Record, Clone)]
pub struct GoalProgress {
    pub state: String,
    pub goal_id: u64,
    pub token: Option<String>,
    pub distance: f64,
    pub x: f64,
    pub y: f64,
}
#[derive(uniffi::Record, Clone)]
pub struct RoverView {
    pub id: u64,
    pub membership: String,
    pub pose: Option<RoverPose>,
    pub pose_age: Option<f64>,
    pub goal: Option<GoalProgress>,
    pub goal_age: Option<f64>,
    pub command_phase: String,
    pub command_token: Option<String>,
}
#[derive(uniffi::Record, Clone)]
pub struct FleetSnapshot {
    pub link: String,
    pub fleet_age: Option<f64>,
    pub rovers: Vec<RoverView>,
    pub notice: Option<String>,
}
#[derive(uniffi::Object)]
pub struct ArgosClient {
    client: Mutex<Option<argos_zenoh::Client>>,
}
#[uniffi::export]
impl ArgosClient {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            client: Mutex::new(None),
        })
    }
    pub fn connect(&self, config: ConnectionConfig) -> Result<(), ClientError> {
        let mut slot = self.client.lock().unwrap();
        // Replacing a session never carries over pending commands or old telemetry.
        if let Some(old) = slot.take() {
            old.disconnect();
        }
        *slot = Some(argos_zenoh::Client::connect(
            &config.endpoint,
            &config.prefix,
        )?);
        Ok(())
    }
    pub fn disconnect(&self) {
        if let Some(client) = self.client.lock().unwrap().take() {
            client.disconnect();
        }
    }
    pub fn send_goal(&self, rover_id: u64, waypoint: Waypoint) -> Result<String, ClientError> {
        let slot = self.client.lock().unwrap();
        let client = slot.as_ref().ok_or_else(|| ClientError::Operation {
            message: "Disconnected".into(),
        })?;
        let request = match waypoint {
            Waypoint::Local { x, y, yaw } => argos_core::GoalRequest::Local { x, y, yaw },
            Waypoint::Geographic {
                latitude,
                longitude,
                yaw,
            } => argos_core::GoalRequest::Geographic {
                latitude,
                longitude,
                yaw,
            },
        };
        Ok(client.send_goal(rover_id, request)?)
    }
    pub fn cancel_goal(&self, rover_id: u64) -> Result<(), ClientError> {
        let slot = self.client.lock().unwrap();
        let client = slot.as_ref().ok_or_else(|| ClientError::Operation {
            message: "Disconnected".into(),
        })?;
        Ok(client.cancel_goal(rover_id)?)
    }
    pub fn operator_snapshot(&self) -> String {
        self.client
            .lock()
            .unwrap()
            .as_ref()
            .map(|c| c.operator_snapshot())
            .unwrap_or_else(|| "{}".into())
    }
    pub fn operator_command(
        &self,
        rover_id: u64,
        kind: String,
        payload: String,
    ) -> Result<(), ClientError> {
        let slot = self.client.lock().unwrap();
        let client = slot.as_ref().ok_or_else(|| ClientError::Operation {
            message: "Disconnected".into(),
        })?;
        Ok(client.operator_command(rover_id, &kind, &payload)?)
    }
    pub fn localization_snapshot(&self) -> String {
        self.client
            .lock()
            .unwrap()
            .as_ref()
            .map(|c| c.localization_snapshot())
            .unwrap_or_else(|| "{}".into())
    }
    pub fn snapshot(&self) -> FleetSnapshot {
        let slot = self.client.lock().unwrap();
        let Some(client) = slot.as_ref() else {
            return FleetSnapshot {
                link: "disconnected".into(),
                fleet_age: None,
                rovers: vec![],
                notice: None,
            };
        };
        let s = client.snapshot();
        FleetSnapshot {
            link: s.link,
            fleet_age: s.fleet_age,
            notice: s.notice,
            rovers: s
                .rovers
                .into_iter()
                .map(|r| RoverView {
                    id: r.id,
                    membership: r.membership,
                    pose_age: r.pose_age,
                    goal_age: r.goal_age,
                    command_phase: r.command_phase,
                    command_token: r.command_token,
                    pose: r.pose.map(|p| RoverPose {
                        x: p.x,
                        y: p.y,
                        yaw: p.yaw,
                    }),
                    goal: r.goal.map(|g| GoalProgress {
                        state: match g.state {
                            argos_core::GoalState::Idle => "idle",
                            argos_core::GoalState::Active => "active",
                            argos_core::GoalState::Arrived => "arrived",
                        }
                        .into(),
                        goal_id: g.goal_id,
                        token: g.token,
                        distance: g.distance,
                        x: g.x,
                        y: g.y,
                    }),
                })
                .collect(),
        }
    }
}
