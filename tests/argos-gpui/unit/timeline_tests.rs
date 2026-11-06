use super::*;

fn seen(help: Option<&str>, attended: bool) -> Observation {
    Observation {
        online: true,
        help: help.map(str::to_string),
        attended,
        ..Observation::default()
    }
}

#[test]
fn neglect_wait_interaction_neglect() {
    let mut timeline = Timeline::default();
    let mut t = 0.0;
    while t < 10.0 {
        timeline.observe(t, 1, &seen(None, false));
        t += 0.1;
    }
    while t < 15.0 {
        timeline.observe(t, 1, &seen(Some("h-1-1"), false));
        t += 0.1;
    }
    // Selected once: interaction for the rest of the episode, even after deselect.
    timeline.observe(t, 1, &seen(Some("h-1-1"), true));
    t += 0.1;
    while t < 20.0 {
        timeline.observe(t, 1, &seen(Some("h-1-1"), false));
        t += 0.1;
    }
    while t < 22.0 {
        timeline.observe(t, 1, &seen(None, false));
        t += 0.1;
    }
    let rows = timeline.rows(t - 0.1, |_| Vec::new());
    let phases: Vec<Phase> = rows[0].spans.iter().map(|s| s.phase).collect();
    assert_eq!(
        phases,
        [
            Phase::Neglect,
            Phase::Waiting,
            Phase::Interaction,
            Phase::Neglect
        ]
    );
    assert!((rows[0].spans[1].start - 10.0).abs() < 0.11);
    // A new episode starts waiting again.
    timeline.observe(t, 1, &seen(Some("h-1-2"), false));
    assert_eq!(
        timeline.rows(t, |_| Vec::new())[0].phase,
        Some(Phase::Waiting)
    );
}

#[test]
fn manual_rover_waits_until_driven() {
    let mut timeline = Timeline::default();
    let manual = |attended| Observation {
        online: true,
        manual: true,
        attended,
        ..Observation::default()
    };
    timeline.observe(0.0, 3, &manual(false));
    assert_eq!(
        timeline.rows(0.0, |_| Vec::new())[0].phase,
        Some(Phase::Waiting)
    );
    timeline.observe(0.1, 3, &manual(true));
    assert_eq!(
        timeline.rows(0.1, |_| Vec::new())[0].phase,
        Some(Phase::Interaction)
    );
}

#[test]
fn queue_order_and_history() {
    let mut timeline = Timeline::default();
    for t in [0.0, 0.5, 1.0, 1.5, 2.0] {
        timeline.observe(t, 4, &seen(Some("h-4-1"), false));
        if t >= 1.0 {
            timeline.observe(t, 2, &seen(Some("h-2-1"), false));
        }
        timeline.observe(t, 5, &seen(None, false));
        timeline.observe(t, 1, &seen(None, false));
        timeline.observe(
            t,
            3,
            &Observation {
                online: true,
                manual: true,
                attended: true,
                ..Observation::default()
            },
        );
    }
    let rows = timeline.rows(2.0, |id| match id {
        5 => vec![Forecast {
            in_s: 100.,
            label: "arrives".into(),
        }],
        1 => vec![Forecast {
            in_s: 200.,
            label: "budget".into(),
        }],
        _ => Vec::new(),
    });
    let order: Vec<u64> = rows.iter().map(|r| r.id).collect();
    assert_eq!(order, [4, 2, 5, 1, 3]);
    // Old spans fall off after the kept history.
    timeline.observe(2.0 + TIMELINE_PAST_S + 10., 4, &seen(None, false));
    let row = timeline
        .rows(2.0 + TIMELINE_PAST_S + 10., |_| Vec::new())
        .into_iter()
        .find(|r| r.id == 4)
        .unwrap();
    assert_eq!(row.spans.len(), 1);
}

#[test]
fn speed_from_recent_poses() {
    let mut timeline = Timeline::default();
    for step in 0..=30 {
        let t = step as f64 * 0.1;
        timeline.observe(
            t,
            1,
            &Observation {
                online: true,
                x: t * 0.5,
                ..Observation::default()
            },
        );
    }
    assert!((timeline.speed(1).unwrap() - 0.5).abs() < 1e-9);
    assert_eq!(clock(125.4), "2:05");
}
