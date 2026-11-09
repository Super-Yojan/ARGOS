use super::*;

fn item(key: &str, rover: u64, urgency: u8) -> Item {
    Item {
        key: key.into(),
        rover,
        level: "L3 Planned".into(),
        need: Need::Hold,
        urgency,
        text: key.into(),
        rec_id: None,
    }
}

#[test]
fn ranks_by_urgency_then_wait() {
    let mut tracker = Tracker::default();
    tracker.update(&[item("a", 1, 2)], 0.);
    tracker.update(&[item("a", 1, 2), item("b", 2, 2), item("c", 3, 3)], 10.);
    let order: Vec<_> = tracker
        .ranked(20.)
        .into_iter()
        .map(|(i, _)| i.key)
        .collect();
    assert_eq!(order, ["c", "a", "b"]);
    assert_eq!(tracker.waited("a", 20.), 20.);
}

#[test]
fn leaving_the_queue_resolves_with_the_wait() {
    let mut tracker = Tracker::default();
    tracker.update(&[item("a", 1, 2)], 5.);
    tracker.update(&[], 47.);
    let done: Vec<_> = tracker.resolved().collect();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].waited_s, 42.);
    assert_eq!(tracker.mean_wait(48.), None);
    tracker.update(&[], 47. + RESOLVED_WINDOW_S + 1.);
    assert_eq!(tracker.resolved().count(), 0);
}

#[test]
fn clock_formats_minutes_and_seconds() {
    assert_eq!(clock(72.), "01:12");
    assert_eq!(clock(-3.), "00:00");
}
