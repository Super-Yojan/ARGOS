//! Objects (buckets, balls) a rover's detector has found.
//!
//! Terra publishes version-1 status on `{prefix}/{id}/objects`: each object has
//! a local position in the rover's map frame and, outdoors, latitude and longitude.
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct FoundObject {
    pub id: u32,
    /// `bucket` or `ball`.
    pub class: String,
    /// `blue`, `orange`, or `None`.
    pub color: Option<String>,
    /// Local map position, metres.
    pub x: f64,
    pub y: f64,
    /// Rover-reported GPS position, when it had a good fix and heading.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub geo_accuracy_m: Option<f64>,
    pub confidence: f64,
    pub sightings: u32,
}

impl FoundObject {
    /// "orange bucket", "ball".
    pub fn label(&self) -> String {
        match &self.color {
            Some(color) => format!("{color} {}", self.class),
            None => self.class.clone(),
        }
    }
    /// Marker colour for the map, as 0xRRGGBB.
    pub fn marker_color(&self) -> u32 {
        match (self.class.as_str(), self.color.as_deref()) {
            ("bucket", Some("blue")) => 0x2f7bff,
            ("bucket", Some("orange")) => 0xff8a1f,
            ("bucket", _) => 0xd94fd0,
            _ => 0xf4e04d,
        }
    }
}

fn token(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    ((1..=32).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)))
    .then(|| s.to_string())
}

/// Confirmed objects from an `objects` body. Unconfirmed sightings and
/// malformed entries are dropped; at most 64 are kept.
pub fn decode_objects(value: &Value) -> Vec<FoundObject> {
    if value["schema_version"] != 1 {
        return Vec::new();
    }
    let Some(list) = value["objects"].as_array() else {
        return Vec::new();
    };
    list.iter()
        .filter(|o| o["confirmed"] == true)
        .filter_map(|o| {
            let x = o["x"].as_f64().filter(|v| v.is_finite())?;
            let y = o["y"].as_f64().filter(|v| v.is_finite())?;
            let latitude = o["latitude"]
                .as_f64()
                .filter(|v| v.is_finite() && (-85.0..=85.).contains(v));
            let longitude = o["longitude"]
                .as_f64()
                .filter(|v| v.is_finite() && (-180.0..=180.).contains(v));
            let (latitude, longitude) = match (latitude, longitude) {
                (Some(a), Some(b)) => (Some(a), Some(b)),
                _ => (None, None),
            };
            Some(FoundObject {
                id: u32::try_from(o["id"].as_u64()?).ok()?,
                class: token(&o["class"])?,
                color: o.get("color").and_then(token),
                x,
                y,
                latitude,
                longitude,
                geo_accuracy_m: o["geo_accuracy_m"].as_f64().filter(|v| v.is_finite()),
                confidence: o["confidence"].as_f64().filter(|v| v.is_finite())?,
                sightings: u32::try_from(o["sightings"].as_u64()?).ok()?,
            })
        })
        .take(64)
        .collect()
}
