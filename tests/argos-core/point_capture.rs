use argos_core::point_capture::*;
use terra_types::point_capture::{CaptureClassifier, CapturePoint};

fn frame(index: u16, request: &str) -> Vec<u8> {
    CaptureFrame {
        rover_id: 7,
        request_id: request.into(),
        index,
        count: 2,
        timestamp: index as f64,
        width: 3,
        height: 1,
        camera_position: [0.0, 0.0, 0.5],
        camera_orientation: [0.0, 0.0, 0.0, 1.0],
        intrinsics: [1.0, 1.0, 1.0, 0.0],
        classifier: CaptureClassifier::GroundPlane {
            normal: [0.0, 0.0, 1.0],
            offset: 0.0,
        },
        min_obstacle_height: 0.15,
        max_obstacle_height: 2.0,
        max_range: 15.0,
        points: vec![
            CapturePoint {
                position: [1.0, 0.0, 0.0],
                label: PointLabel::Free,
            },
            CapturePoint {
                position: [2.0, 0.0, 0.5],
                label: PointLabel::Obstacle,
            },
            CapturePoint {
                position: [f32::NAN; 3],
                label: PointLabel::NoReturn,
            },
        ],
    }
    .encode()
    .unwrap()
}

fn request() -> CaptureRequest {
    CaptureRequest {
        duration_seconds: 2.0,
        ..CaptureRequest::new("cap")
    }
}

/// Frames for another request, repeats and garbage are ignored; the capture
/// is complete once every expected frame is in.
#[test]
fn collector_keeps_matching_frames_once() {
    let mut c = CaptureCollector::new(7, request());
    assert_eq!(c.expected(), 2);
    assert!(c.frame(&frame(0, "cap")));
    assert!(!c.frame(&frame(0, "cap")));
    assert!(!c.frame(&frame(1, "other")));
    assert!(!c.frame(b"junk"));
    assert!(!c.complete(1.0));
    assert!(c.frame(&frame(1, "cap")));
    assert!(c.complete(1.0));
    assert_eq!(c.label_counts()[PointLabel::Obstacle as usize], 2);
}

/// A finished rover that sent fewer frames, or a timeout, also completes it.
#[test]
fn collector_completes_on_done_or_timeout() {
    let mut c = CaptureCollector::new(7, request());
    assert!(c.frame(&frame(0, "cap")));
    let done = serde_json::json!({"version":1,"rover_id":7,"request_id":"cap","state":"done",
        "frames_sent":1,"frames_total":2,"detail":"no_depth"});
    assert!(c.progress(done.to_string().as_bytes()));
    assert!(c.complete(0.5));
    let c = CaptureCollector::new(7, request());
    assert!(!c.complete(16.0));
    assert!(c.complete(17.1));
}

/// The PLY header declares exactly the points with a return, and every vertex
/// carries its label colour.
#[test]
fn ply_lists_labelled_points() {
    let mut c = CaptureCollector::new(7, request());
    c.frame(&frame(0, "cap"));
    c.frame(&frame(1, "cap"));
    let mut bytes = Vec::new();
    write_ply(&c, &mut bytes).unwrap();
    let end = bytes
        .windows(11)
        .position(|w| w == b"end_header\n")
        .unwrap()
        + 11;
    let header = std::str::from_utf8(&bytes[..end]).unwrap();
    assert!(header.contains("element vertex 4\n"));
    assert!(header.contains("comment label 1 obstacle rgb 218 54 51"));
    let body = &bytes[end..];
    assert_eq!(body.len(), 4 * 18);
    assert_eq!(f32::from_le_bytes(body[18..22].try_into().unwrap()), 2.0);
    assert_eq!(&body[18 + 12..18 + 16], &[218, 54, 51, 1]);
    assert_eq!(
        u16::from_le_bytes(body[18 * 3 + 16..18 * 4].try_into().unwrap()),
        1
    );
}
