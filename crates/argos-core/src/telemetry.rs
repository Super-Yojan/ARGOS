//! Decode rover pose and project local metres into geographic coordinates.

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
/// Lightweight pose sent by a real phone; no depth image is required.
pub fn decode_phone_pose(bytes: &[u8]) -> Result<Pose> {
    if bytes.len() > MAX_STATE_BYTES {
        return Err(invalid("pose payload too large"));
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|e| crate::Error(e.to_string()))?;
    Ok(Pose {
        rover_id: value["rover_id"]
            .as_u64()
            .ok_or_else(|| invalid("invalid pose rover id"))?,
        sequence: value["sequence"]
            .as_u64()
            .ok_or_else(|| invalid("invalid pose sequence"))?,
        x: finite(value.get("x"))?,
        y: finite(value.get("y"))?,
        yaw: finite(value.get("yaw"))?,
    })
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
/// Pose on `<id>/pose`. `topic_id` is the rover named by the Zenoh key.
///
/// A payload may omit `rover_id` and `sequence` (Zorvane sends `x`, `y`, `yaw`,
/// and `heading`). When both a payload id and a topic id are present they must
/// match. `sequence` defaults to 0. Yaw is `yaw` when that field is present,
/// otherwise `heading`; both are radians.
pub fn decode_topic_pose(bytes: &[u8], topic_id: Option<u64>) -> Result<Pose> {
    if bytes.len() > MAX_STATE_BYTES {
        return Err(invalid("oversized pose"));
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid("invalid pose JSON"))?;
    let payload_id = match value.get("rover_id") {
        None | Some(Value::Null) => None,
        Some(id) => Some(
            id.as_u64()
                .ok_or_else(|| invalid("invalid pose rover id"))?,
        ),
    };
    let rover_id = match (payload_id, topic_id) {
        (Some(payload), Some(topic)) if payload != topic => {
            return Err(invalid("pose rover id does not match topic"));
        }
        (Some(payload), _) => payload,
        (None, Some(topic)) => topic,
        (None, None) => return Err(invalid("missing rover ID")),
    };
    let sequence = match value.get("sequence") {
        None | Some(Value::Null) => 0,
        Some(sequence) => sequence
            .as_u64()
            .ok_or_else(|| invalid("invalid pose sequence"))?,
    };
    Ok(Pose {
        rover_id,
        sequence,
        x: finite(value.get("x"))?,
        y: finite(value.get("y"))?,
        yaw: pose_yaw(&value)?,
    })
}
fn pose_yaw(value: &Value) -> Result<f64> {
    if let Some(yaw) = value.get("yaw").filter(|yaw| !yaw.is_null()) {
        finite(Some(yaw))
    } else {
        finite(value.get("heading"))
    }
}
/// Direct pose with no topic id. `rover_id` is required; `sequence` is optional.
pub fn decode_body_pose(bytes: &[u8]) -> Result<Pose> {
    decode_topic_pose(bytes, None)
}
/// Local metres relative to a geographic origin.
///
/// `rotation` is radians from local +x/+y into north/west, matching Terra's
/// geographic reference. +x is north and +y is west after that rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalFrame {
    pub latitude: f64,
    pub longitude: f64,
    pub anchor_x: f64,
    pub anchor_y: f64,
    pub rotation: f64,
}
impl LocalFrame {
    pub fn origin(latitude: f64, longitude: f64) -> Result<Self> {
        let frame = Self {
            latitude,
            longitude,
            anchor_x: 0.,
            anchor_y: 0.,
            rotation: 0.,
        };
        frame.validate()?;
        Ok(frame)
    }
    fn validate(&self) -> Result<()> {
        Anchor {
            latitude: self.latitude,
            longitude: self.longitude,
        }
        .validate()?;
        if !self.anchor_x.is_finite() || !self.anchor_y.is_finite() || !self.rotation.is_finite() {
            return Err(invalid("invalid local frame"));
        }
        Ok(())
    }
    pub fn geographic(&self, x: f64, y: f64) -> Result<(f64, f64)> {
        self.validate()?;
        let dx = x - self.anchor_x;
        let dy = y - self.anchor_y;
        let north = self.rotation.cos() * dx - self.rotation.sin() * dy;
        let west = self.rotation.sin() * dx + self.rotation.cos() * dy;
        Anchor {
            latitude: self.latitude,
            longitude: self.longitude,
        }
        .geographic(north, west)
    }
    /// Frame that places local `(x, y)` at a geographic fix and turns local
    /// `yaw` (radians, counter-clockwise) to true bearing `heading_deg`
    /// (clockwise from north). For drawing rover-frame data under a GPS marker;
    /// it is only as good as that fix and heading.
    pub fn pinned(
        latitude: f64,
        longitude: f64,
        x: f64,
        y: f64,
        yaw: f64,
        heading_deg: f64,
    ) -> Result<Self> {
        let frame = Self {
            latitude,
            longitude,
            anchor_x: x,
            anchor_y: y,
            rotation: -heading_deg.to_radians() - yaw,
        };
        frame.validate()?;
        Ok(frame)
    }
    /// Map heading, degrees clockwise from north, of local `yaw` in this frame.
    pub fn heading_degrees(&self, yaw: f64) -> f64 {
        (-(self.rotation + yaw).to_degrees()).rem_euclid(360.)
    }
    /// Geographic alignment from a localization report.
    ///
    /// Returns a frame only for schema version 1 in `geographic` mode with an
    /// origin. Local-mode reports are `None` even when they carry GPS fields.
    /// Missing anchor and rotation default to 0.
    pub fn from_localization(value: &Value) -> Option<Self> {
        if value.get("version").and_then(Value::as_u64) != Some(1)
            || value.get("mode").and_then(Value::as_str) != Some("geographic")
        {
            return None;
        }
        let latitude = value.get("originLatitude").and_then(Value::as_f64)?;
        let longitude = value.get("originLongitude").and_then(Value::as_f64)?;
        let frame = Self {
            latitude,
            longitude,
            anchor_x: component(value, "anchorX")?,
            anchor_y: component(value, "anchorY")?,
            rotation: component(value, "rotation")?,
        };
        frame.validate().ok()?;
        Some(frame)
    }
    /// Inverse of [`Self::geographic`].
    pub fn local(&self, latitude: f64, longitude: f64) -> Result<(f64, f64)> {
        self.validate()?;
        let (north, west) = Anchor {
            latitude: self.latitude,
            longitude: self.longitude,
        }
        .local(latitude, longitude)?;
        let (sin, cos) = (self.rotation.sin(), self.rotation.cos());
        let dx = cos * north + sin * west;
        let dy = -sin * north + cos * west;
        Ok((dx + self.anchor_x, dy + self.anchor_y))
    }
}
fn component(value: &Value, key: &str) -> Option<f64> {
    match value.get(key) {
        None | Some(Value::Null) => Some(0.),
        Some(component) => component.as_f64().filter(|n| n.is_finite()),
    }
}
/// Clockwise degrees about local up. Terra yaw is radians from +x toward +y.
pub fn map_heading_degrees(yaw: f64) -> f64 {
    (-yaw.to_degrees()).rem_euclid(360.)
}
/// Project a pose. A geographic localization origin wins; otherwise `fallback`.
/// GPS observations in the report are not the marker position.
pub fn rover_geographic(
    pose: &Pose,
    localization: Option<&Value>,
    fallback: LocalFrame,
) -> Result<(f64, f64)> {
    let frame = localization
        .and_then(LocalFrame::from_localization)
        .unwrap_or(fallback);
    frame.geographic(pose.x, pose.y)
}
