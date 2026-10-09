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
