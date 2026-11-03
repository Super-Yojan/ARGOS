//! The Zenoh subscriptions a `Client` opens on connect, one function each.
//!
//! Every callback writes into a shared cache that `Client` reads later. Most
//! are declared here at connect; the `*_handler` ones are kept so `focus` can
//! subscribe the selected rover's heavy topics on demand.

use crate::{
    AUTHORITY_TOPICS, Handler, InputState, SharedLocalizations, SharedMaps, SharedTelemetry,
    SharedTracking, content_heartbeat, session_log, utc,
};
use argos_core::*;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use zenoh::Wait;
use zenoh::pubsub::Subscriber;

/// Zenoh client settings for one router endpoint (`tcp/` or `tls/`), with
/// optional user/password. Multicast scouting is off.
pub(crate) fn client_config(
    endpoint: &str,
    credentials: Option<(&str, &str)>,
) -> Result<zenoh::Config> {
    if endpoint.len() > 512
        || !(endpoint.starts_with("tcp/") || endpoint.starts_with("tls/"))
        || endpoint.parse::<zenoh::config::EndPoint>().is_err()
    {
        return Err(invalid(
            "endpoint must be a valid tcp/host:port or tls/host:port",
        ));
    }
    let mut config = zenoh::Config::default();
    config
        .insert_json5("mode", "\"client\"")
        .map_err(|e| Error(e.to_string()))?;
    config
        .insert_json5(
            "connect/endpoints",
            &serde_json::to_string(&vec![endpoint]).unwrap(),
        )
        .map_err(|e| Error(e.to_string()))?;
    config
        .insert_json5("scouting/multicast/enabled", "false")
        .map_err(|e| Error(e.to_string()))?;
    config
        .insert_json5("connect/timeout_ms", "3000")
        .map_err(|e| Error(e.to_string()))?;
    if let Some((user, password)) = credentials {
        for (key, value) in [
            ("transport/auth/usrpwd/user", user),
            ("transport/auth/usrpwd/password", password),
        ] {
            config
                .insert_json5(key, &serde_json::to_string(value).unwrap())
                .map_err(|e| Error(e.to_string()))?;
        }
    }
    Ok(config)
}

/// `<prefix>/fleet`: fleet membership into the cache.
pub(crate) fn subscribe_fleet(
    session: &zenoh::Session,
    topics: &Topics,
    cache: &Arc<Mutex<FleetCache>>,
    start: Instant,
) -> Result<Subscriber<()>> {
    let fleet_cache = cache.clone();
    session
        .declare_subscriber(topics.fleet())
        .callback(move |sample| {
            let payload = sample.payload().to_bytes();
            let mut cache = fleet_cache.lock().unwrap();
            match decode_fleet(&payload) {
                Ok(f) => cache.fleet(f, start.elapsed().as_secs_f64()),
                Err(e) => cache.notice(e.to_string()),
            }
        })
        .wait()
        .map_err(|e| Error(e.to_string()))
}

/// `<prefix>/*/goal/status`: goal acknowledgements into the cache.
pub(crate) fn subscribe_goal_status(
    session: &zenoh::Session,
    topics: &Topics,
    cache: &Arc<Mutex<FleetCache>>,
    start: Instant,
) -> Result<Subscriber<()>> {
    let fleet_cache = cache.clone();
    let subscribed_topics = topics.clone();
    session
        .declare_subscriber(topics.statuses())
        .callback(move |sample| {
            let Some(id) =
                subscribed_topics.rover_from_key(sample.key_expr().as_str(), "/goal/status")
            else {
                return;
            };
            let payload = sample.payload().to_bytes();
            let mut cache = fleet_cache.lock().unwrap();
            match decode_status(&payload) {
                Ok(s) => cache.status(id, s, start.elapsed().as_secs_f64()),
                Err(e) => cache.notice(e.to_string()),
            }
        })
        .wait()
        .map_err(|e| Error(e.to_string()))
}

/// `camera/depth` (focused rover only): its pose and tracking flag.
pub(crate) fn depth_handler(
    topics: &Topics,
    cache: &Arc<Mutex<FleetCache>>,
    tracking: &SharedTracking,
    start: Instant,
) -> Handler {
    let fleet_cache = cache.clone();
    let subscribed_topics = topics.clone();
    let depth_tracking = tracking.clone();
    Arc::new(move |sample: zenoh::sample::Sample| {
        let Some(id) =
            subscribed_topics.rover_from_key(sample.key_expr().as_str(), "/camera/depth")
        else {
            return;
        };
        let payload = sample.payload().to_bytes();
        let flag = serde_json::from_slice::<serde_json::Value>(&payload)
            .ok()
            .as_ref()
            .and_then(pose_tracking_flag);
        let mut cache = fleet_cache.lock().unwrap();
        match decode_pose(&payload) {
            Ok(Some(p)) => {
                depth_tracking.lock().unwrap().insert(id, flag);
                cache.pose(id, p, start.elapsed().as_secs_f64());
            }
            Ok(None) => {}
            Err(e) => cache.notice(e.to_string()),
        }
    })
}

/// `<prefix>/*/pose`: every rover's pose and tracking flag.
pub(crate) fn subscribe_pose(
    session: &zenoh::Session,
    topics: &Topics,
    cache: &Arc<Mutex<FleetCache>>,
    tracking: &SharedTracking,
    start: Instant,
) -> Result<Subscriber<()>> {
    let direct_cache = cache.clone();
    let direct_topics = topics.clone();
    let pose_tracking = tracking.clone();
    session
        .declare_subscriber(format!("{}/*/pose", topics.prefix))
        .callback(move |sample| {
            let bytes = sample.payload().to_bytes();
            if let Some(id) = direct_topics.rover_from_key(sample.key_expr().as_str(), "/pose")
                && let Ok(pose) = decode_topic_pose(&bytes, Some(id))
            {
                let flag = serde_json::from_slice::<serde_json::Value>(&bytes)
                    .ok()
                    .as_ref()
                    .and_then(pose_tracking_flag);
                pose_tracking.lock().unwrap().insert(id, flag);
                direct_cache
                    .lock()
                    .unwrap()
                    .pose(id, pose, start.elapsed().as_secs_f64());
            }
        })
        .wait()
        .map_err(|e| Error(e.to_string()))
}

/// `map/occupancy` (focused rover only): keep the newest grid per rover,
/// tagged with the localization frame it was drawn in.
pub(crate) fn occupancy_handler(
    topics: &Topics,
    localizations: &SharedLocalizations,
    maps: &SharedMaps,
    start: Instant,
) -> Handler {
    let map_frames = localizations.clone();
    let grids = maps.clone();
    let map_topics = topics.clone();
    Arc::new(move |sample: zenoh::sample::Sample| {
        let Some(id) = map_topics.rover_from_key(sample.key_expr().as_str(), "/map/occupancy")
        else {
            return;
        };
        let Ok(grid) = decode_occupancy(&sample.payload().to_bytes()) else {
            return;
        };
        if grid.rover_id != id {
            return;
        }
        let frames = map_frames.lock().unwrap();
        let frame_id = frames
            .get(&id)
            .and_then(|(v, _)| v["frameID"].as_str())
            .map(String::from);
        let mut maps = grids.lock().unwrap();
        if maps
            .get(&id)
            .is_some_and(|(old, _, _)| old.run_id == grid.run_id && old.sequence >= grid.sequence)
        {
            return;
        }
        // Keep memory bounded even if an endpoint publishes unlisted IDs.
        if !maps.contains_key(&id) && maps.len() >= 256 {
            return;
        }
        maps.insert(id, (grid, start.elapsed().as_secs_f64(), frame_id));
    })
}

/// Start this session's log at `log_dir/argos-session-<uuid>.jsonl`.
/// A folder that cannot be written gives a recorder that drops events.
pub(crate) fn open_session_log(
    log_dir: &std::path::Path,
) -> (String, std::path::PathBuf, Arc<session_log::RunRecorder>) {
    let session_id = uuid::Uuid::new_v4().to_string();
    let _ = std::fs::create_dir_all(log_dir);
    let log_path = log_dir.join(format!("argos-session-{session_id}.jsonl"));
    let recorder = Arc::new(
        session_log::RunRecorder::create(
            &log_path,
            serde_json::json!({
                "session_id": session_id,
                "utc": utc(),
                "platform": "argos",
                "clock": "session_relative_monotonic",
            }),
            8192,
        )
        .unwrap_or_else(|_| session_log::RunRecorder::unavailable()),
    );
    (session_id, log_path, recorder)
}

/// `AUTHORITY_TOPICS` for every rover: run and authority state. A new run
/// drops that rover's held keys.
pub(crate) fn authority_handler(
    topics: &Topics,
    cache: &Arc<Mutex<FleetCache>>,
    operator: &Arc<Mutex<OperatorCache>>,
    held: &Arc<Mutex<InputState>>,
    recorder: &Arc<session_log::RunRecorder>,
    session_id: &str,
    start: Instant,
) -> Handler {
    let restart_input = held.clone();
    let authority_cache = operator.clone();
    let log = recorder.clone();
    let subscribed_topics = topics.clone();
    let fleet_cache = cache.clone();
    let callback_session_id = session_id.to_string();
    Arc::new(move |sample: zenoh::sample::Sample| {
        for kind in AUTHORITY_TOPICS {
            let Some(id) =
                subscribed_topics.rover_from_key(sample.key_expr().as_str(), &format!("/{kind}"))
            else {
                continue;
            };
            if !fleet_cache
                .lock()
                .unwrap()
                .snapshot(start.elapsed().as_secs_f64())
                .rovers
                .iter()
                .any(|r| r.id == id && r.membership != "absent")
            {
                return;
            }
            let bytes = sample.payload().to_bytes();
            let now = start.elapsed().as_secs_f64();
            let (ok, changed) = {
                let mut state = authority_cache.lock().unwrap();
                let previous = state.run_id(id);
                let ok = state.apply(id, kind, &bytes, now).is_ok();
                (ok, previous != state.run_id(id))
            };
            if changed {
                let mut input = restart_input.lock().unwrap();
                if input.rover_id == Some(id) {
                    input.rover_id = None;
                    input.keys.clear();
                    input.authority_generation = None;
                }
            }
            if ok {
                let _ = log.record(serde_json::json!({
                    "kind": "state_observed",
                    "topic": kind,
                    "rover_id": id,
                    "time": now,
                    "utc": utc(),
                    "session_id": callback_session_id,
                    "payload": serde_json::from_slice::<serde_json::Value>(&bytes).ok(),
                }));
            }
            return;
        }
    })
}

/// `<prefix>/*/localization`: the frame each rover's grid is drawn in. A
/// frame change drops that rover's cached grid.
pub(crate) fn subscribe_localization(
    session: &zenoh::Session,
    topics: &Topics,
    localizations: &SharedLocalizations,
    maps: &SharedMaps,
) -> Result<Subscriber<()>> {
    let l = localizations.clone();
    let frame_grids = maps.clone();
    let subscribed_topics = topics.clone();
    session
        .declare_subscriber(format!("{}/*/localization", topics.prefix))
        .callback(move |sample| {
            let Some(id) =
                subscribed_topics.rover_from_key(sample.key_expr().as_str(), "/localization")
            else {
                return;
            };
            let bytes = sample.payload().to_bytes();
            if bytes.len() > 4096 {
                return;
            }
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                return;
            };
            if value.get("version").and_then(|v| v.as_u64()) != Some(1) || !value.is_object() {
                return;
            }
            let mut entries = l.lock().unwrap();
            let previous = entries.get(&id).and_then(|(v, _)| v["frameID"].as_str());
            if previous != value["frameID"].as_str() {
                frame_grids.lock().unwrap().remove(&id);
            }
            entries.retain(|_, (_, received): &mut (serde_json::Value, Instant)| {
                received.elapsed().as_secs_f64() < SUPERVISION_STALE_SECONDS
            });
            if entries.len() < 4096 || entries.contains_key(&id) {
                entries.insert(id, (value, Instant::now()));
            }
        })
        .wait()
        .map_err(|e| Error(e.to_string()))
}

/// `STATE_TOPICS` for every rover and `DETAIL_TOPICS` for the focused one:
/// latest JSON per (rover, kind), plus heartbeats that refresh cached grids
/// and clouds.
pub(crate) fn telemetry_handler(
    topics: &Topics,
    operator_states: &SharedTelemetry,
    maps: &SharedMaps,
    recorder: &Arc<session_log::RunRecorder>,
    session_id: &str,
    start: Instant,
) -> Handler {
    let telemetry_cache = operator_states.clone();
    let heartbeat_maps = maps.clone();
    let subscribed_topics = topics.clone();
    let search_log = recorder.clone();
    let search_session = session_id.to_string();
    Arc::new(move |sample: zenoh::sample::Sample| {
        let key = sample.key_expr().as_str();
        let suffix = if key.ends_with("/map/occupancy/heartbeat") {
            "/map/occupancy/heartbeat"
        } else if key.ends_with("/pointcloud/heartbeat") {
            "/pointcloud/heartbeat"
        } else if key.ends_with("/hardware/status") {
            "/hardware/status"
        } else if key.ends_with("/pointcloud") {
            "/pointcloud"
        } else if key.ends_with("/autonomy/status") {
            "/autonomy/status"
        } else if key.ends_with("/pose/quality") {
            "/pose/quality"
        } else if key.ends_with("/navigation/outdoor") {
            "/navigation/outdoor"
        } else if key.ends_with("/search/report") {
            "/search/report"
        } else if key.ends_with("/search/status") {
            "/search/status"
        } else if key.ends_with("/objects") {
            "/objects"
        } else {
            return;
        };
        let Some(id) = subscribed_topics.rover_from_key(key, suffix) else {
            return;
        };
        let bytes = sample.payload().to_bytes();
        if suffix == "/map/occupancy/heartbeat" {
            if let Some(heartbeat) = content_heartbeat::occupancy_identity(&bytes, id)
                && let Some(cached) = heartbeat_maps.lock().unwrap().get_mut(&id)
            {
                content_heartbeat::refresh_grid(cached, &heartbeat, start.elapsed().as_secs_f64());
            }
            return;
        }
        if suffix == "/pointcloud/heartbeat" {
            if let Some(heartbeat) = content_heartbeat::cloud_identity(&bytes, id)
                && let Some(cached) = telemetry_cache
                    .lock()
                    .unwrap()
                    .get_mut(&(id, "cloud".into()))
            {
                content_heartbeat::refresh_cloud(cached, &heartbeat, Instant::now());
            }
            return;
        }
        if bytes.len() > 262144 {
            return;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return;
        };
        if !value.is_object() {
            return;
        }
        if matches!(
            suffix,
            "/search/status" | "/search/report" | "/hardware/status"
        ) {
            let _ = search_log.record(serde_json::json!({
                "kind": "state_observed",
                "topic": suffix.trim_start_matches('/'),
                "rover_id": id,
                "time": start.elapsed().as_secs_f64(),
                "utc": utc(),
                "session_id": search_session,
                "payload": value,
            }));
        }
        let kind = if suffix == "/search/report" {
            let Some(search) = value["search_id"].as_str() else {
                return;
            };
            if !valid_token(search) {
                return;
            }
            // Each search retains one report; batches replay on the canonical topic.
            let kind = format!("search_report:{search}");
            let mut entries = telemetry_cache.lock().unwrap();
            entries.retain(|_, (_, at)| at.elapsed().as_secs_f64() < 2.5);
            if entries.len() < 4096 || entries.contains_key(&(id, kind.clone())) {
                entries.insert((id, kind), (value, Instant::now()));
            }
            return;
        } else if suffix == "/search/status" {
            "search"
        } else if suffix == "/hardware/status" {
            "hardware"
        } else if suffix == "/pointcloud" {
            "cloud"
        } else if suffix == "/map/occupancy" {
            "occupancy"
        } else if suffix == "/pose/quality" {
            "pose_quality"
        } else if suffix == "/navigation/outdoor" {
            "outdoor"
        } else if suffix == "/objects" {
            "objects"
        } else {
            "authority"
        };
        let mut entries = telemetry_cache.lock().unwrap();
        entries.retain(|_, (_, at): &mut (serde_json::Value, Instant)| {
            at.elapsed().as_secs_f64() < SUPERVISION_STALE_SECONDS
        });
        if kind == "cloud"
            && entries
                .get(&(id, kind.into()))
                .is_some_and(|(old, _)| !content_heartbeat::accepts_cloud(old, &value))
        {
            return;
        }
        if entries.len() < 4096 || entries.contains_key(&(id, kind.into())) {
            entries.insert((id, kind.into()), (value, Instant::now()));
        }
    })
}
