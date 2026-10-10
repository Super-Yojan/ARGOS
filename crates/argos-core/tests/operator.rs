use argos_core::*;
#[test]
fn mode_receipts_and_stale_authority() {
    let mut c = OperatorCache::default();
    c.pending(1, "mode-1".into(), 0.);
    c.apply(1,"autonomy/status",br#"{"requested_level":"teleop","effective_level":"teleop","active_source":"none","safety":"clear","reason":"idle","revision":1,"token":"mode-1","result":"accepted","supported_levels":["teleop"],"paused":false,"assigned_level":null}"#,0.1).unwrap();
    assert!(c.can_drive(1, 0.2));
    assert!(!c.can_drive(1, 3.));
    assert_eq!(c.snapshot(0.2)["1"]["action_phase"], "accepted");
}
#[test]
fn release_and_opposites() {
    let mut t = HeldInput::default();
    t.press("w");
    assert_eq!(t.sample(), (1., 0.));
    t.press("s");
    assert_eq!(t.sample(), (0., 0.));
    t.clear();
    assert_eq!(t.sample(), (0., 0.));
}
#[test]
fn pending_mode_prevents_driving() {
    let mut c = OperatorCache::default();
    let status=br#"{"requested_level":"teleop","effective_level":"teleop","active_source":"none","safety":"clear","reason":"idle","revision":1,"token":"old","result":"accepted","supported_levels":["teleop"],"paused":false,"assigned_level":null}"#;
    c.apply(1, "autonomy/status", status, 0.).unwrap();
    assert!(c.can_drive(1, 0.1));
    c.pending(1, "new-mode".into(), 0.2);
    assert!(!c.can_drive(1, 0.3));
}
#[test]
fn restart_invalidates_cached_authority_and_proposal() {
    let mut c = OperatorCache::default();
    c.apply(
        1,
        "experiment/status",
        br#"{"run_id":"old","run_elapsed":4}"#,
        0.,
    )
    .unwrap();
    c.apply(1,"autonomy/status",br#"{"requested_level":"teleop","effective_level":"teleop","revision":99,"safety":"clear","run_id":"old"}"#,0.).unwrap();
    c.apply(
        1,
        "goal/proposal",
        br#"{"proposal_id":1,"x":1,"y":1,"expires_at":30,"run_id":"old"}"#,
        0.,
    )
    .unwrap();
    c.apply(
        1,
        "experiment/status",
        br#"{"run_id":"new","run_elapsed":0}"#,
        0.1,
    )
    .unwrap();
    assert!(!c.can_drive(1, 0.2));
    assert!(c.snapshot(0.2)["1"]["proposal"].is_null());
    c.apply(1,"autonomy/status",br#"{"requested_level":"teleop","effective_level":"teleop","revision":0,"safety":"clear","run_id":"new"}"#,0.2).unwrap();
    assert!(c.can_drive(1, 0.3));
}
#[test]
fn reordered_releases_are_independent_but_clear_fences_old_presses() {
    let mut order = InputOrder::default();
    assert!(order.accept("w", 1));
    assert!(order.accept("d", 2));
    assert!(order.accept("d", 4));
    assert!(order.accept("w", 3));
    order.clear(10);
    assert!(!order.accept("w", 9));
    assert!(order.accept("w", 11));
}
#[test]
fn authority_heartbeats_do_not_extend_proposal_expiry() {
    let mut c = OperatorCache::default();
    c.apply(
        1,
        "experiment/status",
        br#"{"run_id":"run","run_elapsed":0}"#,
        0.,
    )
    .unwrap();
    c.apply(
        1,
        "goal/proposal",
        br#"{"run_id":"run","proposal_id":1,"x":1,"y":1,"expires_at":30}"#,
        0.,
    )
    .unwrap();
    c.apply(
        1,
        "autonomy/status",
        br#"{"run_id":"run","revision":1,"requested_level":"supervised"}"#,
        32.,
    )
    .unwrap();
    assert!(c.snapshot(32.)["1"]["proposal_remaining"].as_f64().unwrap() <= 0.);
}

#[test]
fn direct_waypoint_authority_is_retained_as_a_distinct_supported_level() {
    let mut cache = OperatorCache::default();
    cache.apply(8, "autonomy/status", br#"{"requested_level":"waypoint_direct","effective_level":"waypoint_direct","active_source":"waypoint","safety":"clear","reason":"active","revision":1,"supported_levels":["teleop","waypoint_direct","waypoint"]}"#, 0.).unwrap();
    assert_eq!(cache.snapshot(0.1)["8"]["status"]["requested_level"], "waypoint_direct");
}
