use super::*;
#[test]
#[ignore = "Requires a native GPU"]
fn gpu_objects_load_and_update_without_google_credentials() {
    use super::super::api::{GeoPosition, Map, ObjectStatus};
    let config = MapConfig {
        api_key: None,
        demo: false,
        ..Default::default()
    };
    let map = Map::new(config.clone()).unwrap();
    let worker = MapWorker::start_with_map(config.clone(), Some(map.clone()));
    worker.update(Update {
        camera: config.camera(),
        width: 320,
        height: 240,
        generation: 1,
        epoch: 0,
        survey: true,
    });
    // Add after the worker has entered its missing-key fallback.
    thread::sleep(Duration::from_millis(50));
    let id = map
        .add_3d_obj(
            GeoPosition::new(config.latitude, config.longitude, 80.),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/assets/robot.glb"),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut rendered = false;
    while Instant::now() < deadline {
        if let Some(frame) = worker.take_frame() {
            assert!(frame.status.contains("Map tiles unavailable"));
            rendered |= frame.pixels.is_some();
        }
        if rendered && map.object_status(id).unwrap() == ObjectStatus::Ready {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(rendered);
    assert_eq!(map.object_status(id).unwrap(), ObjectStatus::Ready);
}
