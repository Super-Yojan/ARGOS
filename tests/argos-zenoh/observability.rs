use argos_zenoh::observability::project_event;
use serde_json::json;
#[test]
fn command_sequence_and_receipt_source_are_preserved() {
    let e = project_event(
        json!({"kind":"teleop_sent","rover_id":4,"payload":{"sequence":55,"linear":0.1},"time":1.2}),
    );
    assert_eq!(e.rover_id, "4");
    assert_eq!(e.kind, "teleop_sent");
    assert_eq!(e.attributes["payload"]["sequence"], 55);
}
#[test]
fn credentials_are_removed_from_nested_fields() {
    let e =
        project_event(json!({"kind":"state","rover_id":4,"payload":{"password":"do-not-store"}}));
    assert!(!serde_json::to_string(&e).unwrap().contains("do-not-store"));
}

#[test]
fn mission_correlation_is_distinct_from_operator_connection() {
    let e = project_event(
        json!({"kind":"teleop_sent","rover_id":4,"session_id":"connection-X","payload":{"run_id":"mission-Y","sequence":12}}),
    );
    assert_eq!(e.session_id.as_deref(), Some("mission-Y"));
    assert_eq!(e.attributes["session_id"], "connection-X");
    assert_eq!(e.attributes["payload"]["run_id"], "mission-Y");
}
