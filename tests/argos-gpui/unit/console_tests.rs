use super::*;
use crate::GoalMark;
use crate::fleet_presence;
use argos_core::{
    CommandOrigin, HardwareObservation, OutdoorReport, RouteSample, RoverSnapshot, Snapshot,
};

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
        exploration: None,
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

const SPACE: CardSpace = CardSpace {
    card_width: 240.,
    card_height: 56.,
    field: FieldRect {
        x: 0.,
        y: 0.,
        width: 1000.,
        height: 700.,
    },
    avoid: None,
};

fn side(group: &CardGroup) -> (bool, bool) {
    (group.leading, group.below)
}

#[test]
fn rovers_on_the_same_pixel_share_one_stacked_card_group() {
    let groups = place_cards(&[(1, 400., 300.), (0, 401., 300.)], SPACE);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].ids, vec![0, 1]);
    assert_eq!((groups[0].x, groups[0].y), (401., 300.));
}

#[test]
fn rovers_whose_cards_do_not_meet_keep_their_own_cards() {
    let groups = place_cards(&[(0, 100., 300.), (1, 400., 300.), (2, 100., 500.)], SPACE);
    assert_eq!(groups.len(), 3);
    assert!(groups.iter().all(|group| group.ids.len() == 1));
}

#[test]
fn cards_stand_above_and_after_the_rover_in_open_field() {
    assert_eq!(
        side(&place_cards(&[(0, 400., 300.)], SPACE)[0]),
        (false, false)
    );
}

#[test]
fn cards_move_away_from_field_edges() {
    assert_eq!(
        side(&place_cards(&[(0, 900., 300.)], SPACE)[0]),
        (true, false)
    );
    assert_eq!(
        side(&place_cards(&[(0, 400., 20.)], SPACE)[0]),
        (false, true)
    );
    assert_eq!(
        side(&place_cards(&[(0, 900., 20.)], SPACE)[0]),
        (true, true)
    );
}

#[test]
fn cards_clear_the_inspector_or_go_below_it() {
    // An inspector on the trailing side, covering the top of the field.
    let inspector = CardSpace {
        avoid: Some(FieldRect {
            x: 560.,
            y: 12.,
            width: 320.,
            height: 340.,
        }),
        ..SPACE
    };
    assert_eq!(
        side(&place_cards(&[(0, 500., 300.)], inspector)[0]),
        (true, false)
    );
    // Too close on both sides, as in a narrow field: hang below the anchor.
    let narrow = CardSpace {
        field: FieldRect {
            width: 580.,
            ..SPACE.field
        },
        avoid: Some(FieldRect {
            x: 248.,
            y: 12.,
            width: 320.,
            height: 340.,
        }),
        ..SPACE
    };
    assert_eq!(
        side(&place_cards(&[(0, 290., 380.)], narrow)[0]),
        (false, true)
    );
}

#[test]
fn a_stack_too_tall_for_any_side_keeps_the_default() {
    let tiny = CardSpace {
        field: FieldRect {
            x: 0.,
            y: 0.,
            width: 200.,
            height: 50.,
        },
        ..SPACE
    };
    assert_eq!(
        side(&place_cards(&[(0, 100., 25.)], tiny)[0]),
        (false, false)
    );
}

#[test]
fn status_prefers_this_windows_note_over_the_feed_command() {
    assert_eq!(
        status_line("Stop sent to rover 1", "goal accepted"),
        "Stop sent to rover 1"
    );
    assert_eq!(status_line("", "goal accepted"), "goal accepted");
}

#[test]
fn level_group_selects_the_requested_level_not_the_effective_one() {
    let mut rover = mark(1);
    rover.requested_level = Some("teleop".into());
    rover.effective_level = Some("waypoint".into());
    assert_eq!(level_index(&rover), Some(0));
    assert_eq!(OPERATOR_LEVELS[level_index(&rover).unwrap()].1, "L0 Manual");
    rover.requested_level = Some("explore".into());
    assert_eq!(
        OPERATOR_LEVELS[level_index(&rover).unwrap()].1,
        "L4 Explore"
    );
    rover.requested_level = Some("not_a_level".into());
    assert_eq!(level_index(&rover), None);
    rover.requested_level = None;
    assert_eq!(level_index(&rover), None);
    rover.recommended_level = Some("supervised".into());
    assert_eq!(level_line(&rover), "L3 Waypoint · recommended Supervised");
}

#[test]
fn navigation_source_follows_the_vehicle_report() {
    let mut rover = mark(1);
    assert_eq!(source_index(&rover), 0);
    assert!(navigation_note(&rover).starts_with("Local navigation. Outdoor GPS stays off"));
    rover.outdoor = Some(outdoor("driving", None, None));
    rover.outdoor_gps = true;
    assert_eq!(NAVIGATION_SOURCES[source_index(&rover)].0, "outdoor_gps");
    assert!(navigation_note(&rover).starts_with("Outdoor GPS acknowledged"));
    rover.app_update_required = true;
    assert!(navigation_note(&rover).contains("unavailable in this runtime"));
}

#[test]
fn speed_chips_match_the_active_cap_only() {
    assert_eq!(speed_index(0.3), Some(2));
    assert_eq!(speed_index(0.3000004), Some(2));
    assert_eq!(speed_index(0.25), None);
}

#[test]
fn hardware_lines_put_the_stop_ahead_of_the_relay() {
    let rover = mark(1);
    assert_eq!(
        hardware_lines(&rover, false),
        (
            "Hardware status unavailable",
            Some("Hardware status unavailable · connect the updated Terra app")
        )
    );
    assert_eq!(
        hardware_lines(&rover, true),
        ("Stop active", Some("Use Reset stop before driving"))
    );
}

#[test]
fn a_path_is_provisional_only_with_an_unspecified_segment() {
    let mut rover = mark(1);
    let sample = |travel| RouteSample {
        x: 0.,
        y: 0.,
        travel,
    };
    rover.route = vec![sample(Travel::Unspecified)];
    assert!(!path_provisional(&rover));
    rover.route = vec![sample(Travel::Forward), sample(Travel::Forward)];
    assert!(!path_provisional(&rover));
    rover.route.push(sample(Travel::Unspecified));
    assert!(path_provisional(&rover));
}

#[test]
fn unlocated_lists_only_fleet_members_without_a_mark() {
    let member = |id, pose_age| RoverSnapshot {
        id,
        membership: "online".into(),
        pose: None,
        pose_age,
        goal: None,
        goal_age: None,
        goal_received_at: None,
        command_phase: "none".into(),
        command_token: None,
    };
    let snapshot = Snapshot {
        link: "healthy".into(),
        fleet_age: None,
        rovers: vec![member(4, Some(2.)), member(1, None), member(2, None)],
        notice: None,
    };
    let marks = [mark(2)];
    let presence = fleet_presence(&snapshot, &marks);
    let rows = unlocated(&marks, &presence);
    assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), [1, 4]);
    assert_eq!(rows[0].label, "Rover 1 · online");
    assert_eq!(
        rows[0].detail,
        "Position unavailable · Waiting for telemetry"
    );
    assert_eq!(
        rows[1].detail,
        "Position unavailable · Last observation 2.0 s ago"
    );
}

#[test]
fn inspector_tabs_round_trip_through_the_tab_bar_index() {
    for tab in InspectorTab::ALL {
        assert_eq!(InspectorTab::from_index(tab.index()), tab);
    }
    assert_eq!(InspectorTab::from_index(9), InspectorTab::Overview);
}

#[test]
fn field_notices_show_the_link_reason_only_while_unhealthy() {
    assert_eq!(
        field_notices(
            "password",
            "Set ARGOS_ZENOH_PASSWORD in .env",
            Some("Set GOOGLE_MAPS_API_KEY in .env")
        ),
        [
            "Link password · Set ARGOS_ZENOH_PASSWORD in .env",
            "Set GOOGLE_MAPS_API_KEY in .env"
        ]
    );
    assert!(field_notices("healthy", "2 rovers", None).is_empty());
    assert!(field_notices("connecting", "", None).is_empty());
}

#[test]
fn callout_names_the_exploration_state_instead_of_idle() {
    let mut rover = mark(1);
    rover.requested_level = Some("explore".into());
    rover.effective_level = Some("explore".into());
    let report =
        |phase: &str, end: Option<&str>, reason: Option<&str>| argos_core::ExplorationReport {
            phase: phase.into(),
            elapsed_s: 60.,
            remaining_s: 120.,
            budget_s: 180.,
            target: Some((1., 2.)),
            coverage_m2: 10.,
            explored_ratio: 0.2,
            end_reason: end.map(Into::into),
            frontiers_attempted: 1,
            blacklisted: 0,
            reason: reason.map(Into::into),
        };
    rover.exploration = Some(report("running", None, Some("frontier_retry")));
    let running = callout(&rover, false, false);
    assert_eq!(running.tag, Some(CalloutTag::Goal));
    assert_eq!(
        running.state,
        "Stuck · retrying another frontier · 2 min left"
    );
    rover.exploration = Some(report("ended", Some("no_frontiers_left"), None));
    let ended = callout(&rover, false, false);
    assert_eq!(ended.tag, None);
    assert_eq!(ended.state, "Done · no frontiers left");
}
