//! Polling C API for the Unity fleet spike.
//!
//! Unity calls `argos_bridge_open`, then `argos_bridge_poll` from the main thread.
//! The poll buffer is caller-owned JSON in the same shape as `fleet-telemetry.json`.
//! There are no native-to-managed callbacks.

mod publish;

use argos_core::{RoverSnapshot, STALE_SECONDS, Snapshot};
use argos_zenoh::Client;
use std::ffi::{CStr, CString, c_char};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub use publish::{FleetPublisher, connect_publisher, serve};

static BRIDGE: Mutex<Option<Client>> = Mutex::new(None);
static LAST_ERROR: Mutex<String> = Mutex::new(String::new());

fn set_error(message: impl Into<String>) {
    *LAST_ERROR.lock().unwrap_or_else(|p| p.into_inner()) = message.into();
}

fn take_client() -> Option<Client> {
    BRIDGE.lock().unwrap_or_else(|p| p.into_inner()).take()
}

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn rover_label(id: u64) -> String {
    if id < 100 {
        format!("terra-{id:02}")
    } else {
        format!("terra-{id}")
    }
}

/// Ground-plane pose used by the fake publisher and the Mac framing grid.
/// `(x, y, yaw)` with yaw in radians. Unity maps `y` onto `z`.
pub fn rover_layout(id: u64) -> (f64, f64, f64) {
    let index = id.saturating_sub(1);
    let col = (index % 5) as f64;
    let row = (index / 5) as f64;
    let x = (col - 2.0) * 4.6;
    let y = (row - 1.5) * 4.4;
    let yaw = if id == 1 {
        std::f64::consts::FRAC_PI_2
    } else {
        (id as f64) * 0.17
    };
    (x, y, yaw)
}

fn lost_link(rover: &RoverSnapshot) -> bool {
    rover.membership != "online" || rover.pose_age.is_none_or(|age| age > STALE_SECONDS)
}

fn visual_status(rover: &RoverSnapshot, operator: &serde_json::Value) -> &'static str {
    let entry = &operator[rover.id.to_string()];
    let age = entry["age"].as_f64().unwrap_or(0.0);
    let fresh_operator = entry["status"].is_object() && age <= STALE_SECONDS;
    let status = if fresh_operator {
        &entry["status"]
    } else {
        &serde_json::Value::Null
    };
    let safety = status["safety"].as_str().unwrap_or("");
    let reason = status["reason"].as_str().unwrap_or("");
    if safety == "emergency_stop"
        || matches!(reason, "map_stale" | "sensor_unhealthy" | "invalid_time")
        || lost_link(rover)
    {
        return "critical";
    }
    let proposal = entry.get("proposal").is_some_and(|value| !value.is_null());
    let rejected = status["result"].as_str() == Some("rejected")
        && status["request_reason"].as_str() == Some("authority_required");
    if safety == "hold" || proposal || rejected {
        return "attention";
    }
    "normal"
}

pub fn fleet_json(snapshot: &Snapshot, operator: &serde_json::Value, now_unix: f64) -> String {
    let mut rovers = Vec::new();
    for rover in &snapshot.rovers {
        let Some(pose) = rover.pose.as_ref() else {
            continue;
        };
        let age = rover.pose_age.unwrap_or(STALE_SECONDS + 1.0).max(0.0);
        rovers.push(serde_json::json!({
            "id": rover_label(rover.id),
            "position": {"x": pose.x, "y": 0.0, "z": pose.y},
            "headingDegrees": pose.yaw.to_degrees(),
            "status": visual_status(rover, operator),
            "lastUpdateUnixSeconds": now_unix - age,
        }));
    }
    serde_json::json!({
        "snapshotUnixSeconds": now_unix,
        "link": snapshot.link,
        "rovers": rovers,
    })
    .to_string()
}

pub const OCCUPANCY_SIZE: u32 = 24;
pub const OCCUPANCY_RESOLUTION: f64 = 0.5;

pub fn occupancy_origin(id: u64) -> (f64, f64) {
    let (x, y, _) = rover_layout(id);
    let half = f64::from(OCCUPANCY_SIZE) * OCCUPANCY_RESOLUTION * 0.5;
    (x - half, y - half)
}

fn cell_offset(index: u32) -> f64 {
    (f64::from(index) + 0.5) * OCCUPANCY_RESOLUTION
        - f64::from(OCCUPANCY_SIZE) * OCCUPANCY_RESOLUTION * 0.5
}

/// Row-major cell in the rover's local frame. +column is north, +row is west.
/// The rover sits in free space at the centre. Beyond the sensed radius is unknown.
/// A north wall and a west wall, each with a gap, plus a small obstacle, are occupied.
pub fn occupancy_cell(id: u64, column: u32, row: u32) -> i8 {
    let x = cell_offset(column);
    let y = cell_offset(row);
    let sensed = 5.4;
    if x * x + y * y > sensed * sensed {
        return -1;
    }
    let gap = ((id % 5) as f64 - 2.0) * 0.8;
    if (2.0..3.0).contains(&x) && (-3.2..3.2).contains(&y) && (y - gap).abs() > 0.6 {
        return 92;
    }
    if (2.0..3.0).contains(&y) && (-3.2..1.2).contains(&x) && (x - gap).abs() > 0.6 {
        return 88;
    }
    let obstacle_x = -1.6 + (id % 3) as f64 * 0.5;
    let obstacle_y = -1.4;
    if (obstacle_x..obstacle_x + 1.0).contains(&x) && (obstacle_y..obstacle_y + 1.0).contains(&y) {
        return 96;
    }
    0
}

pub fn occupancy_cells(id: u64) -> Vec<i8> {
    let mut cells = Vec::with_capacity((OCCUPANCY_SIZE * OCCUPANCY_SIZE) as usize);
    for row in 0..OCCUPANCY_SIZE {
        for column in 0..OCCUPANCY_SIZE {
            cells.push(occupancy_cell(id, column, row));
        }
    }
    cells
}

fn map_entry(id: u64, entry: &serde_json::Value, include_cells: bool) -> serde_json::Value {
    let grid = &entry["grid"];
    let mut value = serde_json::json!({
        "id": rover_label(id),
        "roverId": id,
        "sequence": grid["sequence"].as_u64().unwrap_or(0),
        "runId": grid["run_id"].as_str().unwrap_or(""),
        "age": entry["age"].as_f64().unwrap_or(0.0),
        "width": grid["width"].as_u64().unwrap_or(0),
        "height": grid["height"].as_u64().unwrap_or(0),
        "resolution": grid["resolution"].as_f64().unwrap_or(0.0),
        "originX": grid["origin_x"].as_f64().unwrap_or(0.0),
        "originY": grid["origin_y"].as_f64().unwrap_or(0.0),
    });
    if include_cells {
        value["occupancy"] = grid["occupancy"].clone();
    }
    value
}

fn occupancy_object(raw: &str) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

/// Index without cell payloads. Unity polls a grid only when `sequence` or `runId` changes.
pub fn map_index_json(raw: &str) -> String {
    let object = occupancy_object(raw);
    let mut ids: Vec<u64> = object.keys().filter_map(|key| key.parse().ok()).collect();
    ids.sort_unstable();
    let maps: Vec<_> = ids
        .into_iter()
        .map(|id| map_entry(id, &object[&id.to_string()], false))
        .collect();
    serde_json::json!({ "maps": maps }).to_string()
}

pub fn map_grid_json(raw: &str, rover_id: u64) -> Option<String> {
    let object = occupancy_object(raw);
    let entry = object.get(&rover_id.to_string())?;
    Some(map_entry(rover_id, entry, true).to_string())
}

fn write_buf(buf: *mut c_char, cap: usize, text: &str) -> i32 {
    let needed = text.len().saturating_add(1);
    if buf.is_null() || cap < needed {
        return -(needed.min(i32::MAX as usize) as i32);
    }
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr(), buf.cast(), text.len());
        *buf.add(text.len()) = 0;
    }
    text.len() as i32
}

fn read_cstr<'a>(ptr: *const c_char) -> Result<&'a str, i32> {
    if ptr.is_null() {
        set_error("null string");
        return Err(-1);
    }
    match unsafe { CStr::from_ptr(ptr) }.to_str() {
        Ok(text) => Ok(text),
        Err(_) => {
            set_error("string is not utf-8");
            Err(-2)
        }
    }
}

/// Open a Zenoh client. `endpoint` is `tcp/host:port`. `prefix` is `terra/rover`.
/// Returns 0, or a negative code. The message is available from `argos_bridge_last_error`.
#[unsafe(no_mangle)]
pub extern "C" fn argos_bridge_open(endpoint: *const c_char, prefix: *const c_char) -> i32 {
    let endpoint = match read_cstr(endpoint) {
        Ok(value) => value,
        Err(code) => return code,
    };
    let prefix = match read_cstr(prefix) {
        Ok(value) => value,
        Err(code) => return code,
    };
    if let Some(previous) = take_client() {
        previous.disconnect();
    }
    match Client::connect(endpoint, prefix) {
        Ok(client) => {
            set_error("");
            *BRIDGE.lock().unwrap_or_else(|p| p.into_inner()) = Some(client);
            0
        }
        Err(error) => {
            set_error(error.to_string());
            -3
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn argos_bridge_close() {
    if let Some(client) = take_client() {
        client.disconnect();
    }
}

/// 1 when a session is open, otherwise 0. This does not mean fleet data has arrived.
#[unsafe(no_mangle)]
pub extern "C" fn argos_bridge_connected() -> i32 {
    i32::from(BRIDGE.lock().unwrap_or_else(|p| p.into_inner()).is_some())
}

/// Writes the ARGOS link word: `connecting`, `healthy`, `stale`, or `disconnected`.
#[unsafe(no_mangle)]
pub extern "C" fn argos_bridge_link(buf: *mut c_char, cap: usize) -> i32 {
    let link = BRIDGE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .map(|client| client.snapshot().link)
        .unwrap_or_else(|| "disconnected".to_string());
    write_buf(buf, cap, &link)
}

/// Copy the latest fleet JSON into `buf`, including a trailing NUL.
/// Returns the byte length excluding the NUL, or a negative code.
/// `-1` means there is no session. A larger negative value is the required capacity.
#[unsafe(no_mangle)]
pub extern "C" fn argos_bridge_poll(buf: *mut c_char, cap: usize) -> i32 {
    let guard = BRIDGE.lock().unwrap_or_else(|p| p.into_inner());
    let Some(client) = guard.as_ref() else {
        set_error("not connected");
        return -1;
    };
    let snapshot = client.snapshot();
    // The spike draws every rover's grid, so it focuses the whole fleet.
    let _ = client.focus(
        snapshot
            .rovers
            .iter()
            .filter(|rover| rover.membership != "absent")
            .map(|rover| rover.id),
    );
    let operator = serde_json::from_str::<serde_json::Value>(&client.operator_snapshot())
        .unwrap_or_else(|_| serde_json::json!({}));
    let json = fleet_json(&snapshot, &operator, unix_now());
    write_buf(buf, cap, &json)
}

/// Per-rover occupancy index: id, sequence, run id, age, origin, and size. No cells.
#[unsafe(no_mangle)]
pub extern "C" fn argos_bridge_poll_map_index(buf: *mut c_char, cap: usize) -> i32 {
    let guard = BRIDGE.lock().unwrap_or_else(|p| p.into_inner());
    let Some(client) = guard.as_ref() else {
        set_error("not connected");
        return -1;
    };
    write_buf(buf, cap, &map_index_json(&client.occupancy_snapshot()))
}

/// One rover grid, including cells. `-4` means that rover has no grid yet.
#[unsafe(no_mangle)]
pub extern "C" fn argos_bridge_poll_map(rover_id: u64, buf: *mut c_char, cap: usize) -> i32 {
    let guard = BRIDGE.lock().unwrap_or_else(|p| p.into_inner());
    let Some(client) = guard.as_ref() else {
        set_error("not connected");
        return -1;
    };
    let Some(json) = map_grid_json(&client.occupancy_snapshot(), rover_id) else {
        set_error("no occupancy grid");
        return -4;
    };
    write_buf(buf, cap, &json)
}

#[unsafe(no_mangle)]
pub extern "C" fn argos_bridge_last_error(buf: *mut c_char, cap: usize) -> i32 {
    let message = LAST_ERROR.lock().unwrap_or_else(|p| p.into_inner()).clone();
    write_buf(buf, cap, &message)
}

pub fn c_string(text: &str) -> CString {
    CString::new(text).expect("interior nul")
}

#[cfg(test)]
#[path = "../../../../../tests/argos-unity-bridge/unit/lib_tests.rs"]
mod tests;
