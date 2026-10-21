use super::*;
fn frame(generation: u64, pixels: Option<Vec<u8>>) -> Frame {
    Frame {
        generation,
        epoch: 0,
        width: 1,
        height: 1,
        pixels,
        credits: "source".into(),
        status: String::new(),
        tiles: 1,
        metrics: Default::default(),
        completed_at: Instant::now(),
    }
}
#[test]
fn mailbox_preserves_pending_pixels_when_status_arrives() {
    let mailbox = Mutex::new(Mailbox::default());
    publish(&mailbox, frame(1, Some(vec![1, 2, 3, 255])));
    let mut status_frame = frame(1, None);
    status_frame.status = "Refining".into();
    publish(&mailbox, status_frame);
    let mailbox = mailbox.lock().unwrap();
    let frame = mailbox.frame.as_ref().unwrap();
    assert!(frame.pixels.is_some());
    assert_eq!(frame.status, "Refining");
}
#[test]
fn mailbox_delivers_completed_frame_while_camera_keeps_moving() {
    let worker = MapWorker {
        mailbox: Arc::new(Mutex::new(Mailbox {
            generation: 2,
            frame: Some(frame(1, Some(vec![0; 4]))),
            ..Default::default()
        })),
        stop: Arc::new(AtomicBool::new(false)),
    };
    assert_eq!(worker.take_frame().map(|f| f.generation), Some(1));
}
#[test]
fn error_notice_only_uses_current_failed_requests() {
    let now = Instant::now();
    let failed = HashMap::from([(
        1,
        (
            "temporary error".into(),
            Some(now + Duration::from_secs(30)),
        ),
    )]);
    assert!(request_notice(&[2], &failed, now).is_none());
    assert_eq!(
        request_notice(&[1], &failed, now),
        Some("temporary error".into())
    );
    assert!(request_notice(&[1], &failed, now + Duration::from_secs(31)).is_none());
    assert!(request_notice(&[1], &HashMap::new(), now).is_none());
}
