use super::super::{
    MapConfig,
    api::{Color, GeoPosition},
};
use super::*;
#[test]
#[ignore = "Requires a native GPU"]
fn gpu_async_models_report_failures_and_do_not_resurrect_removed_ids() {
    let map = Map::new(MapConfig::default()).unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/assets/robot.glb");
    let position = GeoPosition::new(37.7749, -122.4194, 80.);
    let removed = map.add_3d_obj(position, &path).unwrap();
    let ready = map.add_3d_obj(position, &path).unwrap();
    let alias = map
        .add_3d_obj(position, path.parent().unwrap().join("../assets/robot.glb"))
        .unwrap();
    let failed = map
        .add_3d_obj(position, "/private/tmp/terra-map-missing-model.glb")
        .unwrap();
    map.illuminate_3d_object(ready, Color::rgb(255, 100, 0))
        .unwrap();
    let mut objects = Objects::new(Some(map.clone()), 1024 * 1024);
    let mut renderer = Renderer::new().unwrap();
    objects.poll(&mut renderer).unwrap();
    map.remove_object(removed).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        objects.poll(&mut renderer).unwrap();
        if map.object_status(ready).unwrap() == ObjectStatus::Ready
            && map.object_status(alias).unwrap() == ObjectStatus::Ready
            && matches!(map.object_status(failed).unwrap(), ObjectStatus::Failed(_))
        {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(map.object_status(ready).unwrap(), ObjectStatus::Ready);
    assert!(matches!(
        map.object_status(failed).unwrap(),
        ObjectStatus::Failed(_)
    ));
    assert!(map.object_status(removed).is_err());
    assert_eq!(map.object_status(alias).unwrap(), ObjectStatus::Ready);
    assert_eq!(objects.models.len(), 1);
    assert!(renderer.has_objects());
    map.remove_object(ready).unwrap();
    map.remove_object(alias).unwrap();
    objects.poll(&mut renderer).unwrap();
    assert!(objects.models.is_empty());
    assert!(!renderer.has_objects());
}
