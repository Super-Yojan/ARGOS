use super::*;
use crate::GoalMark;
use argos_core::{CommandOrigin, HardwareObservation, OutdoorReport};

fn mark(id: u64) -> RoverMark {
    RoverMark {
        id,
        latitude: 38.83,
        longitude: -77.31,
        map_frame: None,
        heading: 0.,
        local_x: 0.,
        local_y: 0.,
        yaw: 0.,
        sequence: 1,
        status: "online".into(),
        occupancy: String::new(),
        grid: None,
        requested_level: Some("waypoint".into()),
        effective_level: Some("waypoint".into()),
        recommended_level: None,
        route: Vec::new(),
        goal: None,
        degraded: false,
        degrade_reason: String::new(),
        quality_reported: false,
        reversing: false,
        hold: None,
        command_origin: CommandOrigin::Unknown,
        arrival_radius: 10.,
        outdoor_gps: false,
        hardware: HardwareObservation::default(),
        authority_revision: 0,
        safety: "clear".into(),
        run_id: "run-1".into(),
        navigation_revision: 0,
        outdoor: None,
        raw_gps: None,
        pose_available: true,
        gps_display_freshness_seconds: 5.,
        gps_source_fresh: true,
        app_update_required: false,
        mission_active: false,
        goal_active: false,
    }
}

fn outdoor(phase: &str, hold: Option<&str>, remaining: Option<f64>) -> OutdoorReport {
    OutdoorReport {
        schema_version: 1,
        supported: true,
        acknowledged: true,
        source: "outdoor_gps".into(),
        run_id: "run-1".into(),
        navigation_revision: 1,
        ready: hold.is_none(),
        hold_reason: hold.map(Into::into),
        gps_age_s: None,
        gps_accuracy_m: None,
        heading_age_s: None,
        heading_accuracy_deg: None,
        arrival_radius_m: 5.,
        destination_latitude: None,
        destination_longitude: None,
        remaining_m: remaining,
        phase: phase.into(),
        gps_latitude: None,
        gps_longitude: None,
        token: None,
        result: None,
        age_s: None,
    }
}

#[test]
fn an_idle_rover_names_its_level_and_carries_no_tag() {
    let callout = callout(&mark(1), false, false);
    assert_eq!(callout.tag, None);
    assert_eq!(callout.state, "L3 Waypoint · idle");
    assert_eq!(callout.attention, None);
}

#[test]
fn a_stop_outranks_driving_and_attention_but_trace_keeps_the_reason() {
    let mut rover = mark(2);
    rover.status = "offline".into();
    let stopped = callout(&rover, true, true);
    assert_eq!(stopped.tag, Some(CalloutTag::Stopped));
    assert_eq!(stopped.attention, None);
    let rows = trace(&rover, true);
    assert!(rows.contains(&("Hardware", "Stop active".into())));
    assert!(rows.contains(&("Attention", "Offline".into())));
    assert_eq!(callout(&rover, false, true).tag, Some(CalloutTag::Driving));
    let offline = callout(&rover, false, false);
    assert_eq!(offline.tag, Some(CalloutTag::Attention));
    assert_eq!(offline.attention.as_deref(), Some("Offline"));
}

#[test]
fn a_stale_pose_needs_a_person_except_under_manual_control() {
    let mut rover = mark(3);
    rover.status = "pose stale".into();
    assert_eq!(callout(&rover, false, false).state, "Pose stale");
    rover.requested_level = Some("teleop".into());
    rover.effective_level = Some("teleop".into());
    assert_eq!(callout(&rover, false, false).tag, None);
}

#[test]
fn outdoor_holds_and_missions_read_as_the_vehicle_reports_them() {
    let mut rover = mark(4);
    rover.outdoor = Some(outdoor("holding", Some("gps_stale"), Some(42.4)));
    let held = callout(&rover, false, false);
    assert_eq!(held.tag, Some(CalloutTag::Attention));
    assert_eq!(held.state, "GPS stale");
    rover.outdoor = Some(outdoor("active", None, Some(42.4)));
    rover.goal = Some(GoalMark {
        x: 0.,
        y: 0.,
        latitude: None,
        longitude: None,
        state: "active".into(),
    });
    let going = callout(&rover, false, false);
    assert_eq!(going.tag, Some(CalloutTag::Goal));
    assert_eq!(going.state, "Phase active (provisional) · 42 m left");
    assert!(trace(&rover, false).contains(&("Goal", "active · 42 m left".into())));
}

#[test]
fn trace_separates_requested_from_effective_autonomy() {
    let mut rover = mark(5);
    rover.requested_level = Some("teleop".into());
    rover.effective_level = None;
    assert!(
        trace(&rover, false).contains(&("Autonomy", "requested L0 Manual · not confirmed".into()))
    );
    rover.effective_level = Some("waypoint".into());
    assert!(trace(&rover, false).contains(&(
        "Autonomy",
        "requested L0 Manual · effective L3 Waypoint".into()
    )));
}

#[test]
fn heading_hold_and_motor_fault_are_attention() {
    let mut rover = mark(6);
    rover.hold = Some(HEADING_HOLD.into());
    assert_eq!(callout(&rover, false, false).state, HEADING_HOLD_LABEL);
    rover.hold = None;
    rover.hardware.fault = Some("Left motor over current".into());
    assert_eq!(
        callout(&rover, false, false).attention.as_deref(),
        Some("Left motor over current")
    );
}

#[test]
fn fleet_ids_cover_located_and_unlocated_rovers_once() {
    assert_eq!(fleet_ids(&[mark(1), mark(3)], &[]), BTreeSet::from([1, 3]));
}
