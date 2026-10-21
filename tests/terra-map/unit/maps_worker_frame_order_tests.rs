use super::*;
fn worker_with(frame_epoch: u64, generation: u64, delivered: u64) -> MapWorker {
    MapWorker {
        mailbox: Arc::new(Mutex::new(Mailbox {
            epoch: 2,
            generation: 10,
            delivered_generation: delivered,
            frame: Some(Frame {
                epoch: frame_epoch,
                generation,
                width: 1,
                height: 1,
                pixels: Some(vec![0; 4]),
                credits: String::new(),
                status: String::new(),
                tiles: 1,
                metrics: Default::default(),
                completed_at: Instant::now(),
            }),
            ..Default::default()
        })),
        stop: Arc::new(AtomicBool::new(false)),
    }
}
#[test]
fn resize_or_mode_change_rejects_old_epoch() {
    assert!(worker_with(1, 9, 0).take_frame().is_none());
}
#[test]
fn completed_frames_never_regress() {
    assert!(worker_with(2, 8, 9).take_frame().is_none());
}
#[test]
fn camera_updates_do_not_starve_completed_frames() {
    assert_eq!(worker_with(2, 9, 8).take_frame().unwrap().generation, 9);
}
