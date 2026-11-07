use super::*;
use serde_json::json;

fn status(level: &str, reason: &str, source: &str) -> Value {
    json!({"requested_level": level, "reason": reason, "active_source": source, "safety": "clear"})
}

#[test]
fn classifies_status_for_the_operator() {
    assert_eq!(
        classify(&status("waypoint", "active", "waypoint")).0,
        Phase::Working
    );
    assert_eq!(
        classify(&status("waypoint", "idle", "none")),
        (Phase::Help, Some("done"))
    );
    assert_eq!(
        classify(&status("waypoint", "route_no_path", "none")),
        (Phase::Help, Some("blocked"))
    );
    assert_eq!(classify(&status("teleop", "idle", "none")).0, Phase::Manual);
    let mut stop = status("teleop", "operator_stop", "none");
    stop["safety"] = "emergency_stop".into();
    assert_eq!(classify(&stop), (Phase::Help, Some("stopped")));
}

#[test]
fn raises_and_clears_after_debounce() {
    let mut help = HelpTracker::default();
    assert!(
        help.observe(2, &status("waypoint", "active", "waypoint"), 0.0)
            .is_empty()
    );
    // A short hold does not count.
    assert!(
        help.observe(2, &status("waypoint", "route_no_path", "none"), 1.0)
            .is_empty()
    );
    assert!(
        help.observe(2, &status("waypoint", "active", "waypoint"), 1.5)
            .is_empty()
    );
    assert_eq!(help.open(2), None);
    // A long one does, stamped from when it began.
    assert!(
        help.observe(2, &status("waypoint", "route_no_path", "none"), 3.0)
            .is_empty()
    );
    let raised = help.observe(2, &status("waypoint", "route_no_path", "none"), 4.6);
    assert_eq!(raised.len(), 1);
    assert_eq!(raised[0]["event"], "raised");
    assert_eq!(raised[0]["class"], "blocked");
    assert_eq!(raised[0]["at"], 3.0);
    assert_eq!(help.open(2).as_deref(), Some("h-2-1"));
    // Flicker inside the episode keeps it open.
    assert!(
        help.observe(2, &status("waypoint", "idle", "none"), 5.0)
            .is_empty()
    );
    help.observe(2, &status("waypoint", "active", "waypoint"), 6.0);
    let cleared = help.observe(2, &status("waypoint", "active", "waypoint"), 7.6);
    assert_eq!(cleared[0]["event"], "cleared");
    assert_eq!(cleared[0]["outcome"], "resumed");
    assert_eq!(cleared[0]["duration_s"], 3.0);
    assert_eq!(help.open(2), None);
}

#[test]
fn takeover_clears_as_manual() {
    let mut help = HelpTracker::default();
    help.observe(1, &status("waypoint", "idle", "none"), 0.0);
    assert_eq!(help.open(1).as_deref(), Some("h-1-1"));
    help.observe(1, &status("teleop", "idle", "none"), 1.0);
    let cleared = help.observe(1, &status("teleop", "idle", "none"), 2.6);
    assert_eq!(cleared[0]["outcome"], "to_manual");
}

#[test]
fn open_episode_carries_class_and_level() {
    let mut help = HelpTracker::default();
    help.observe(4, &status("waypoint", "active", "waypoint"), 0.0);
    help.observe(4, &status("waypoint", "idle", "none"), 1.0);
    help.observe(4, &status("waypoint", "idle", "none"), 3.0);
    let open = help.episode(4).unwrap();
    assert_eq!(open.class, "done");
    assert_eq!(open.level.as_deref(), Some("waypoint"));
    assert_eq!(open.raised, 1.0);
    assert_eq!(help.open(4), Some(open.help_id));
}
