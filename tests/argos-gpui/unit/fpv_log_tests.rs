use super::*;

fn sample(phase: &'static str, detail: &str, frames: u64) -> FpvSample {
    FpvSample {
        phone_id: Some(1),
        session: Some("s-1".into()),
        phase,
        detail: detail.into(),
        codec: "H264".into(),
        width: 640,
        height: 480,
        fps: 24.,
        bitrate_kbps: 900.,
        rtt_ms: Some(40.),
        frame_seq: frames,
    }
}

fn events(records: &[Value]) -> Vec<&str> {
    records
        .iter()
        .map(|record| record["event"].as_str().unwrap())
        .collect()
}

#[test]
fn quiet_window_logs_nothing() {
    let mut log = FpvLog::default();
    let idle = FpvSample {
        phase: "Idle",
        detail: "Select a rover, then start FPV".into(),
        ..FpvSample::default()
    };
    assert!(log.observe(0.0, &idle).is_empty());
    assert!(log.observe(5.0, &idle).is_empty());
}

#[test]
fn healthy_call_logs_phases_first_frame_and_periodic_stats() {
    let mut log = FpvLog::default();
    let start = log.action(1.0, "start", Some(1), None);
    assert_eq!(start["event"], "start");
    let records = log.observe(1.1, &sample("Starting", "Asking rover 1 for video", 0));
    assert_eq!(events(&records), ["phase"]);
    assert_eq!(records[0]["phase"], "Starting");
    let records = log.observe(1.5, &sample("Connecting", "Waiting for the phone", 0));
    assert_eq!(events(&records), ["phase"]);
    let records = log.observe(2.0, &sample("Connected", "Video call connected", 0));
    assert_eq!(events(&records), ["phase", "stats"]);
    let records = log.observe(2.1, &sample("Connected", "Video call connected", 3));
    assert_eq!(events(&records), ["first_frame"]);
    assert!((records[0]["since_start_s"].as_f64().unwrap() - 1.1).abs() < 1e-9);
    // Frames keep coming: nothing until the next stats period.
    for step in 1..50 {
        let now = 2.1 + f64::from(step) * 0.1;
        let records = log.observe(
            now,
            &sample("Connected", "Video call connected", 3 + step as u64),
        );
        assert!(records.is_empty(), "unexpected {records:?} at {now}");
    }
    let records = log.observe(12.1, &sample("Connected", "Video call connected", 60));
    assert_eq!(events(&records), ["stats"]);
    assert_eq!(records[0]["frames"], 60);
}

#[test]
fn connected_without_pictures_is_frozen_then_resumed() {
    let mut log = FpvLog::default();
    log.action(0.0, "start", Some(1), None);
    log.observe(0.1, &sample("Starting", "Asking rover 1 for video", 0));
    log.observe(0.5, &sample("Connected", "Video call connected", 0));
    // Never got a first frame: frozen counts from Start.
    let records = log.observe(2.0, &sample("Connected", "Video call connected", 0));
    assert_eq!(events(&records), ["frozen"]);
    assert!(
        log.observe(3.0, &sample("Connected", "Video call connected", 0))
            .is_empty()
    );
    let records = log.observe(4.0, &sample("Connected", "Video call connected", 1));
    assert_eq!(events(&records), ["first_frame", "resumed"]);
    // Pictures stop again mid-call.
    let records = log.observe(6.5, &sample("Connected", "Video call connected", 1));
    assert_eq!(events(&records), ["frozen"]);
    assert_eq!(records[0]["no_frame_s"], 2.5);
    let records = log.observe(7.0, &sample("Connected", "Video call connected", 2));
    assert_eq!(events(&records), ["resumed"]);
    assert_eq!(records[0]["gap_s"], 3.0);
}

#[test]
fn failure_records_detail_and_frozen_time() {
    let mut log = FpvLog::default();
    log.action(0.0, "start", Some(1), None);
    log.observe(0.1, &sample("Starting", "Asking rover 1 for video", 0));
    log.observe(0.5, &sample("Connected", "Video call connected", 5));
    log.observe(3.0, &sample("Connected", "Video call connected", 5));
    let records = log.observe(4.0, &sample("Failed", "WebRTC connection failed", 5));
    assert_eq!(events(&records), ["phase"]);
    assert_eq!(records[0]["phase"], "Failed");
    assert_eq!(records[0]["from"], "Connected");
    assert_eq!(records[0]["detail"], "WebRTC connection failed");
    assert_eq!(records[0]["since_start_s"], 4.0);
    assert_eq!(records[0]["frozen_s"], 3.5);
    // A failed call is not frozen and logs no stats.
    assert!(
        log.observe(20.0, &sample("Failed", "WebRTC connection failed", 5))
            .is_empty()
    );
}

#[test]
fn timeout_before_connecting_is_logged() {
    let mut log = FpvLog::default();
    log.action(0.0, "start", Some(2), None);
    log.observe(0.1, &sample("Starting", "Asking rover 2 for video", 0));
    let records = log.observe(10.0, &sample("Failed", "The video call timed out", 0));
    assert_eq!(events(&records), ["phase"]);
    assert_eq!(records[0]["detail"], "The video call timed out");
    assert!(records[0].get("frozen_s").is_none());
}

#[test]
fn rotate_action_carries_degrees() {
    let mut log = FpvLog::default();
    let record = log.action(0.0, "rotate", Some(1), Some(3));
    assert_eq!(record["rotation_deg"], 270);
    assert_eq!(record["kind"], "fpv");
}
