use argos_core::{
    Anchor, LocalFrame, Pose, decode_body_pose, decode_pose, decode_topic_pose,
    map_heading_degrees, rover_geographic,
};
use serde_json::json;

#[test]
fn plain_pose_takes_the_topic_id_and_heading() {
    let pose = decode_topic_pose(br#"{"x":1.5,"y":-2,"heading":0.25}"#, Some(7)).unwrap();
    assert_eq!(
        (pose.rover_id, pose.sequence, pose.x, pose.y, pose.yaw),
        (7, 0, 1.5, -2., 0.25)
    );
    let yaw_only = decode_topic_pose(br#"{"x":1,"y":2,"yaw":0.5}"#, Some(7)).unwrap();
    assert_eq!(yaw_only.yaw, 0.5);
    let yaw_wins = decode_topic_pose(br#"{"x":1,"y":2,"yaw":0.5,"heading":9}"#, Some(7)).unwrap();
    assert_eq!(yaw_wins.yaw, 0.5);
    let named = decode_body_pose(br#"{"rover_id":4,"x":1,"y":2,"yaw":0.1}"#).unwrap();
    assert_eq!((named.rover_id, named.sequence), (4, 0));
}

#[test]
fn plain_pose_rejects_disagreement_and_missing_or_bad_numbers() {
    assert!(decode_topic_pose(br#"{"rover_id":4,"x":1,"y":2,"yaw":0.1}"#, Some(9)).is_err());
    assert!(decode_topic_pose(br#"{"x":1,"y":2,"yaw":0.1}"#, None).is_err());
    assert!(decode_body_pose(br#"{"x":1,"y":2,"yaw":0.1}"#).is_err());
    assert!(decode_topic_pose(br#"{"rover_id":"4","x":1,"y":2,"yaw":0.1}"#, Some(4)).is_err());
    assert!(
        decode_topic_pose(
            br#"{"rover_id":4,"sequence":"no","x":1,"y":2,"yaw":0.1}"#,
            None
        )
        .is_err()
    );
    assert!(decode_topic_pose(br#"{"rover_id":4,"x":"east","y":2,"yaw":0.1}"#, None).is_err());
    assert!(decode_topic_pose(br#"{"rover_id":4,"x":1,"y":2}"#, None).is_err());
    assert!(decode_topic_pose(&vec![b' '; 65_537], Some(1)).is_err());
}

#[test]
fn depth_body_pose_still_decodes() {
    let mut packet = br#"{"version":1,"encoding":"32FC1_LE","rover_id":3,"sequence":9,"body":{"x":1.5,"y":-2,"yaw":0.3}}"#.to_vec();
    packet.push(b'\n');
    packet.extend([0xff, 0x00]);
    let pose = decode_pose(&packet).unwrap().unwrap();
    assert_eq!(
        (pose.rover_id, pose.sequence, pose.x, pose.y, pose.yaw),
        (3, 9, 1.5, -2., 0.3)
    );
}

#[test]
fn local_metres_match_the_geographic_anchor() {
    let origin = LocalFrame::origin(38.8297, -77.3075).unwrap();
    let anchor = Anchor {
        latitude: 38.8297,
        longitude: -77.3075,
    };
    let (lat, lon) = origin.geographic(12., 4.).unwrap();
    let (expected_lat, expected_lon) = anchor.geographic(12., 4.).unwrap();
    assert!((lat - expected_lat).abs() < 1e-9 && (lon - expected_lon).abs() < 1e-9);
    assert!(lat > anchor.latitude && lon < anchor.longitude);

    let turned = LocalFrame {
        rotation: std::f64::consts::FRAC_PI_2,
        ..origin
    };
    let (lat, lon) = turned.geographic(10., 0.).unwrap();
    let (expected_lat, expected_lon) = anchor.geographic(0., 10.).unwrap();
    assert!((lat - expected_lat).abs() < 1e-9 && (lon - expected_lon).abs() < 1e-9);
}

#[test]
fn geographic_localization_beats_the_fallback_and_local_mode_ignores_gps() {
    let fallback = LocalFrame::origin(0., 0.).unwrap();
    let pose = Pose {
        rover_id: 1,
        sequence: 0,
        x: 12.,
        y: 4.,
        yaw: 0.,
    };
    let geographic = json!({
        "version": 1,
        "mode": "geographic",
        "originLatitude": 38.8297,
        "originLongitude": -77.3075,
        "gpsLatitude": 0,
        "gpsLongitude": 0
    });
    let (lat, lon) = rover_geographic(&pose, Some(&geographic), fallback).unwrap();
    let (expected_lat, expected_lon) = Anchor {
        latitude: 38.8297,
        longitude: -77.3075,
    }
    .geographic(12., 4.)
    .unwrap();
    assert!((lat - expected_lat).abs() < 1e-9 && (lon - expected_lon).abs() < 1e-9);

    let local = json!({
        "version": 1,
        "mode": "local",
        "gpsLatitude": 38.9,
        "gpsLongitude": -77.2,
        "originLatitude": 38.8297,
        "originLongitude": -77.3075
    });
    assert!(LocalFrame::from_localization(&local).is_none());
    let (lat, lon) = rover_geographic(&pose, Some(&local), fallback).unwrap();
    let (expected_lat, expected_lon) = fallback.geographic(12., 4.).unwrap();
    assert!((lat - expected_lat).abs() < 1e-9 && (lon - expected_lon).abs() < 1e-9);
    assert!(lat.abs() < 1.);
}

#[test]
fn map_heading_is_clockwise_degrees() {
    assert!((map_heading_degrees(0.) - 0.).abs() < 1e-9);
    assert!((map_heading_degrees(std::f64::consts::FRAC_PI_2) - 270.).abs() < 1e-9);
    assert!((map_heading_degrees(-std::f64::consts::FRAC_PI_2) - 90.).abs() < 1e-9);
}
