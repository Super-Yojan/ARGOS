use super::super::{
    MapConfig,
    api::{Color, GeoPosition, Map},
    camera::Camera,
};
use super::*;
#[test]
#[ignore = "Requires a native GPU"]
fn gpu_objects_share_assets_and_highlight_independently() {
    let config = MapConfig {
        latitude: 0.,
        longitude: 0.,
        altitude: 0.,
        distance: 25.,
        pitch: 89.,
        survey: false,
        ..Default::default()
    };
    let camera = Camera::new(0., 0., 0., 25., 0., 89.);
    let map = Map::new(config).unwrap();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/assets/robot.glb");
    let left = map
        .add_3d_obj(GeoPosition::new(0., -0.00004, 0.), &path)
        .unwrap();
    let right = map
        .add_3d_obj(GeoPosition::new(0., 0.00004, 0.), &path)
        .unwrap();
    let models = HashMap::from([(
        path.clone(),
        super::super::objects::load(&path, 1024 * 1024).unwrap(),
    )]);
    let mut renderer = Renderer::new().unwrap();
    let snapshot = map.snapshot();
    assert!(renderer.sync_objects(&snapshot.objects, &models).is_empty());
    assert_eq!(renderer.models.len(), 1);
    assert_eq!(renderer.objects.len(), 2);
    let view = camera.view(320, 240);
    let original = renderer.render(&view, &[], 320, 240).unwrap();
    assert!(original.as_chunks::<4>().0.iter().any(|p| p[0] < 200));
    map.illuminate_3d_object(left, Color::rgb(255, 0, 0))
        .unwrap();
    renderer.sync_objects(&map.snapshot().objects, &models);
    let highlighted = renderer.render(&view, &[], 320, 240).unwrap();
    let mut changed_left = 0;
    for (i, (before, after)) in original
        .as_chunks::<4>()
        .0
        .iter()
        .zip(highlighted.as_chunks::<4>().0.iter())
        .enumerate()
    {
        if i % 320 < 160 {
            if before != after {
                changed_left += 1;
            }
        } else {
            assert_eq!(before, after, "Other instance must retain its material");
        }
    }
    assert!(
        changed_left > 50,
        "highlight should affect visible model pixels"
    );
    // Material toggles on map tiles must not recolor the object instances.
    renderer.set_survey(true);
    let surveyed = renderer.render(&view, &[], 320, 240).unwrap();
    for (a, b) in highlighted
        .as_chunks::<4>()
        .0
        .iter()
        .zip(surveyed.as_chunks::<4>().0.iter())
    {
        if a != &[255, 255, 255, 255] {
            assert_eq!(a, b);
        }
    }
    renderer.set_survey(false);
    map.clear_illumination(left).unwrap();
    renderer.sync_objects(&map.snapshot().objects, &models);
    assert_eq!(renderer.render(&view, &[], 320, 240).unwrap(), original);
    map.set_object_position(right, GeoPosition::new(0.00004, 0.00004, 0.))
        .unwrap();
    renderer.sync_objects(&map.snapshot().objects, &models);
    assert_ne!(renderer.render(&view, &[], 320, 240).unwrap(), original);
    map.remove_object(left).unwrap();
    map.remove_object(right).unwrap();
    renderer.sync_objects(&map.snapshot().objects, &HashMap::new());
    assert!(!renderer.has_objects());
    assert!(renderer.models.is_empty());
    assert!(
        renderer
            .render(&view, &[], 320, 240)
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p == &[255, 255, 255, 255])
    );
}
