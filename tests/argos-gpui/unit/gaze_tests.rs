use super::*;

fn area(kind: &'static str, rover: Option<u64>, x: f32, y: f32) -> Area {
    Area {
        kind,
        rover,
        x,
        y,
        width: 10.,
        height: 10.,
    }
}

#[test]
fn parses_tracker_messages() {
    assert_eq!(
        GazeMessage::parse(br#"{"type":"gaze","utc":5.0,"x":10,"y":20.5,"confidence":0.8}"#),
        Some(GazeMessage::Gaze {
            utc: 5.0,
            x: 10.,
            y: 20.5,
            confidence: 0.8
        })
    );
    assert_eq!(
        GazeMessage::parse(br#"{"type":"blink","utc":6.0,"duration_ms":120}"#),
        Some(GazeMessage::Blink {
            utc: 6.0,
            duration_ms: 120.
        })
    );
    assert_eq!(GazeMessage::parse(br#"{"type":"gaze","utc":1}"#), None);
    assert_eq!(GazeMessage::parse(b"not json"), None);
}

#[test]
fn first_matching_area_wins() {
    let areas = [area("card", Some(2), 0., 0.), area("map", None, 0., 0.)];
    assert_eq!(
        hit(&areas, 5., 5., (100., 100.)),
        Target {
            kind: "card",
            rover: Some(2)
        }
    );
    assert_eq!(hit(&areas, 50., 50., (100., 100.)).kind, "window");
    assert_eq!(hit(&areas, 150., 50., (100., 100.)).kind, "off_window");
}

#[test]
fn dwell_ignores_short_glances() {
    let a = Target {
        kind: "card",
        rover: Some(1),
    };
    let b = Target {
        kind: "card",
        rover: Some(2),
    };
    let mut dwell = DwellTracker::default();
    assert_eq!(dwell.feed(a.clone(), 0.0), None);
    assert_eq!(dwell.feed(b.clone(), 1.0), None);
    // 0.05 s on rover 2 is a saccade; back on rover 1.
    assert_eq!(dwell.feed(a.clone(), 1.05), None);
    assert_eq!(dwell.feed(b.clone(), 2.0), None);
    let ended = dwell.feed(b.clone(), 2.2).unwrap();
    assert_eq!(ended.target, a);
    assert_eq!(ended.start, 0.0);
    assert_eq!(ended.duration, 2.0);
    let last = dwell.finish(3.0).unwrap();
    assert_eq!(last.target, b);
    assert_eq!(last.duration, 1.0);
}
