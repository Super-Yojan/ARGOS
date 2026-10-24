use super::camera::*;
use super::tiles::*;
use glam::{DMat4, DVec3};
use url::Url;

#[test]
fn ground_pixels_round_trip_through_the_camera() {
    let mut camera = Camera::new(38.8297, -77.3075, 40., 400., 0., 90.);
    camera.set_mode(MapMode::TwoD);
    let (x, y) = camera.project_ground(800, 600, 38.8297, -77.3075).unwrap();
    assert!((x - 400.).abs() < 2.);
    assert!((y - 300.).abs() < 2.);
    let (lat, lon) = camera.unproject_ground(800, 600, x, y).unwrap();
    assert!((lat - 38.8297).abs() < 1e-5);
    assert!((lon + 77.3075).abs() < 1e-5);
    let north = 38.8297 + (80. / EARTH_RADIUS).to_degrees();
    let (nx, ny) = camera.project_ground(800, 600, north, -77.3075).unwrap();
    assert!(ny < y);
    let (back, _) = camera.unproject_ground(800, 600, nx, ny).unwrap();
    assert!((back - north).abs() < 1e-4);
}

#[test]
fn wgs84_equator_and_pole() {
    assert!((ecef(0.0, 0.0, 0.0) - DVec3::new(6_378_137.0, 0.0, 0.0)).length() < 0.001);
    assert!((ecef(90.0, 0.0, 0.0).z - 6_356_752.314_245).abs() < 0.001);
}

#[test]
fn authenticated_links_preserve_session_and_replace_key_once() {
    let parent =
        Url::parse("https://tile.googleapis.com/v1/root.json?key=old&session=abc&v=2").unwrap();
    let child = resolve_url(&parent, "child.glb?key=other", "secret").unwrap();
    let query: Vec<_> = child.query_pairs().into_owned().collect();
    assert!(query.contains(&("session".into(), "abc".into())));
    assert!(query.contains(&("v".into(), "2".into())));
    assert_eq!(query.iter().filter(|(k, _)| k == "key").count(), 1);
    assert!(query.contains(&("key".into(), "secret".into())));
    assert!(resolve_url(&parent, "https://attacker.test/tile.glb", "secret").is_err());
}

#[test]
fn nested_tiles_compose_transforms_and_inherit_refinement() {
    let json = serde_json::json!({"asset":{"version":"1.1"},"root":{
        "boundingVolume":{"sphere":[0,0,0,1]}, "geometricError":100, "refine":"ADD",
        "transform":DMat4::from_translation(DVec3::X).to_cols_array(),
        "children":[{"boundingVolume":{"sphere":[0,0,0,1]},"geometricError":0,
        "transform":DMat4::from_translation(DVec3::Y).to_cols_array()}]}});
    let tree = TileTree::parse(
        &serde_json::to_vec(&json).unwrap(),
        Url::parse("https://tile.googleapis.com/root.json").unwrap(),
    )
    .unwrap();
    assert!(tree.nodes[1].additive);
    assert!(
        (tree.nodes[1].transform.transform_point3(DVec3::ZERO) - DVec3::new(1., 1., 0.)).length()
            < 1e-8
    );
}

#[test]
fn replacement_waits_for_all_visible_children() {
    assert_eq!(
        replacement_ids(Some(1), vec![(true, vec![2]), (false, vec![])], false),
        (true, vec![1])
    );
    assert_eq!(
        replacement_ids(Some(1), vec![(true, vec![2]), (true, vec![3])], false),
        (true, vec![2, 3])
    );
    assert_eq!(
        replacement_ids(Some(1), vec![(true, vec![2])], true),
        (true, vec![1, 2])
    );
}

#[test]
fn camera_keeps_neighborhood_coordinates_precise() {
    let camera = Camera::new(37.7749, -122.4194, 50., 1800., 0., 50.);
    let view = camera.view(960, 600);
    let p = view
        .view_projection
        .project_point3(camera.target - view.origin);
    assert!(p.x.abs() < 1e-8 && p.y.abs() < 1e-8);
    assert!((0.0..1.0).contains(&p.z));
    assert!(view.visible(Sphere {
        center: camera.target,
        radius: 50.
    }));
    assert!(!view.visible(Sphere {
        center: view.eye + (view.eye - camera.target).normalize() * 1000.,
        radius: 1.
    }));
}

#[test]
fn region_bounds_contain_antimeridian_corners() {
    let bounds = region_sphere([
        170_f64.to_radians(),
        -0.1,
        -170_f64.to_radians(),
        0.1,
        0.,
        100.,
    ]);
    for lon in [170., 180., -170.] {
        for lat in [-0.1_f64, 0.1] {
            assert!((ecef(lat.to_degrees(), lon, 100.) - bounds.center).length() <= bounds.radius);
        }
    }
}

#[test]
fn draco_box_uses_extension_attribute_ids() {
    let source = Url::parse("https://tile.googleapis.com/test/box.gltf").unwrap();
    let content = super::content::decode(
        include_bytes!("../../../crates/terra-map/src/maps/fixtures/box-draco.gltf"),
        &source,
        |url| {
            assert_eq!(url.path(), "/test/Box.bin");
            Ok(include_bytes!("../../../crates/terra-map/src/maps/fixtures/Box.bin").to_vec())
        },
    )
    .unwrap();
    assert_eq!(content.primitives.len(), 1);
    let p = &content.primitives[0];
    assert_eq!(p.indices.len(), 36);
    assert_eq!(p.vertices.len(), 24);
    assert!(
        p.vertices
            .iter()
            .all(|v| v.position.iter().all(|x| x.abs() < 0.51))
    );
}

#[test]
fn attribution_is_aggregated_by_frequency() {
    assert_eq!(copyrights(["B; A", "B; C", "A"].into_iter()), "A; B; C");
}

#[test]
fn embedded_textured_glb_decodes_images_and_uvs() {
    let content = super::content::decode(
        include_bytes!("../../../crates/terra-map/src/maps/fixtures/textured-quad.glb"),
        &Url::parse("https://tile.googleapis.com/quad.glb").unwrap(),
        |_| panic!("embedded image must not fetch"),
    )
    .unwrap();
    let p = &content.primitives[0];
    assert_eq!(p.vertices.len(), 4);
    assert_eq!(p.vertices[0].uv, [0., 1.]);
    assert_eq!(p.texture.width(), 2);
    assert_eq!(p.texture.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(content.copyright, "Synthetic test fixture");
}

#[test]
#[ignore = "Requires a native GPU; run explicitly with --ignored"]
fn gpu_smoke_depth_texture_and_padded_rows() {
    let source = Url::parse("https://tile.googleapis.com/quad.glb").unwrap();
    let mut content = super::content::decode(
        include_bytes!("../../../crates/terra-map/src/maps/fixtures/textured-quad.glb"),
        &source,
        |_| unreachable!(),
    )
    .unwrap();
    content.primitives[0].transform = DMat4::IDENTITY;
    let mut back = content.primitives[0].clone();
    back.transform = DMat4::from_translation(DVec3::new(0., 0., -1.));
    back.texture = std::sync::Arc::new(image::RgbaImage::from_pixel(
        1,
        1,
        image::Rgba([255, 0, 255, 255]),
    ));
    content.primitives.push(back);
    let mut renderer = super::renderer::Renderer::new().unwrap();
    renderer.upload(0, &content, DMat4::IDENTITY).unwrap();
    let view = View {
        origin: DVec3::ZERO,
        eye: DVec3::new(0., 0., 5.),
        height: 129,
        meters_per_pixel: None,
        view_projection: DMat4::perspective_rh(50_f64.to_radians(), 257. / 129., 0.1, 100.)
            * DMat4::look_at_rh(DVec3::new(0., 0., 5.), DVec3::ZERO, DVec3::Y),
    };
    let pixels = renderer.render(&view, &[0], 257, 129).unwrap();
    assert_eq!(pixels.len(), 257 * 129 * 4);
    // Nearest filtering keeps the center quadrants unambiguous. BGRA output.
    let pixel = |x: usize, y: usize| &pixels[(y * 257 + x) * 4..(y * 257 + x + 1) * 4];
    assert_eq!(pixel(115, 50), [0, 0, 255, 255]);
    assert_eq!(pixel(140, 50), [0, 255, 0, 255]);
    assert_eq!(pixel(115, 78), [255, 0, 0, 255]);
    assert_eq!(pixel(140, 78), [255, 255, 255, 255]);
    let rgba: Vec<_> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[2], p[1], p[0], p[3]])
        .collect();
    image::RgbaImage::from_raw(257, 129, rgba)
        .unwrap()
        .save("/private/tmp/learn-gpui-map-smoke.png")
        .unwrap();
    renderer.set_survey(true);
    let toned = renderer.render(&view, &[0], 257, 129).unwrap();
    let p = &toned[(50 * 257 + 115) * 4..(50 * 257 + 115) * 4 + 4];
    assert!(
        p[0] > p[1] && p[1] > p[2],
        "Survey mode should tint the scene blue-green: {p:?}"
    );
    assert_ne!(toned, pixels);
    let rgba: Vec<_> = toned
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[2], p[1], p[0], p[3]])
        .collect();
    image::RgbaImage::from_raw(257, 129, rgba)
        .unwrap()
        .save("/private/tmp/learn-gpui-map-survey-smoke.png")
        .unwrap();
    let empty = renderer.render(&view, &[], 257, 129).unwrap();
    assert_ne!(&empty[(50 * 257 + 115) * 4..(50 * 257 + 115) * 4 + 4], p);
}

#[test]
fn viewport_budget_preserves_aspect_and_handles_zero_size() {
    assert_eq!(super::worker::render_size(0., 600., 2., 1_000_000), None);
    let (w, h) = super::worker::render_size(3840., 2160., 2., 1_000_000).unwrap();
    assert!(u64::from(w) * u64::from(h) <= 1_000_000);
    assert!((w as f64 / h as f64 - 16. / 9.).abs() < 0.01);
    assert!(super::worker::render_size(f64::NAN, 600., 2., 1_000_000).is_none());
}

#[test]
fn local_http_fixture_loads_nested_hierarchy_and_redacts_failures() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for (path,body,status) in [("/root.json",br#"{"root":{"boundingVolume":{"sphere":[0,0,0,1]},"geometricError":0,"content":{"uri":"child.json?session=test"}}}"#.as_slice(),"200 OK"),("/child.json",br#"{"root":{"boundingVolume":{"sphere":[0,0,0,1]},"geometricError":0}}"#.as_slice(),"200 OK"),("/denied",b"denied".as_slice(),"403 Forbidden")] {
            let (mut stream,_)=listener.accept().unwrap();let mut request=[0;2048];let n=stream.read(&mut request).unwrap();
            let request=String::from_utf8_lossy(&request[..n]);assert!(request.starts_with(&format!("GET {path}")));
            write!(stream,"HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",body.len()).unwrap();stream.write_all(body).unwrap();
        }
    });
    let http = HttpClient::new().unwrap();
    let base = Url::parse(&format!("http://{addr}/root.json?key=hidden")).unwrap();
    let response = http.get(&base).unwrap();
    assert_eq!(response.cache_for, std::time::Duration::ZERO);
    let mut tree = TileTree::parse(&response.bytes, base.clone()).unwrap();
    let url = tree.nodes[0].content.clone().unwrap();
    assert!(url.query_pairs().any(|(k, v)| k == "key" && v == "hidden"));
    let child = http.get(&url).unwrap();
    tree.expand(0, &child.bytes).unwrap();
    assert_eq!(tree.nodes.len(), 2);
    let error = http
        .get(&base.join("denied?key=hidden").unwrap())
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("403"));
    assert!(!error.contains("hidden"));
    server.join().unwrap();
}

#[test]
fn malformed_accessor_range_is_rejected_without_panic() {
    let source = Url::parse("https://tile.googleapis.com/bad.gltf").unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../crates/terra-map/src/maps/fixtures/box-draco.gltf"
    ))
    .unwrap();
    json["bufferViews"][0]["byteOffset"] = serde_json::json!(1_000_000);
    let error = super::content::decode(&serde_json::to_vec(&json).unwrap(), &source, |_| {
        Ok(include_bytes!("../../../crates/terra-map/src/maps/fixtures/Box.bin").to_vec())
    })
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("buffer view range"));
}

#[test]
fn unknown_required_extensions_are_reported() {
    let source = Url::parse("https://tile.googleapis.com/bad.gltf").unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../crates/terra-map/src/maps/fixtures/box-draco.gltf"
    ))
    .unwrap();
    json["extensionsRequired"] = serde_json::json!(["EXT_unsupported"]);
    let error = super::content::decode(&serde_json::to_vec(&json).unwrap(), &source, |_| {
        panic!("Must reject before network")
    })
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("EXT_unsupported"));
}

#[test]
fn selection_keeps_ready_children_while_parent_covers_pending_sibling() {
    let json = serde_json::json!({"root":{"boundingVolume":{"sphere":[0,0,0,1]},"geometricError":100,"content":{"uri":"parent.glb"},"children":[
        {"boundingVolume":{"sphere":[0,0,0,1]},"geometricError":0,"content":{"uri":"a.glb"}},
        {"boundingVolume":{"sphere":[0,0,0,1]},"geometricError":0,"content":{"uri":"b.glb"}}
    ]}});
    let tree = TileTree::parse(
        &serde_json::to_vec(&json).unwrap(),
        Url::parse("https://tile.googleapis.com/root.json").unwrap(),
    )
    .unwrap();
    let view = View {
        origin: DVec3::ZERO,
        eye: DVec3::new(0., 0., 5.),
        height: 600,
        meters_per_pixel: None,
        view_projection: DMat4::perspective_rh(1., 1., 0.1, 100.)
            * DMat4::look_at_rh(DVec3::new(0., 0., 5.), DVec3::ZERO, DVec3::Y),
    };
    let selection = tree.select(&view, &[0, 1].into_iter().collect(), 16.);
    assert_eq!(selection.draw, vec![0]);
    assert!(selection.keep.contains(&1));
    assert!(selection.requests.contains(&2));
}

#[test]
fn node_translation_preserves_double_precision() {
    let glb = gltf::binary::Glb::from_slice(include_bytes!(
        "../../../crates/terra-map/src/maps/fixtures/textured-quad.glb"
    ))
    .unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&glb.json).unwrap();
    json["nodes"][0]["translation"] = serde_json::json!([6378137.0123456, 9.123456789, 1.23456789]);
    json["buffers"][0]["uri"] = serde_json::json!("buffer.bin");
    let bin = glb.bin.unwrap().into_owned();
    let content = super::content::decode(
        &serde_json::to_vec(&json).unwrap(),
        &Url::parse("https://tile.googleapis.com/test.gltf").unwrap(),
        |_| Ok(bin.clone()),
    )
    .unwrap();
    let actual = content.primitives[0]
        .transform
        .transform_point3(DVec3::ZERO);
    assert!(
        (actual - DVec3::new(6378137.0123456, -1.23456789, 9.123456789)).length() < 1e-8,
        "{actual:?}"
    );
}

#[test]
fn empty_selection_clears_old_scene_after_pending_work_finishes() {
    assert!(super::worker::should_render(true, true, false, 0));
    assert!(!super::worker::should_render(true, true, false, 2));
    assert!(super::worker::should_render(true, false, true, 2));
    assert!(!super::worker::should_render(false, false, true, 0));
}

#[test]
fn draco_rgb_color_and_nonidentity_mapping_are_preserved() {
    let content = super::content::decode(
        include_bytes!("../../../crates/terra-map/src/maps/fixtures/rgb-quad.gltf"),
        &Url::parse("https://tile.googleapis.com/rgb.gltf").unwrap(),
        |_| Ok(include_bytes!("../../../crates/terra-map/src/maps/fixtures/rgb-quad.drc").to_vec()),
    )
    .unwrap();
    for v in &content.primitives[0].vertices {
        let expected = if v.position[0] * v.position[1] > 0. {
            [0., 1., 0., 1.]
        } else {
            [1., 0., 0., 1.]
        };
        assert_eq!(v.color, expected);
    }
}

#[test]
#[ignore = "Live Google API, network and GPU; run explicitly with GOOGLE_MAPS_API_KEY"]
fn live_google_tiles_render() {
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let config = super::MapConfig {
        api_key: Some(
            std::env::var("GOOGLE_MAPS_API_KEY")
                .expect("Set GOOGLE_MAPS_API_KEY for the live check"),
        ),
        demo: false,
        ..Default::default()
    };
    let camera = Camera::new(
        config.latitude,
        config.longitude,
        config.altitude,
        config.distance,
        config.heading,
        config.pitch,
    );
    let worker = super::worker::MapWorker::start(config);
    worker.update(super::worker::Update {
        camera: camera.clone(),
        width: 960,
        height: 600,
        generation: 1,
        epoch: 0,
        survey: true,
    });
    let started = Instant::now();
    let mut best = None;
    let mut status = String::new();
    while started.elapsed() < Duration::from_secs(75) {
        if let Some(frame) = worker.take_frame() {
            if frame.status != status && !frame.status.is_empty() {
                println!("Live check: {}", frame.status);
            }
            status = frame.status.clone();
            if best.is_none()
                && (status.contains("Unsupported required") || status.contains("Invalid glTF"))
            {
                panic!("{status}");
            }
            if frame.pixels.is_some() && frame.tiles > 0 {
                let enough = frame.tiles >= 8 && started.elapsed() > Duration::from_secs(20);
                best = Some(frame);
                if enough {
                    break;
                }
            }
        }
        if best.is_some() && started.elapsed() > Duration::from_secs(30) {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let mut moving_camera = camera.clone();
    let mut displayed = 0;
    let mut renders = Vec::new();
    let mut waits = Vec::new();
    let mut downloads = Vec::new();
    let mut decodes = Vec::new();
    for generation in 2..=121 {
        moving_camera.pan(0.5, 0.);
        worker.update(super::worker::Update {
            camera: moving_camera.clone(),
            width: 960,
            height: 600,
            generation,
            epoch: 0,
            survey: true,
        });
        thread::sleep(Duration::from_millis(16));
        if let Some(frame) = worker.take_frame()
            && frame.pixels.is_some()
        {
            displayed += 1;
            renders.push(frame.metrics.render_ms);
            waits.push(frame.metrics.gpu_wait_ms);
            downloads.push(frame.metrics.download_ms);
            decodes.push(frame.metrics.decode_ms);
        }
    }
    let percentile = |values: &mut Vec<f64>, percent: usize| {
        values.sort_by(f64::total_cmp);
        values
            .get((values.len().saturating_sub(1)) * percent / 100)
            .copied()
            .unwrap_or(0.)
    };
    println!(
        "Motion profile: {displayed}/120 camera updates delivered; render+readback p50={:.2}ms p95={:.2}ms; GPU wait p50={:.2}ms; recent tile download p50={:.2}ms decode p50={:.2}ms",
        percentile(&mut renders, 50),
        percentile(&mut renders, 95),
        percentile(&mut waits, 50),
        percentile(&mut downloads, 50),
        percentile(&mut decodes, 50)
    );
    moving_camera.set_mode(super::camera::MapMode::TwoD);
    worker.update(super::worker::Update {
        camera: moving_camera.clone(),
        width: 960,
        height: 600,
        generation: 122,
        epoch: 1,
        survey: true,
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut two_d = None;
    while Instant::now() < deadline {
        if let Some(frame) = worker.take_frame()
            && frame.pixels.is_some()
            && frame.tiles > 0
        {
            assert_eq!(frame.epoch, 1);
            two_d = Some(frame);
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let two_d = two_d.expect("2D map should render cached Google tiles");
    let rgba: Vec<_> = two_d
        .pixels
        .unwrap()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[2], p[1], p[0], p[3]])
        .collect();
    image::RgbaImage::from_raw(960, 600, rgba)
        .unwrap()
        .save("/private/tmp/learn-gpui-google-2d.png")
        .unwrap();
    moving_camera.set_mode(super::camera::MapMode::ThreeD);
    worker.update(super::worker::Update {
        camera: moving_camera,
        width: 960,
        height: 600,
        generation: 123,
        epoch: 2,
        survey: true,
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut restored = false;
    while Instant::now() < deadline {
        if let Some(frame) = worker.take_frame()
            && frame.pixels.is_some()
        {
            assert_eq!(frame.epoch, 2);
            restored = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(restored, "3D should render again after mode switch");
    println!("Live mode switches: 3D → 2D → 3D passed");
    let frame = best.unwrap_or_else(|| panic!("No live Google tiles rendered: {status}"));
    let pixels = frame.pixels.unwrap();
    assert!(
        pixels.as_chunks::<4>().0.windows(2).any(|p| p[0] != p[1]),
        "Live frame contains only background"
    );
    println!(
        "Live Google scene rendered: {} tiles, {} × {}, attribution present: {}",
        frame.tiles,
        frame.width,
        frame.height,
        !frame.credits.is_empty()
    );
    let rgba: Vec<_> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[2], p[1], p[0], p[3]])
        .collect();
    image::RgbaImage::from_raw(frame.width, frame.height, rgba)
        .unwrap()
        .save("/private/tmp/learn-gpui-google-live.png")
        .unwrap();
    std::fs::write(
        "/private/tmp/learn-gpui-google-live-attribution.txt",
        format!("Google Maps\n{}", frame.credits),
    )
    .unwrap();
}

#[test]
fn google_gltf_content_extension_is_supported() {
    let json=br#"{"extensionsRequired":["3DTILES_content_gltf"],"root":{"boundingVolume":{"sphere":[0,0,0,1]},"geometricError":0,"content":{"uri":"mesh.glb"}}}"#;
    assert!(
        TileTree::parse(
            json,
            Url::parse("https://tile.googleapis.com/root.json").unwrap()
        )
        .is_ok()
    );
}

#[test]
fn identity_tile_transform_preserves_bounding_sphere_radius() {
    let tree = TileTree::parse(
        br#"{"root":{"boundingVolume":{"sphere":[0,0,0,1]},"geometricError":16}}"#,
        Url::parse("https://tile.googleapis.com/root.json").unwrap(),
    )
    .unwrap();
    assert!((tree.nodes[0].bounds.radius - 1.).abs() < 1e-10);
    assert!((tree.nodes[0].error - 16.).abs() < 1e-10);
}

#[test]
fn two_d_is_north_up_parallel_and_restores_three_d_camera() {
    use super::camera::MapMode;
    let mut camera = Camera::new(37.7749, -122.4194, 40., 1800., 35., 50.);
    let original = (camera.heading, camera.pitch, camera.target, camera.distance);
    camera.set_mode(MapMode::TwoD);
    assert_eq!(camera.heading, 0.);
    assert_eq!(camera.pitch, std::f64::consts::FRAC_PI_2);
    let view = camera.view(960, 600);
    assert!(view.view_projection.is_finite());
    let lon = camera.longitude.to_radians();
    let east = DVec3::new(-lon.sin(), lon.cos(), 0.);
    let up = (view.eye - view.origin).normalize();
    let length_at = |height: f64| {
        let a = view.view_projection.project_point3(up * height);
        let b = view
            .view_projection
            .project_point3(up * height + east * 100.);
        (b - a).truncate().length()
    };
    assert!((length_at(0.) - length_at(500.)).abs() < 1e-10);
    assert_eq!(
        view.error(
            Sphere {
                center: view.origin,
                radius: 1.
            },
            10.
        ),
        view.error(
            Sphere {
                center: view.origin + up * 500.,
                radius: 1.
            },
            10.
        )
    );
    camera.set_mode(MapMode::TwoD); // Re-selecting must not overwrite saved orbit.
    camera.set_mode(MapMode::ThreeD);
    assert_eq!(
        (camera.heading, camera.pitch, camera.target, camera.distance),
        original
    );
}

#[test]
fn two_d_drag_pans_without_tilting_or_rotating() {
    use super::camera::MapMode;
    let mut camera = Camera::new(37.7749, -122.4194, 40., 1800., 35., 50.);
    camera.set_mode(MapMode::TwoD);
    let target = camera.target;
    camera.orbit(25., 20.);
    assert_ne!(camera.target, target);
    assert_eq!(camera.heading, 0.);
    assert_eq!(camera.pitch, std::f64::consts::FRAC_PI_2);
}

#[test]
fn two_d_drag_tracks_pointer_at_different_window_heights() {
    for height in [400., 1080.] {
        let mut camera = Camera::new(37.7749, -122.4194, 40., 1800., 0., 50.);
        camera.set_mode(super::camera::MapMode::TwoD);
        let old_target = camera.target;
        camera.pan_in_view(100., 0., height);
        let view = camera.view(800, height as u32);
        let new_ndc = view
            .view_projection
            .project_point3(old_target - view.origin);
        let moved_pixels = new_ndc.x * 800. / 2.;
        assert!(
            (moved_pixels - 100.).abs() < 1.,
            "{height}: moved {moved_pixels}"
        );
    }
}

#[test]
fn elevated_model_hit_bounds_follow_render_transform() {
    use super::api::{GeoPosition, Map};
    let map = Map::new(Default::default()).unwrap();
    let id = map
        .add_3d_obj(GeoPosition::new(38., -77., 80.), "robot.glb")
        .unwrap();
    map.set_object_scale(id, 12.).unwrap();
    for heading in [0., 90., 180.] {
        map.set_object_heading(id, heading).unwrap();
        let object = map.snapshot().objects[0].clone();
        let camera = Camera::new(38., -77., 0., 400., 0., 45.);
        let transform = super::objects::transform(&object);
        let (lo, hi) = camera
            .project_box(
                800,
                600,
                transform,
                DVec3::new(-1.225, -1.25, 0.),
                DVec3::new(1.225, 1.25, 2.25),
            )
            .unwrap();
        let ground = camera.project_ground(800, 600, 38., -77.).unwrap();
        assert!(
            hi.1 < ground.1,
            "The model hit target must be above the ground marker"
        );
        assert!(hi.0 > lo.0 && hi.1 > lo.1);
        let view = camera.view(800, 600);
        let center = transform.transform_point3(DVec3::new(0., 0., 1.125));
        let clip = view.view_projection * (center - view.origin).extend(1.);
        let pixel = ((clip.x / clip.w + 1.) * 400., (1. - clip.y / clip.w) * 300.);
        assert!((lo.0..=hi.0).contains(&pixel.0) && (lo.1..=hi.1).contains(&pixel.1));
    }
}

#[test]
fn measured_rover_has_35_inch_square_footprint_at_every_zoom() {
    use super::api::{GeoPosition, Map};
    let map = Map::new(Default::default()).unwrap();
    let id = map
        .add_3d_obj(GeoPosition::new(38., -77., 40.), "robot.glb")
        .unwrap();
    map.set_object_axis_scale(id, [0.889 / 2.45, 0.889 / 2.5, 0.889 / 2.5])
        .unwrap();
    let transform = super::objects::transform(&map.snapshot().objects[0]);
    assert!((transform.transform_vector3(DVec3::X * 2.45).length() - 0.889).abs() < 1e-9);
    assert!((transform.transform_vector3(DVec3::Y * 2.5).length() - 0.889).abs() < 1e-9);
    assert!((transform.transform_point3(DVec3::ZERO) - ecef(38., -77., 40.)).length() < 1e-9);
    let bounds = |distance| {
        Camera::new(38., -77., 40., distance, 0., 60.)
            .project_box(
                800,
                600,
                transform,
                DVec3::new(-1.225, -1.25, 0.),
                DVec3::new(1.225, 1.25, 2.25),
            )
            .unwrap()
    };
    let near = bounds(20.);
    let far = bounds(200.);
    assert!(near.1.0 - near.0.0 > 9. * (far.1.0 - far.0.0));
}

#[test]
fn polygon_vertices_outside_viewport_can_still_be_projected() {
    let mut camera = Camera::new(38., -77., 40., 20., 0., 90.);
    camera.set_mode(MapMode::TwoD);
    for longitude in [-77.001, -76.999] {
        assert!(camera.project_ground(800, 600, 38., longitude).is_none());
        let (x, y) = camera
            .project_ground_unclipped(800, 600, 38., longitude)
            .unwrap();
        assert!(!(0. ..=800.).contains(&x));
        assert!(y.is_finite());
    }
    assert_eq!(
        camera.project_ground(800, 600, 38., -77.),
        camera.project_ground_unclipped(800, 600, 38., -77.)
    );
    assert!(camera.project_ground_unclipped(0, 600, 38., -77.).is_none());
}

#[test]
fn raised_surface_projection_and_pointer_share_the_same_height() {
    let camera = Camera::new(38., -77., 40., 400., 0., 60.);
    let mut surface_camera = camera.clone();
    surface_camera.altitude = 140.;
    let (x, y) = surface_camera
        .project_ground_unclipped(800, 600, 38., -77.)
        .unwrap();
    let coordinate = camera
        .unproject_ground_at_altitude(800, 600, x, y, 140.)
        .unwrap();
    assert!((coordinate.0 - 38.).abs() < 1e-6);
    assert!((coordinate.1 + 77.).abs() < 1e-6);
}

#[test]
#[ignore = "Requires a native GPU; run explicitly with --ignored"]
fn rendered_tile_surface_height_tracks_transform_and_eviction() {
    let source = Url::parse("https://tile.googleapis.com/quad.glb").unwrap();
    let mut content = super::content::decode(
        include_bytes!("../../../crates/terra-map/src/maps/fixtures/textured-quad.glb"),
        &source,
        |_| unreachable!(),
    )
    .unwrap();
    content.primitives[0].transform = DMat4::IDENTITY;
    let base = ecef(0., 0., 160.);
    let world = DMat4::from_cols(
        DVec3::Y.extend(0.),
        DVec3::Z.extend(0.),
        DVec3::X.extend(0.),
        base.extend(1.),
    );
    let map = super::api::Map::new(super::MapConfig::default()).unwrap();
    let mut renderer = super::renderer::Renderer::new().unwrap();
    renderer.surface = map.surface.clone();
    renderer.upload(0, &content, world).unwrap();
    let camera = Camera::new(0., 0., 160., 20., 0., 90.);
    renderer.render(&camera.view(64, 64), &[0], 64, 64).unwrap();
    assert!((map.surface_altitude(0., 0.).unwrap() - 160.).abs() < 1e-5);
    renderer.remove(0);
    assert!(map.surface_altitude(0., 0.).is_none());
}
