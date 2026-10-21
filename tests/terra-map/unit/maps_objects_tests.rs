use super::super::{MapConfig, api::GeoPosition};
use super::*;
#[test]
fn static_local_model_and_enu_placement() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/maps/fixtures/textured-quad.glb");
    let content = load(&path, 8 * 1024 * 1024).unwrap();
    assert!(!content.primitives.is_empty());
    assert!(load(&path, 1).is_err());
    let map = Map::new(MapConfig::default()).unwrap();
    let id = map
        .add_3d_obj(GeoPosition::new(0., 0., 100.), path)
        .unwrap();
    let snapshot = map.snapshot();
    let world = transform(&snapshot.objects[0]);
    assert_eq!(world.w_axis.truncate(), ecef(0., 0., 100.));
    assert!(world.transform_vector3(DVec3::X).distance(DVec3::Y) < 1e-9);
    map.set_object_heading(id, 90.).unwrap();
    let world = transform(&map.snapshot().objects[0]);
    assert!(world.transform_vector3(DVec3::X).distance(-DVec3::Z) < 1e-9);
}
#[test]
fn gltf_external_resources_resolve_locally() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/maps/fixtures/rgb-quad.gltf");
    assert!(load(&path, 8 * 1024 * 1024).is_ok());
}
