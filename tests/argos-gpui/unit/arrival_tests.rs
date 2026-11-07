use super::*;

fn done<'a>(help: &'a str, level: &'a str, selected: bool) -> Seen<'a> {
    Seen {
        help_id: Some(help),
        done: true,
        level: Some(level),
        selected,
    }
}

fn working(selected: bool) -> Seen<'static> {
    Seen {
        selected,
        ..Seen::default()
    }
}

fn events(update: &Update) -> Vec<&str> {
    update
        .records
        .iter()
        .map(|record| record["event"].as_str().unwrap())
        .collect()
}

#[test]
fn arrival_chimes_flashes_and_queues_until_seen_then_closes() {
    let mut alerts = ArrivalAlerts::new(Cue::On);
    assert!(alerts.observe(0.0, 1, &working(false)).records.is_empty());
    let shown = alerts.observe(10.0, 1, &done("h-1-1", "waypoint", false));
    assert_eq!(events(&shown), ["shown"]);
    assert!(shown.chime);
    assert_eq!(shown.records[0]["help_id"], "h-1-1");
    assert_eq!(shown.records[0]["cues"], json!(["sound", "flash", "queue"]));
    let alert = alerts.alert(1).unwrap();
    assert!(alert.unseen());
    assert!(alert.flash_on(10.0));
    assert!(!alert.flash_on(10.6));
    // Nothing new while it waits.
    assert_eq!(
        alerts.observe(12.0, 1, &done("h-1-1", "waypoint", false)),
        Update::default()
    );
    let seen = alerts.observe(14.0, 1, &done("h-1-1", "waypoint", true));
    assert_eq!(events(&seen), ["seen"]);
    assert_eq!(seen.records[0]["via"], "select");
    assert_eq!(seen.records[0]["after_s"], 4.0);
    assert!(!alerts.alert(1).unwrap().flash_on(14.0));
    // Still queued until the rover has work again.
    assert_eq!(alerts.queue().len(), 1);
    let closed = alerts.observe(20.0, 1, &working(true));
    assert_eq!(events(&closed), ["closed"]);
    assert_eq!(closed.records[0]["open_s"], 10.0);
    assert_eq!(closed.records[0]["seen_after_s"], 4.0);
    assert!(alerts.queue().is_empty());
}

#[test]
fn unseen_arrival_gets_one_reminder() {
    let mut alerts = ArrivalAlerts::new(Cue::On);
    alerts.observe(0.0, 2, &done("h-2-1", "waypoint_direct", false));
    let reminded = alerts.observe(REMIND_AFTER_S, 2, &done("h-2-1", "waypoint_direct", false));
    assert_eq!(events(&reminded), ["reminded"]);
    assert!(reminded.chime);
    let later = alerts.observe(
        REMIND_AFTER_S * 3.,
        2,
        &done("h-2-1", "waypoint_direct", false),
    );
    assert_eq!(later, Update::default());
}

#[test]
fn queue_pick_is_logged_as_queue() {
    let mut alerts = ArrivalAlerts::new(Cue::On);
    alerts.observe(0.0, 3, &done("h-3-1", "waypoint", false));
    alerts.picked_from_queue(3);
    let seen = alerts.observe(2.0, 3, &done("h-3-1", "waypoint", true));
    assert_eq!(seen.records[0]["via"], "queue");
}

#[test]
fn only_done_episodes_at_goal_levels_alert() {
    let mut alerts = ArrivalAlerts::new(Cue::On);
    let blocked = Seen {
        help_id: Some("h-1-1"),
        done: false,
        level: Some("waypoint"),
        selected: false,
    };
    assert!(alerts.observe(0.0, 1, &blocked).records.is_empty());
    assert!(
        alerts
            .observe(0.0, 2, &done("h-2-1", "explore", false))
            .records
            .is_empty()
    );
    assert!(alerts.queue().is_empty());
}

#[test]
fn already_selected_rover_is_queued_but_not_flashed() {
    let mut alerts = ArrivalAlerts::new(Cue::On);
    let shown = alerts.observe(0.0, 1, &done("h-1-1", "waypoint", true));
    assert_eq!(shown.records[0]["selected"], true);
    assert!(!alerts.alert(1).unwrap().unseen());
    assert_eq!(alerts.queue().len(), 1);
}

#[test]
fn new_episode_replaces_the_old_alert() {
    let mut alerts = ArrivalAlerts::new(Cue::On);
    alerts.observe(0.0, 1, &done("h-1-1", "waypoint", false));
    let next = alerts.observe(5.0, 1, &done("h-1-2", "waypoint", false));
    assert_eq!(events(&next), ["closed", "shown"]);
    assert_eq!(alerts.alert(1).unwrap().help_id, "h-1-2");
}

#[test]
fn silent_and_off_cues() {
    let mut silent = ArrivalAlerts::new(Cue::Silent);
    let shown = silent.observe(0.0, 1, &done("h-1-1", "waypoint", false));
    assert!(!shown.chime);
    assert_eq!(shown.records[0]["cues"], json!(["flash", "queue"]));
    let mut off = ArrivalAlerts::new(Cue::Off);
    assert_eq!(
        off.observe(0.0, 1, &done("h-1-1", "waypoint", false)),
        Update::default()
    );
    assert!(off.queue().is_empty());
    assert_eq!(off.take_config_record().unwrap()["cue"], "off");
    assert!(off.take_config_record().is_none());
}

#[test]
fn cue_parse() {
    assert_eq!(Cue::parse("OFF"), Cue::Off);
    assert_eq!(Cue::parse("silent"), Cue::Silent);
    assert_eq!(Cue::parse("on"), Cue::On);
    assert_eq!(Cue::parse("whatever"), Cue::On);
}

#[test]
fn queue_is_longest_wait_first() {
    let mut alerts = ArrivalAlerts::new(Cue::On);
    alerts.observe(5.0, 1, &done("h-1-1", "waypoint", false));
    alerts.observe(2.0, 2, &done("h-2-1", "waypoint", false));
    let order: Vec<u64> = alerts.queue().iter().map(|(id, _)| *id).collect();
    assert_eq!(order, [2, 1]);
}
