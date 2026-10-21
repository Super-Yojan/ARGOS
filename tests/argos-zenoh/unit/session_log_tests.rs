use super::*;
#[test]
fn unavailable_recorder_exposes_incomplete_without_control_dependencies() {
    let log = RunRecorder::unavailable();
    assert!(log.failed());
    assert!(log.record(serde_json::json!({"kind": "action"})).is_err());
    assert!(log.flush().is_err());
}
#[test]
fn export_barrier_keeps_complete_lines_and_tail_recovery() {
    let path = std::env::temp_dir().join(format!("argos-writer-test-{}.jsonl", std::process::id()));
    let log = RunRecorder::create(&path, serde_json::json!({}), 16).unwrap();
    log.record(serde_json::json!({"kind": "action","token": "correlated"}))
        .unwrap();
    log.flush().unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.extend_from_slice(b"{broken");
    assert_eq!(read_records(&bytes).unwrap().len(), 2);
    log.close().unwrap();
    assert!(
        read_records(&std::fs::read(&path).unwrap())
            .unwrap()
            .last()
            .unwrap()["complete"]
            == true
    );
    std::fs::remove_file(path).unwrap();
}
