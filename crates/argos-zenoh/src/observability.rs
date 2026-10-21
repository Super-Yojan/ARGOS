//! Projection of existing operator evidence into the shared durable journal.
use serde_json::Value;
use terra_observability::{EventInput, redact};
pub fn project_event(mut value: Value) -> EventInput {
    redact(&mut value);
    let rover = value["rover_id"]
        .as_u64()
        .map(|r| r.to_string())
        .unwrap_or_else(|| "_producer".into());
    let kind = value["kind"]
        .as_str()
        .unwrap_or("operator.event")
        .to_string();
    let mut e = EventInput::new(&rover, "argos", &kind, value.clone());
    // The operator connection remains in attributes; mission identity correlates hops.
    e.session_id = value["payload"]["run_id"]
        .as_str()
        .or_else(|| value["run_id"].as_str())
        .or_else(|| value["payload"]["state"]["run_id"].as_str())
        .map(str::to_string);
    e.monotonic_ns = (value["time"].as_f64().unwrap_or(0.).max(0.) * 1e9) as u64;
    e
}
pub(crate) fn record_value(value: Value) {
    terra_observability::record_runtime_input(project_event(value));
}
