//! The `argos-fleet` background thread: connect, reconnect every 2 s, and
//! refresh `FleetPicture` from the Zenoh snapshot every 100 ms.

use super::config::DashboardConfig;
use super::config::session_dir;
use super::feed::unix_millis;
use super::marks::FleetPicture;
use super::marks::LocalizationHistory;
use super::marks::describe;
use super::marks::display_link;
use super::marks::display_policy;
use super::marks::fleet_presence;
use super::marks::placed_fleet_marks;
use super::marks::retain_geographic_markers;
use super::marks::scope_localization;
use super::overlay::OverlayFrames;
use argos_core::{
    FrameAligner, LocalFrame, PoseWatch, decode_raw_gps, fallback_degrade_reason, reported_outdoor,
};
use argos_zenoh::Client;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(super) fn run(
    config: DashboardConfig,
    picture: Arc<Mutex<FleetPicture>>,
    slot: Arc<Mutex<Option<Arc<Client>>>>,
    focus: Arc<Mutex<Option<u64>>>,
    stopped: Arc<AtomicBool>,
    gps: Arc<AtomicBool>,
) {
    let mut history = LocalizationHistory::default();
    let started = std::time::Instant::now();
    while !stopped.load(Ordering::Acquire) {
        let credentials = config
            .credentials
            .as_ref()
            .map(|(user, password)| (user.as_str(), password.as_str()));
        match Client::connect_logged(
            &config.endpoint,
            &config.prefix,
            credentials,
            &session_dir(),
        ) {
            Ok(client) => {
                let client = Arc::new(client);
                if stopped.load(Ordering::Acquire) {
                    client.disconnect();
                    return;
                }
                *slot.lock().unwrap() = Some(client.clone());
                poll(
                    &client,
                    &config,
                    &picture,
                    &focus,
                    &mut history,
                    started,
                    &stopped,
                    &gps,
                );
                if stopped.load(Ordering::Acquire) {
                    *slot.lock().unwrap() = None;
                    client.disconnect();
                    return;
                }
                mark_disconnected(&picture);
                *slot.lock().unwrap() = None;
            }
            Err(error) => {
                let mut current = picture.lock().unwrap();
                current.link = "disconnected".into();
                current.detail = error.to_string();
                for mark in &mut current.marks {
                    mark.status = "offline".into();
                    mark.gps_source_fresh = false;
                }
                for row in &mut current.presence {
                    row.membership = "offline".into();
                }
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

pub(super) fn mark_disconnected(picture: &Mutex<FleetPicture>) {
    let mut current = picture.lock().unwrap();
    current.link = "disconnected".into();
    current.detail = format!(
        "Disconnected · {} last known rovers",
        current.presence.len()
    );
    for mark in &mut current.marks {
        mark.status = "offline".into();
        mark.gps_source_fresh = false;
    }
    for row in &mut current.presence {
        row.membership = "offline".into();
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn poll(
    client: &Client,
    config: &DashboardConfig,
    picture: &Mutex<FleetPicture>,
    focus: &Mutex<Option<u64>>,
    history: &mut LocalizationHistory,
    started: std::time::Instant,
    stopped: &AtomicBool,
    gps: &AtomicBool,
) {
    let mut watches: BTreeMap<u64, PoseWatch> = BTreeMap::new();
    let mut reporting_holds: BTreeMap<u64, argos_core::ReportingHold> = BTreeMap::new();
    let mut aligners: BTreeMap<u64, FrameAligner> = BTreeMap::new();
    let mut overlays = OverlayFrames::default();
    let issued_after_ms = unix_millis();
    let mut last_receipts: BTreeMap<u64, String> = BTreeMap::new();
    let mut last_refusal: Option<String> = None;
    while !stopped.load(Ordering::Acquire) {
        // A failed declare is retried on the next pass.
        let _ = client.focus(*focus.lock().unwrap());
        let mut snapshot = client.snapshot();
        if snapshot.link == "failed" || snapshot.link == "disconnected" {
            break;
        }
        // A goal refusal is otherwise silent: show it in the status line.
        let refusal = snapshot
            .notice
            .clone()
            .filter(|notice| notice.contains("refused the goal"));
        if let Some(notice) = &refusal
            && last_refusal.as_ref() != Some(notice)
        {
            picture.lock().unwrap().command = notice.clone();
        }
        last_refusal = refusal;
        for (id, result) in client.resend_refused_goals() {
            picture.lock().unwrap().command = match result {
                Ok(_) => format!("Rover {id}: goal resent with the current authority"),
                Err(error) => format!("Rover {id}: goal resend failed · {error}"),
            };
        }
        let localizations =
            serde_json::from_str::<Value>(&client.localization_snapshot()).unwrap_or(Value::Null);
        if snapshot.fleet_age.is_none() {
            // A replacement client has not received authoritative membership yet.
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        let ids = snapshot
            .rovers
            .iter()
            .filter(|r| r.membership != "absent")
            .map(|r| r.id)
            .collect();
        history.observe_membership(
            &snapshot
                .rovers
                .iter()
                .map(|r| (r.id, r.membership.as_str()))
                .collect::<Vec<_>>(),
            started.elapsed().as_secs_f64(),
        );
        history.retain(&ids);
        let operator =
            serde_json::from_str::<Value>(&client.operator_state_snapshot()).unwrap_or(Value::Null);
        history.scope_runs(&operator);
        let mut localizations = localizations;
        if let Some(reports) = localizations.as_object_mut() {
            for (id, report) in reports {
                *report = scope_localization(
                    report.clone(),
                    operator[id]["authority"]["run_id"].as_str(),
                );
            }
        }
        let localizations = history.update(&localizations, started.elapsed().as_secs_f64());
        history.retain(&ids);
        let use_gps = gps.load(Ordering::Relaxed);
        if !use_gps {
            // Start the GPS fit over when GPS is turned back on.
            aligners.clear();
            overlays.retain(&Default::default());
        } else if config.steady_map {
            align(
                &mut aligners,
                &snapshot,
                &localizations,
                &operator,
                started.elapsed().as_secs_f64(),
            );
        }
        aligners.retain(|id, _| ids.contains(id));
        overlays.retain(&ids);
        let mut marks = placed_fleet_marks(
            &snapshot,
            &localizations,
            &operator,
            |id| client.occupancy_grid(id).map(|(grid, age, _)| (grid, age)),
            config.origin,
            |id| aligners.get(&id).and_then(FrameAligner::frame),
            &mut overlays,
            started.elapsed().as_secs_f64(),
            use_gps,
        );
        snapshot.link = display_link(&snapshot, &localizations, &operator);
        if use_gps {
            retain_geographic_markers(
                &snapshot,
                &localizations,
                &picture.lock().unwrap(),
                &mut marks,
            );
        }
        let now = started.elapsed().as_secs_f64();
        // Short telemetry gaps must not flicker the rover and its notices.
        reporting_holds.retain(|id, _| ids.contains(id));
        for mark in &mut marks {
            mark.reporting = reporting_holds
                .entry(mark.id)
                .or_default()
                .observe(mark.reporting, now);
        }
        for mark in &mut marks {
            if mark.quality_reported {
                continue;
            }
            let (degraded, reason) = watches.entry(mark.id).or_default().observe(
                mark.yaw,
                mark.sequence,
                now,
                client.pose_tracking(mark.id),
                config.degrade,
            );
            mark.degraded = degraded;
            mark.degrade_reason = if degraded {
                fallback_degrade_reason(reason)
            } else {
                String::new()
            };
        }
        {
            let mut current = picture.lock().unwrap();
            if let Some(entries) = operator.as_object() {
                for (id, state) in entries {
                    let Ok(id) = id.parse::<u64>() else { continue };
                    if let Some((receipt, message)) =
                        argos_core::vehicle_request_receipt(&state["authority"], issued_after_ms)
                        && last_receipts.get(&id) != Some(&receipt)
                    {
                        last_receipts.insert(id, receipt);
                        current.command = format!("Rover {id}: {message}");
                    }
                }
            }
            current.detail = describe(&snapshot, &marks);
            current.presence = fleet_presence(&snapshot, &marks);
            for row in &mut current.presence {
                if matches!(row.membership.as_str(), "online" | "stale") {
                    let id = row.id.to_string();
                    let limit = display_policy(&localizations[&id], &operator[&id]["authority"])
                        .fleet_display_stale_seconds;
                    row.membership = if snapshot.fleet_age.is_some_and(|age| age >= limit) {
                        "stale".into()
                    } else {
                        "online".into()
                    };
                }
                row.source_reason = localizations[row.id.to_string()]["reason"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned();
            }
            current.link = snapshot.link;
            current.marks = marks;
            current.observed_at = std::time::Instant::now();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Feed each phone rover's pose, fused heading and GPS fixes to its aligner.
/// A rover whose phone publishes its own geographic alignment needs none.
pub(super) fn align(
    aligners: &mut BTreeMap<u64, FrameAligner>,
    snapshot: &argos_core::Snapshot,
    localizations: &Value,
    operator: &Value,
    now: f64,
) {
    for rover in &snapshot.rovers {
        let Some(pose) = &rover.pose else {
            continue;
        };
        let key = rover.id.to_string();
        let report = &localizations[&key];
        if report.is_null() || LocalFrame::from_localization(report).is_some() {
            aligners.remove(&rover.id);
            continue;
        }
        // A tracking restart gives a new frame ID; a new run a new run ID.
        let frame = format!(
            "{}|{}",
            report["frameID"].as_str().unwrap_or(""),
            report["run_id"].as_str().unwrap_or("")
        );
        let aligner = aligners.entry(rover.id).or_default();
        let pose_time = now - rover.pose_age.unwrap_or(0.).max(0.);
        aligner.observe_pose(&frame, pose_time, pose.x, pose.y);
        if let Some(outdoor) = reported_outdoor(&operator[&key])
            && let Some(heading) = outdoor.heading_deg
        {
            aligner.observe_heading(now, pose.yaw, heading, outdoor.heading_accuracy_deg);
        }
        if let Some(gps) = decode_raw_gps(report)
            && let Some(observed_at) = report["gpsObservedAt"].as_f64()
        {
            aligner.observe_fix(
                now,
                observed_at,
                gps.age_s.unwrap_or(f64::INFINITY),
                gps.latitude,
                gps.longitude,
                gps.accuracy_m.unwrap_or(f64::INFINITY),
            );
        }
    }
}
