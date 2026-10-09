use argos_core::*;
#[test]
fn bounded_search_commands_require_session_and_valid_geometry() {
    let p = r#"{"version":1,"run_id":"run","search_id":"search","token":"start","target_class":"survivor","bounds":{"min_x":-5,"min_y":-5,"max_x":5,"max_y":5},"time_budget_s":60}"#;
    assert!(validate_search_command("search", p).is_ok());
    assert!(validate_search_command("search", &p.replace("survivor", "bad class")).is_err());
    assert!(validate_search_command("search", &p.replace("\"max_x\":5", "\"max_x\":500")).is_err());
    assert!(
        validate_search_command(
            "search/action",
            r#"{"version":1,"run_id":"run","search_id":"s","token":"p","action":"pause"}"#
        )
        .is_ok()
    );
    assert!(validate_search_command("search/observation", p).is_err());
}

#[test]
fn mission_operator_preserves_target_search_authority_without_enabling_teleop() {
    let payload = argos_core::operator_payload(
        "autonomy",
        serde_json::json!({"level":"target_search"}),
        "l4-mode",
    )
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&payload).unwrap()["level"],
        "target_search"
    );
    let mut cache = argos_core::OperatorCache::default();
    cache.apply(1,"autonomy/status",br#"{"run_id":"l4","requested_level":"teleop","effective_level":"teleop","revision":0,"safety":"clear"}"#,0.).unwrap();
    assert!(cache.can_drive(1, 0.01));
    cache.apply(1,"autonomy/status",br#"{"run_id":"l4","requested_level":"target_search","effective_level":"target_search","revision":1,"safety":"clear"}"#,0.).unwrap();
    assert_eq!(
        cache.snapshot(0.1)["1"]["status"]["requested_level"],
        "target_search"
    );
    assert!(!cache.can_drive(1, 0.1));
}
