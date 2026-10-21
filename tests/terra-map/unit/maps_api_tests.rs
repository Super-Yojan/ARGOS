use super::*;
#[test]
fn fatal_worker_errors_reject_later_additions_until_restart() {
    let map = Map::new(MapConfig::default()).unwrap();
    let position = GeoPosition::new(0., 0., 0.);
    let id = map.add_3d_obj(position, "robot.glb").unwrap();
    let generation = map.worker_started();
    map.worker_failed(generation, "GPU unavailable".into());
    assert!(matches!(
        map.object_status(id).unwrap(),
        ObjectStatus::Failed(_)
    ));
    assert!(matches!(
        map.add_3d_obj(position, "robot.glb"),
        Err(MapError::WorkerUnavailable(_))
    ));
    map.worker_started();
    // A retired worker must not poison a replacement worker's controller.
    map.worker_failed(generation, "Old worker".into());
    assert_eq!(map.object_status(id).unwrap(), ObjectStatus::Loading);
    assert!(map.add_3d_obj(position, "robot.glb").is_ok());
}
#[test]
fn callback_controller_and_validation() {
    let map = Map::new(MapConfig::default()).unwrap();
    let callback_map = map.clone();
    std::thread::spawn(move || callback_map.show_lat_lon(40., -74.).unwrap())
        .join()
        .unwrap();
    assert_eq!(map.snapshot().navigation.unwrap().1.latitude, 40.);
    assert!(map.show_lat_lon(91., 0.).is_err());
    assert!(map.show_lat_lon(0., f64::NAN).is_err());
}
#[test]
fn handles_are_map_scoped_and_removed_objects_stay_removed() {
    let map = Map::new(MapConfig::default()).unwrap();
    let other = Map::new(MapConfig::default()).unwrap();
    let id = map
        .add_3d_obj(GeoPosition::new(0., 0., 10.), "robot.glb")
        .unwrap();
    assert!(other.remove_object(id).is_err());
    map.illuminate_3d_object(id, Color::rgb(255, 0, 0)).unwrap();
    assert_eq!(
        map.snapshot().objects[0].highlight,
        Some(Color::rgb(255, 0, 0))
    );
    map.remove_object(id).unwrap();
    map.report(id, ObjectStatus::Ready);
    assert!(map.object_status(id).is_err());
    assert!(map.poll_event().is_none());
}
#[test]
fn capacity_transform_and_model_validation() {
    let map = Map::new(MapConfig {
        max_objects: 1,
        ..Default::default()
    })
    .unwrap();
    assert!(
        map.add_3d_obj(GeoPosition::new(0., 0., 0.), "robot.obj")
            .is_err()
    );
    let id = map
        .add_3d_obj(GeoPosition::new(0., 0., 0.), "robot.gltf")
        .unwrap();
    assert!(map.set_object_scale(id, 0.).is_err());
    assert!(map.set_object_heading(id, f64::INFINITY).is_err());
    assert!(
        map.add_3d_obj(GeoPosition::new(0., 0., 0.), "another.glb")
            .is_err()
    );
    assert_eq!(map.object_status(id).unwrap(), ObjectStatus::Loading);
    map.report(id, ObjectStatus::Ready);
    assert!(
        matches!(map.poll_event(), Some(MapEvent::ObjectLoaded { id: actual }) if actual == id)
    );
}
