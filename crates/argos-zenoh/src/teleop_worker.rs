//! The held-input worker: every 50 ms it turns the operator's held keys into
//! one teleop twist for the driven rover, or a zero twist once driving stops.
//! It sends only while the rover's authority, motor relay, and link allow it.

use crate::{
    FleetCache, InputState, SharedTelemetry, require_relay, session_log, utc, zero_payload,
};
use argos_core::*;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use zenoh::Wait;

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_teleop_worker(
    session: &Arc<Mutex<Option<zenoh::Session>>>,
    held: &Arc<Mutex<InputState>>,
    operator: &Arc<Mutex<OperatorCache>>,
    operator_states: &SharedTelemetry,
    cache: &Arc<Mutex<FleetCache>>,
    shutdown: &Arc<std::sync::atomic::AtomicBool>,
    teleop_sequence: &Arc<std::sync::atomic::AtomicU64>,
    topics: &Topics,
    session_id: &str,
    recorder: &Arc<session_log::RunRecorder>,
    start: Instant,
) -> std::thread::JoinHandle<()> {
    let packet_sequence = teleop_sequence.clone();
    let session = session.clone();
    let held = held.clone();
    let authority_cache = operator.clone();
    let hardware_cache = operator_states.clone();
    let fleet_cache = cache.clone();
    let stop = shutdown.clone();
    let prefix = topics.prefix.clone();
    let callback_session_id = session_id.to_string();
    let log = recorder.clone();
    std::thread::spawn(move || {
        let mut last: Option<u64> = None;
        while !stop.load(std::sync::atomic::Ordering::Acquire) {
            let now = start.elapsed().as_secs_f64();
            let mut input = held.lock().unwrap();
            let active = input.rover_id.filter(|id| {
                let authority = authority_cache.lock().unwrap();
                let permitted = authority.can_drive(*id, now)
                    && input
                        .authority_generation
                        .as_ref()
                        .is_none_or(|(run, revision)| {
                            authority.matches_authority(*id, run, *revision)
                        });
                drop(authority);
                let hardware_ready = hardware_cache
                    .lock()
                    .unwrap()
                    .get(&(*id, "hardware".into()))
                    .is_some_and(|(value, at)| require_relay(value, at, false).is_ok());
                permitted
                    && hardware_ready
                    && fleet_cache.lock().unwrap().snapshot(now).link == "healthy"
            });
            if active.is_none() {
                input.rover_id = None;
                input.keys.clear();
                if let Some(guard) = input.guard.as_mut() {
                    guard.reset();
                }
            }
            if input.deadman && input.lease_until > 0. && now >= input.lease_until {
                input.keys.clear();
            }
            let dt = if input.last_step > 0. {
                now - input.last_step
            } else {
                0.05
            };
            input.last_step = now;
            let (unit_linear, unit_angular) = input.keys.sample();
            // Defense in depth; Terra enforces the real limits.
            let sample = if let Some(guard) = input.guard.as_mut() {
                guard.step(unit_linear, unit_angular, dt)
            } else {
                (unit_linear, unit_angular)
            };
            if let Some(id) = active {
                if sample != (0., 0.) || last.is_some() {
                    let sequence =
                        packet_sequence.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    let payload = serde_json::json!({
                        "linear": sample.0,
                        "angular": sample.1,
                        "operator_session_id": callback_session_id,
                        "sequence": sequence,
                        "run_id": input.authority_generation.as_ref().map(|g|&g.0),
                        "authority_revision": input.authority_generation.as_ref().map(|g|g.1),
                    });
                    if let Some(s) = session.lock().unwrap().as_ref() {
                        let _ = s
                            .put(format!("{prefix}/{id}/teleop"), payload.to_string())
                            .wait();
                    }
                    let _ = log.record(serde_json::json!({
                        "kind": "teleop_sent",
                        "rover_id": id,
                        "time": now,
                        "utc": utc(),
                        "sequence": sequence,
                        "session_id": callback_session_id,
                        "payload": payload,
                    }));
                    last = if sample == (0., 0.) { None } else { Some(id) };
                }
            } else if let Some(id) = last.take()
                && let Some(s) = session.lock().unwrap().as_ref()
            {
                let _ = s
                    .put(
                        format!("{prefix}/{id}/teleop"),
                        zero_payload(
                            &callback_session_id,
                            &packet_sequence,
                            input.authority_generation.as_ref(),
                        ),
                    )
                    .wait();
            }
            drop(input);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    })
}
