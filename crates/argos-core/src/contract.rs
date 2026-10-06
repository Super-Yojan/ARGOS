use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);
pub type Result<T> = std::result::Result<T, Error>;
pub fn invalid(message: &str) -> Error {
    Error(message.into())
}
pub const MAX_STATE_BYTES: usize = 65_536;
pub const STALE_SECONDS: f64 = 2.5;
#[derive(Clone, Debug)]
pub struct Topics {
    pub prefix: String,
}
impl Topics {
    pub fn new(prefix: &str) -> Result<Self> {
        let p = prefix.trim().trim_matches('/');
        if p.is_empty()
            || p.len() > 256
            || p.split('/').any(str::is_empty)
            || p.chars().any(|c| c.is_whitespace() || "*?#$".contains(c))
        {
            return Err(invalid("prefix must be a concrete Zenoh key"));
        }
        Ok(Self { prefix: p.into() })
    }
    pub fn goal(&self, id: u64) -> String {
        format!("{}/{id}/goal", self.prefix)
    }
    pub fn fleet(&self) -> String {
        format!("{}/fleet/state", self.prefix)
    }
    pub fn statuses(&self) -> String {
        format!("{}/*/goal/status", self.prefix)
    }
    pub fn depth(&self) -> String {
        format!("{}/*/camera/depth", self.prefix)
    }
    pub fn rover_from_key(&self, key: &str, suffix: &str) -> Option<u64> {
        key.strip_prefix(&format!("{}/", self.prefix))?
            .strip_suffix(suffix)?
            .parse()
            .ok()
    }
}
#[derive(Clone, Debug)]
pub enum GoalRequest {
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
pub fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 64
        && token
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c))
}
impl GoalRequest {
    pub fn encode(&self, token: &str) -> Result<Vec<u8>> {
        if !valid_token(token) {
            return Err(invalid("invalid goal token"));
        }
        let (mut value, yaw) = match *self {
            Self::Local { x, y, yaw } => {
                if !x.is_finite() || !y.is_finite() || x.abs() > 20_000. || y.abs() > 20_000. {
                    return Err(invalid(
                        "local coordinates must be finite and within 20000 metres",
                    ));
                }
                (json!({"frame":"local","x":x,"y":y}), yaw)
            }
            Self::Geographic {
                latitude,
                longitude,
                yaw,
            } => {
                if !latitude.is_finite()
                    || !longitude.is_finite()
                    || !(-85. ..=85.).contains(&latitude)
                    || !(-180. ..=180.).contains(&longitude)
                {
                    return Err(invalid("geographic coordinates outside Terra bounds"));
                }
                (
                    json!({"frame":"wgs84","latitude":latitude,"longitude":longitude}),
                    yaw,
                )
            }
        };
        if let Some(yaw) = yaw {
            if !yaw.is_finite() || yaw.abs() > std::f64::consts::TAU {
                return Err(invalid("yaw must be within +/-2pi"));
            }
            value["yaw"] = json!(yaw);
        }
        value["token"] = json!(token);
        let bytes = serde_json::to_vec(&value).map_err(|e| Error(e.to_string()))?;
        if bytes.len() > 2048 {
            return Err(invalid("goal exceeds 2048 bytes"));
        }
        Ok(bytes)
    }
}
pub fn cancel_payload() -> &'static [u8] {
    br#"{"cancel":true}"#
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum GoalState {
    Idle,
    Active,
    Arrived,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GoalStatus {
    pub state: GoalState,
    pub goal_id: u64,
    pub distance: f64,
    pub x: f64,
    pub y: f64,
    pub token: Option<String>,
    pub yaw: Option<f64>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct FleetState {
    pub count: u64,
    pub max_count: u64,
    pub ids: Vec<u64>,
}
fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if bytes.len() > MAX_STATE_BYTES {
        return Err(invalid("state payload too large"));
    }
    serde_json::from_slice(bytes).map_err(|e| Error(format!("invalid telemetry: {e}")))
}
pub fn decode_fleet(bytes: &[u8]) -> Result<FleetState> {
    let f: FleetState = parse(bytes)?;
    if f.ids.len() > 4096 || f.ids.iter().collect::<BTreeSet<_>>().len() != f.ids.len() {
        return Err(invalid("invalid or duplicate fleet ids"));
    }
    Ok(f)
}
pub fn decode_status(bytes: &[u8]) -> Result<GoalStatus> {
    let s: GoalStatus = parse(bytes)?;
    if !s.distance.is_finite()
        || s.distance < 0.
        || !s.x.is_finite()
        || !s.y.is_finite()
        || (s.state == GoalState::Idle) != (s.goal_id == 0)
        || s.token.as_ref().is_some_and(|t| !valid_token(t))
        || s.yaw
            .is_some_and(|v| !v.is_finite() || v.abs() > std::f64::consts::TAU)
        || s.latitude.is_some() != s.longitude.is_some()
        || s.latitude
            .is_some_and(|v| !v.is_finite() || !(-85. ..=85.).contains(&v))
        || s.longitude
            .is_some_and(|v| !v.is_finite() || !(-180. ..=180.).contains(&v))
    {
        return Err(invalid("invalid goal status values"));
    }
    Ok(s)
}
pub(crate) fn finite(value: Option<&Value>) -> Result<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .ok_or_else(|| invalid("expected a finite number"))
}
