use argos_core::{
    DockReport, MissionCommand, MissionPlan, MissionRun, MissionStep, MissionView, Phase,
    SCORE_STANDOFF_M, StepKind, TAKE_TIMEOUT_S, Wait,
};

fn view<'a>(
    level: &'a str,
    goal: Option<(f64, f64, &'a str)>,
    dock: Option<&'a DockReport>,
) -> MissionView<'a> {
    MissionView {
        x: 0.,
        y: 0.,
        level: Some(level),
        goal,
        dock,
        buckets: &[],
        halted: None,
    }
}

fn dock(dock_id: u64, phase: &str, bucket: u32) -> DockReport {
    DockReport {
        dock_id,
        phase: phase.into(),
        reason: "retries_exhausted".into(),
        object_id: Some(bucket),
        color: Some("orange".into()),
        lateral_m: 0.,
        range_m: 0.,
        heading_deg: 0.,
        in_tolerance: true,
        fix_age_s: None,
        retries: 0,
        verified: true,
        approval: "operator".into(),
        awaiting_approval: false,
        gate_open: false,
    }
}

fn plan(steps: Vec<MissionStep>, repeat: bool) -> MissionPlan {
    MissionPlan { steps, repeat }
}

fn goal(tick: &argos_core::MissionTick) -> (f64, f64) {
    match tick.commands.as_slice() {
        [MissionCommand::Goal { x, y, .. }] => (*x, *y),
        other => panic!("expected one goal, got {other:?}"),
    }
}

#[test]
fn start_refuses_levels_without_waypoint_goals() {
    let p = plan(vec![MissionStep::at(StepKind::Waypoint, 1., 2.)], false);
    let error = MissionRun::start(p.clone(), &view("explore", None, None), 0.).unwrap_err();
    assert!(error.contains("L3 Planned"), "{error}");
    assert!(MissionRun::start(plan(vec![], false), &view("waypoint", None, None), 0.).is_err());
    assert!(MissionRun::start(p, &view("waypoint_direct", None, None), 0.).is_ok());
}

#[test]
fn waypoints_advance_on_arrival_of_their_own_goal() {
    let p = plan(
        vec![
            MissionStep::at(StepKind::Waypoint, 1., 2.),
            MissionStep::at(StepKind::Collect, 5., 0.),
        ],
        false,
    );
    let (mut run, tick) = MissionRun::start(p, &view("waypoint", None, None), 0.).unwrap();
    assert_eq!(goal(&tick), (1., 2.));
    // An old goal elsewhere that reads arrived does not count.
    assert!(
        run.tick(&view("waypoint", Some((9., 9., "arrived")), None), 0.5)
            .commands
            .is_empty()
    );
    run.tick(&view("waypoint", Some((1., 2., "active")), None), 1.);
    let tick = run.tick(&view("waypoint", Some((1., 2., "arrived")), None), 2.);
    assert_eq!(goal(&tick), (5., 0.));
    assert_eq!(run.index, 1);
    run.tick(&view("waypoint", Some((5., 0., "active")), None), 3.);
    run.tick(&view("waypoint", Some((5., 0., "arrived")), None), 4.);
    assert_eq!(run.phase, Phase::Done);
}

#[test]
fn an_untaken_goal_is_resent_then_pauses() {
    let p = plan(vec![MissionStep::at(StepKind::Waypoint, 1., 2.)], false);
    let (mut run, _) = MissionRun::start(p, &view("waypoint", None, None), 0.).unwrap();
    let mut now = 0.;
    for _ in 0..2 {
        now += TAKE_TIMEOUT_S;
        assert_eq!(
            goal(&run.tick(&view("waypoint", None, None), now)),
            (1., 2.)
        );
    }
    now += TAKE_TIMEOUT_S;
    run.tick(&view("waypoint", None, None), now);
    assert!(matches!(&run.phase, Phase::Paused(reason) if reason.contains("did not take")));
}

#[test]
fn a_dropped_goal_or_level_change_pauses_and_resume_resends() {
    let p = plan(vec![MissionStep::at(StepKind::Waypoint, 1., 2.)], false);
    let (mut run, _) = MissionRun::start(p, &view("waypoint", None, None), 0.).unwrap();
    run.tick(&view("waypoint", Some((1., 2., "active")), None), 1.);
    run.tick(&view("waypoint", None, None), 2.);
    assert!(matches!(run.phase, Phase::Paused(_)));
    assert!(run.resume(&view("teleop", None, None), 3.).is_err());
    let tick = run.resume(&view("waypoint", None, None), 3.).unwrap();
    assert_eq!(goal(&tick), (1., 2.));

    run.tick(&view("teleop", Some((1., 2., "active")), None), 4.);
    assert!(matches!(&run.phase, Phase::Paused(reason) if reason.contains("L2 Direct")));
}

#[test]
fn home_waits_for_refill_and_out_of_balls_returns_to_collect() {
    let p = plan(
        vec![
            MissionStep::at(StepKind::Collect, 3., 3.),
            MissionStep::at(StepKind::Home, 0., -1.),
            MissionStep::at(StepKind::Waypoint, 8., 0.),
        ],
        false,
    );
    let (mut run, _) = MissionRun::start(p, &view("waypoint", None, None), 0.).unwrap();
    run.tick(&view("waypoint", Some((3., 3., "arrived")), None), 1.);
    run.tick(&view("waypoint", Some((0., -1., "arrived")), None), 2.);
    assert_eq!(run.phase, Phase::Waiting(Wait::Refill));
    // Waiting holds until the operator answers.
    assert!(
        run.tick(&view("waypoint", None, None), 60.)
            .commands
            .is_empty()
    );
    let tick = run.out_of_balls(&view("waypoint", None, None), 61.);
    assert_eq!(goal(&tick), (3., 3.));
    assert_eq!(run.index, 0);
    run.tick(&view("waypoint", Some((3., 3., "arrived")), None), 62.);
    run.tick(&view("waypoint", Some((0., -1., "arrived")), None), 63.);
    let tick = run.loaded(&view("waypoint", None, None), 64.);
    assert_eq!(goal(&tick), (8., 0.));
}

#[test]
fn score_stops_short_facing_the_bucket_then_docks_until_done() {
    let p = plan(
        vec![
            MissionStep::score(4, "Orange bucket 4".into(), 10., 0.),
            MissionStep::at(StepKind::Home, 0., 0.),
        ],
        true,
    );
    let buckets = [(4, 6., 0.)];
    let mut v = view("waypoint", None, None);
    v.buckets = &buckets;
    let (mut run, tick) = MissionRun::start(p, &v, 0.).unwrap();
    // The live bucket position wins over the planned one.
    match tick.commands.as_slice() {
        [MissionCommand::Goal { x, y, yaw }] => {
            assert!((x - (6. - SCORE_STANDOFF_M)).abs() < 1e-9 && y.abs() < 1e-9);
            assert!(yaw.unwrap().abs() < 1e-9);
        }
        other => panic!("{other:?}"),
    }
    v.goal = Some((4.5, 0., "arrived"));
    let tick = run.tick(&v, 1.);
    assert_eq!(tick.commands, vec![MissionCommand::Dock { bucket: 4 }]);
    let report = dock(7, "aligned", 4);
    v.goal = None;
    v.dock = Some(&report);
    run.tick(&v, 2.);
    assert!(matches!(
        run.phase,
        Phase::Docking {
            dock_id: Some(7),
            ..
        }
    ));
    let report = dock(7, "done", 4);
    v.dock = Some(&report);
    let tick = run.tick(&v, 3.);
    assert_eq!(goal(&tick), (0., 0.));
    assert_eq!(run.index, 1);
    // Repeat: after home, back to step 1.
    v.dock = None;
    v.goal = Some((0., 0., "arrived"));
    run.tick(&v, 4.);
    run.loaded(&v, 5.);
    assert_eq!((run.index, run.laps), (0, 1));
}

#[test]
fn a_failed_dock_pauses_and_resume_docks_again() {
    let p = plan(
        vec![MissionStep::score(4, "Orange bucket 4".into(), 2., 0.)],
        false,
    );
    let mut v = view("waypoint", None, None);
    let (mut run, _) = MissionRun::start(p, &v, 0.).unwrap();
    v.goal = Some((0.5, 0., "arrived"));
    run.tick(&v, 1.);
    let report = dock(3, "creep", 4);
    v.dock = Some(&report);
    run.tick(&v, 2.);
    let report = dock(3, "failed", 4);
    v.dock = Some(&report);
    run.tick(&v, 3.);
    assert!(matches!(&run.phase, Phase::Paused(reason) if reason.contains("failed")));
    let tick = run.resume(&v, 4.).unwrap();
    assert_eq!(tick.commands, vec![MissionCommand::Dock { bucket: 4 }]);
}

#[test]
fn stop_and_pause_hold_the_rover_instead_of_cancelling_its_goal() {
    // Terra drops L2/L3 to L0 on a goal cancel, so the mission holds instead.
    let p = plan(vec![MissionStep::at(StepKind::Waypoint, 1., 2.)], false);
    let v = view("waypoint", Some((1., 2., "active")), None);
    let (mut run, _) = MissionRun::start(p.clone(), &v, 0.).unwrap();
    assert_eq!(
        run.pause_now("operator", Some(&v)).commands,
        vec![MissionCommand::Hold { x: 0., y: 0. }]
    );
    // Still at L3, so Resume sends the step again.
    assert_eq!(goal(&run.resume(&v, 1.).unwrap()), (1., 2.));
    let (mut run, _) = MissionRun::start(p.clone(), &v, 0.).unwrap();
    assert_eq!(
        run.stop(Some(&v)).commands,
        vec![MissionCommand::Hold { x: 0., y: 0. }]
    );
    assert!(!run.active());
    // Rover gone: nothing to send.
    let (mut run, _) = MissionRun::start(p, &v, 0.).unwrap();
    assert!(
        run.pause_now("Rover left the fleet", None)
            .commands
            .is_empty()
    );
}

#[test]
fn skip_sends_the_next_goal_without_cancelling() {
    let p = plan(
        vec![
            MissionStep::at(StepKind::Waypoint, 1., 2.),
            MissionStep::at(StepKind::Collect, 5., 0.),
        ],
        false,
    );
    let v = view("waypoint", Some((1., 2., "active")), None);
    let (mut run, _) = MissionRun::start(p, &v, 0.).unwrap();
    run.tick(&v, 1.);
    let tick = run.skip(&v, 2.);
    assert_eq!(
        tick.commands,
        vec![MissionCommand::Goal {
            x: 5.,
            y: 0.,
            yaw: None
        }]
    );
    assert!(matches!(run.phase, Phase::Driving { .. }));
    assert_eq!(run.index, 1);
    // Skipping the last step holds the rover and ends the mission.
    let tick = run.skip(&view("waypoint", Some((5., 0., "active")), None), 3.);
    assert_eq!(tick.commands, vec![MissionCommand::Hold { x: 0., y: 0. }]);
    assert_eq!(run.phase, Phase::Done);
}

#[test]
fn skip_during_a_dock_cancels_the_dock_then_drives_on() {
    let p = plan(
        vec![
            MissionStep::score(4, "Orange bucket 4".into(), 2., 0.),
            MissionStep::at(StepKind::Home, 0., -1.),
        ],
        false,
    );
    let mut v = view("waypoint", None, None);
    let (mut run, _) = MissionRun::start(p, &v, 0.).unwrap();
    v.goal = Some((0.5, 0., "arrived"));
    run.tick(&v, 1.);
    let tick = run.skip(&v, 2.);
    assert_eq!(
        tick.commands,
        vec![
            MissionCommand::CancelDock,
            MissionCommand::Goal {
                x: 0.,
                y: -1.,
                yaw: None
            }
        ]
    );
}

#[test]
fn plan_edits_keep_order_and_limits() {
    let mut p = MissionPlan::default();
    for i in 0..3 {
        p.push(MissionStep::at(StepKind::Waypoint, f64::from(i), 0.))
            .unwrap();
    }
    p.move_up(2);
    assert_eq!(p.steps[1].x, 2.);
    p.move_down(0);
    assert_eq!(p.steps[0].x, 2.);
    p.remove(0);
    assert_eq!(p.steps.len(), 2);
    assert!(
        p.push(MissionStep::at(StepKind::Waypoint, f64::NAN, 0.))
            .is_err()
    );
    let json = serde_json::to_string(&p).unwrap();
    assert_eq!(serde_json::from_str::<MissionPlan>(&json).unwrap(), p);
}
