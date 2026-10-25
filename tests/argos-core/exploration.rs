use argos_core::*;
use serde_json::{Value, json};

#[test]
fn explore_request_carries_a_budget() {
    let bytes = encode_explore_request("explore-1", 300.).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({"level": "explore", "token": "explore-1", "budget_seconds": 300.0})
    );
    assert!(encode_explore_request("explore-1", 0.).is_err());
    assert!(encode_explore_request("explore-1", f64::NAN).is_err());
    assert!(encode_explore_request("explore-1", EXPLORE_MAX_BUDGET_SECONDS + 1.).is_err());
    assert!(encode_explore_request("", 300.).is_err());
    // The budgetless encoder cannot start a run Terra would reject.
    assert!(encode_level_request("explore", "explore-1").is_err());
    assert_eq!(level_label("explore"), "L4 Explore");
}

#[test]
fn operator_payload_admits_explore_only_with_a_budget() {
    let ok = operator_payload(
        "autonomy",
        json!({"level": "explore", "budget_seconds": 180.0}),
        "tok-1",
    )
    .unwrap();
    let value: Value = serde_json::from_slice(&ok).unwrap();
    assert_eq!(value["budget_seconds"], 180.0);
    assert!(operator_payload("autonomy", json!({"level": "explore"}), "tok-1").is_err());
    assert!(
        operator_payload(
            "autonomy",
            json!({"level": "waypoint", "budget_seconds": 180.0}),
            "tok-1"
        )
        .is_err()
    );
}

#[test]
fn stop_and_outdoor_source_refuse_explore() {
    assert_eq!(explore_refusal(false, false), None);
    assert_eq!(
        explore_refusal(true, false),
        Some("Reset stop before exploring")
    );
    assert_eq!(
        explore_refusal(false, true),
        Some("Switch navigation to Local before exploring")
    );
}

fn status(exploration: Value, reason: &str) -> Value {
    json!({"requested_level": "explore", "reason": reason, "exploration": exploration})
}

#[test]
fn running_status_shows_target_progress_and_stuck() {
    let value = status(
        json!({
            "run_id": "run", "phase": "running", "elapsed_seconds": 65.2,
            "remaining_seconds": 234.8, "budget_seconds": 300.0,
            "target": {"x": 3.25, "y": -1.5, "score": 4.0},
            "coverage_m2": 42.4, "explored_ratio": 0.31, "end_reason": null,
            "blacklist_count": 1, "frontiers_attempted": 4
        }),
        "follow",
    );
    let report = decode_exploration(&value).unwrap();
    assert!(report.running());
    assert_eq!(report.target, Some((3.25, -1.5)));
    assert_eq!(report.state_label(), "Exploring");
    let rows = report.rows();
    assert_eq!(rows[1].1, "1:05 of 5:00 · 3:55 left");
    assert_eq!(rows[2].1, "3.2, -1.5 m");
    assert_eq!(rows[3].1, "42 m² free · 31% of map seen");
    assert_eq!(rows[4].1, "4 tried · 1 skipped as stuck");

    let mut stuck = value.clone();
    stuck["reason"] = "frontier_retry".into();
    assert_eq!(
        decode_exploration(&stuck).unwrap().state_label(),
        "Stuck · retrying another frontier"
    );
}

#[test]
fn ended_status_names_why() {
    let ended = |reason: &str| {
        decode_exploration(&status(
            json!({"phase": "ended", "elapsed_seconds": 90.0, "remaining_seconds": 0.0,
                   "budget_seconds": 90.0, "target": null, "coverage_m2": 10.0,
                   "explored_ratio": 1.0, "end_reason": reason,
                   "blacklist_count": 0, "frontiers_attempted": 0}),
            reason,
        ))
        .unwrap()
    };
    assert!(!ended("no_frontiers_left").running());
    assert_eq!(
        ended("no_frontiers_left").state_label(),
        "Done · no frontiers left"
    );
    assert_eq!(
        ended("budget_expired").state_label(),
        "Done · time budget used"
    );
    assert_eq!(ended("safety_hold").state_label(), "Ended · safety hold");
    assert_eq!(ended("operator_stop").rows()[2].1, "None");
}

#[test]
fn idle_or_missing_exploration_is_none() {
    assert!(decode_exploration(&json!({"requested_level": "waypoint"})).is_none());
    assert!(decode_exploration(&status(json!({"phase": "idle"}), "idle")).is_none());
    assert!(decode_exploration(&status(json!("bad"), "idle")).is_none());
}
