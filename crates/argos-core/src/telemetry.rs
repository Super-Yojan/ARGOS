use crate::{MAX_STATE_BYTES, Result, contract::finite, invalid};
use serde_json::Value;
#[derive(Clone, Debug)]
pub struct Pose {
    pub rover_id: u64,
    pub sequence: u64,
    pub x: f64,
    pub y: f64,
    pub yaw: f64,
}
pub fn decode_pose(bytes: &[u8]) -> Result<Option<Pose>> {
    let split = bytes
        .iter()
        .take(MAX_STATE_BYTES + 1)
        .position(|b| *b == b'\n')
        .filter(|n| *n <= MAX_STATE_BYTES)
        .ok_or_else(|| invalid("missing or oversized depth header"))?;
    let h: Value =
        serde_json::from_slice(&bytes[..split]).map_err(|e| crate::Error(e.to_string()))?;
    if h["version"].as_u64() != Some(1) || h["encoding"].as_str() != Some("32FC1_LE") {
        return Err(invalid("unsupported depth packet"));
    }
    let id = h["rover_id"]
        .as_u64()
        .ok_or_else(|| invalid("invalid depth rover id"))?;
    let sequence = h["sequence"]
        .as_u64()
        .ok_or_else(|| invalid("invalid depth sequence"))?;
    let Some(body) = h.get("body") else {
        return Ok(None);
    };
    Ok(Some(Pose {
        rover_id: id,
        sequence,
        x: finite(body.get("x"))?,
        y: finite(body.get("y"))?,
        yaw: finite(body.get("yaw"))?,
    }))
}
#[derive(Clone, Debug)]
pub struct Anchor {
    pub latitude: f64,
    pub longitude: f64,
}
impl Anchor {
    fn validate(&self) -> Result<()> {
        if !self.latitude.is_finite()
            || !self.longitude.is_finite()
            || !(-85. ..=85.).contains(&self.latitude)
            || !(-180. ..=180.).contains(&self.longitude)
        {
            return Err(invalid("invalid map anchor"));
        }
        Ok(())
    }
    pub fn geographic(&self, x: f64, y: f64) -> Result<(f64, f64)> {
        self.validate()?;
        if !x.is_finite() || !y.is_finite() {
            return Err(invalid("invalid local coordinate"));
        }
        let scale = 40_075_016.686 / 360.;
        Ok((
            self.latitude + x / scale,
            self.longitude - y / (scale * self.latitude.to_radians().cos()),
        ))
    }
    pub fn local(&self, latitude: f64, longitude: f64) -> Result<(f64, f64)> {
        self.validate()?;
        if !latitude.is_finite() || !longitude.is_finite() {
            return Err(invalid("invalid geographic coordinate"));
        }
        let scale = 40_075_016.686 / 360.;
        Ok((
            (latitude - self.latitude) * scale,
            -(longitude - self.longitude) * scale * self.latitude.to_radians().cos(),
        ))
    }
}
/// Direct pose telemetry from the phone-owned shared runtime.
pub fn decode_body_pose(bytes: &[u8]) -> Result<Pose> {
    if bytes.len() > MAX_STATE_BYTES {
        return Err(invalid("oversized pose"));
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid("invalid pose JSON"))?;
    Ok(Pose {
        rover_id: value["rover_id"]
            .as_u64()
            .ok_or_else(|| invalid("missing rover ID"))?,
        sequence: value["sequence"]
            .as_u64()
            .ok_or_else(|| invalid("missing pose sequence"))?,
        x: finite(value.get("x"))?,
        y: finite(value.get("y"))?,
        yaw: finite(value.get("yaw"))?,
    })
}
