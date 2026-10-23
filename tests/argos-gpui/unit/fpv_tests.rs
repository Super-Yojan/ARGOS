use super::config::FpvConfig;
use super::decode::VideoDecoder;
use super::depacketize::{H264Depacketizer, Vp8Depacketizer, timestamp_is_late};
use super::pattern::{TEST_PATTERN_HEIGHT, TEST_PATTERN_WIDTH, test_pattern_yuv};
use fpv_signaling::{Effect, OFFER_TIMEOUT, SignalingSession, Topic};
use openh264::formats::YUVSource;

use std::collections::BTreeMap;

#[test]
fn dashboard_machine_times_out_with_a_stop_request() {
    let mut session = SignalingSession::dashboard();
    let id = "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11";
    let started = session.start(id, 0);
    assert!(started.iter().any(|effect| matches!(
        effect,
        Effect::Publish { topic: Topic::Request, payload }
            if payload.contains("\"v\":1") && payload.contains("\"start\"")
    )));
    assert!(
        session
            .tick(OFFER_TIMEOUT.as_millis() as u64 - 1)
            .is_empty()
    );
    let timed = session.tick(OFFER_TIMEOUT.as_millis() as u64);
    assert!(timed.iter().any(|effect| matches!(
        effect,
        Effect::Publish { topic: Topic::Request, payload } if payload.contains("\"stop\"")
    )));
    assert!(timed.iter().all(|effect| match effect {
        Effect::Publish { topic, .. } => topic.is_dashboard_outbound(),
        _ => true,
    }));
}

#[test]
fn h264_depacketizer_assembles_single_stap_and_fu_a_and_drops_late_frames() {
    let mut packets = H264Depacketizer::default();
    let single = packets.push(&[0x65, 0x11]);
    assert_eq!(single, vec![vec![0x65, 0x11]]);

    let stap = packets.push(&[0x18, 0x00, 0x02, 0x67, 0x42, 0x00, 0x01, 0x68]);
    assert_eq!(stap, vec![vec![0x67, 0x42], vec![0x68]]);

    assert!(packets.push(&[0x7c, 0x85, 0x11, 0x22]).is_empty());
    let nal = packets.push(&[0x7c, 0x45, 0x33]);
    assert_eq!(nal, vec![vec![0x65, 0x11, 0x22, 0x33]]);

    assert!(!timestamp_is_late(1, None));
    assert!(timestamp_is_late(100, Some(100)));
    assert!(timestamp_is_late(99, Some(100)));
    assert!(!timestamp_is_late(101, Some(100)));
    assert!(!timestamp_is_late(50, Some(u32::MAX - 10)));
}

#[test]
fn vp8_descriptor_strips_extension_and_a_new_partition_drops_a_partial_frame() {
    let mut packets = Vp8Depacketizer::default();
    assert!(packets.push(&[0x90, 0x80, 0x00, 0xaa], false).is_none());
    let frame = packets.push(&[0x00, 0xbb], true).unwrap();
    assert_eq!(frame, vec![0xaa, 0xbb]);
    assert!(packets.push(&[0x00, 0x01], false).is_none());
    let replaced = packets.push(&[0x10, 0x02], true).unwrap();
    assert_eq!(replaced, vec![0x02]);
}

#[test]
fn ice_config_defaults_to_public_stun_and_rejects_partial_turn() {
    let values = BTreeMap::new();
    let config = FpvConfig::from_values(&values).unwrap();
    assert_eq!(config.stun_urls, vec!["stun:stun.l.google.com:19302"]);
    assert!(config.turn.is_none());

    let mut none = BTreeMap::new();
    none.insert("ARGOS_FPV_STUN".into(), "none".into());
    assert!(FpvConfig::from_values(&none).unwrap().stun_urls.is_empty());

    let mut turn = BTreeMap::new();
    turn.insert("ARGOS_FPV_TURN_URL".into(), "turn:example.com:3478".into());
    turn.insert("ARGOS_FPV_TURN_USERNAME".into(), "operator".into());
    assert!(FpvConfig::from_values(&turn).is_err());
    turn.insert("ARGOS_FPV_TURN_CREDENTIAL".into(), "secret".into());
    let parsed = FpvConfig::from_values(&turn).unwrap();
    let server = parsed.turn.unwrap();
    assert_eq!(server.username, "operator");
    assert_eq!(server.credential, "secret");
    assert!(!format!("{server:?}").contains("secret"));

    let mut embedded = BTreeMap::new();
    embedded.insert(
        "ARGOS_FPV_TURN_URL".into(),
        "turn:user:secret@example.com:3478".into(),
    );
    embedded.insert("ARGOS_FPV_TURN_USERNAME".into(), "operator".into());
    embedded.insert("ARGOS_FPV_TURN_CREDENTIAL".into(), "secret".into());
    assert!(FpvConfig::from_values(&embedded).is_err());
}

#[test]
fn generated_pattern_decodes_as_h264() {
    let yuv = test_pattern_yuv(0);
    let (width, height) = yuv.dimensions();
    assert_eq!(
        (TEST_PATTERN_WIDTH, TEST_PATTERN_HEIGHT),
        (width as u32, height as u32)
    );
    let config = openh264::encoder::EncoderConfig::new()
        .bitrate(openh264::encoder::BitRate::from_bps(800_000))
        .max_frame_rate(openh264::encoder::FrameRate::from_hz(15.0))
        .intra_frame_period(openh264::encoder::IntraFramePeriod::from_num_frames(1))
        .profile(openh264::encoder::Profile::Baseline);
    let mut encoder =
        openh264::encoder::Encoder::with_api_config(openh264::OpenH264API::from_source(), config)
            .unwrap();
    let mut decoder = VideoDecoder::new().unwrap();
    decoder.set_mime("video/H264");
    let mut picture = None;
    for frame in 0..4 {
        encoder.force_intra_frame();
        let encoded = encoder.encode(&test_pattern_yuv(frame)).unwrap();
        let mut timestamp = frame.saturating_add(1).saturating_mul(6_000);
        let mut units = Vec::new();
        for layer in 0..encoded.num_layers() {
            let layer = encoded.layer(layer).unwrap();
            for index in 0..layer.nal_count() {
                units.push(strip_start_code(layer.nal_unit(index).unwrap()).to_vec());
            }
        }
        let last = units.len().saturating_sub(1);
        for (index, unit) in units.iter().enumerate() {
            if unit.is_empty() {
                continue;
            }
            if let Ok(Some(decoded)) = decoder.push(timestamp, index == last, unit) {
                picture = Some(decoded);
            }
            timestamp = timestamp.saturating_add(1);
        }
    }
    let picture = picture.expect("H.264 test pattern did not decode");
    assert!(picture.width >= 16 && picture.height >= 16);
    assert_eq!(
        picture.bgra.len(),
        picture.width as usize * picture.height as usize * 4
    );
}

fn strip_start_code(nal: &[u8]) -> &[u8] {
    if nal.starts_with(&[0, 0, 0, 1]) {
        &nal[4..]
    } else if nal.starts_with(&[0, 0, 1]) {
        &nal[3..]
    } else {
        nal
    }
}

#[test]
fn local_sender_delivers_a_frame() {
    use super::{FpvController, FpvPhase, SenderOptions, run_test_sender};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let router = argos_zenoh::router::Router::default();
    router
        .start(port, "")
        .unwrap_or_else(|error| panic!("router: {error}"));
    let endpoint = format!("tcp/127.0.0.1:{port}");
    let mut config = crate::DashboardConfig::parse(
        Some(&endpoint),
        Some("terra/phone"),
        Some(""),
        None,
        (None, None),
        (None, None),
    )
    .unwrap();
    config.fpv = FpvConfig::host_only();
    let viewer = FpvController::start(config);
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_sender = cancel.clone();
    let endpoint_sender = endpoint.clone();
    let sender = std::thread::Builder::new()
        .name("fpv-e2e-sender".into())
        .spawn(move || {
            run_test_sender(SenderOptions {
                endpoint: endpoint_sender,
                prefix: "terra/phone".into(),
                phone_id: 7,
                ice: FpvConfig::host_only(),
                announce: false,
                user: None,
                password: None,
                cancel: cancel_sender,
            })
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    viewer.start_phone(7);
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut picture = viewer.view();
    while Instant::now() < deadline {
        picture = viewer.view();
        if picture.frame_seq > 0 && picture.width > 0 {
            break;
        }
        if picture.phase == FpvPhase::Failed {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let detail = format!("{} {}", picture.phase.label(), picture.detail);
    viewer.stop();
    cancel.store(true, Ordering::Relaxed);
    let sender_result = sender.join().expect("sender thread");
    let _ = router.stop();
    sender_result.expect("sender");
    assert!(
        picture.frame_seq > 0 && picture.width > 0,
        "no video frame arrived: {detail}"
    );
}
