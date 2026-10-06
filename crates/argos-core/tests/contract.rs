use argos_core::*;
use serde_json::json;
#[test]
fn goal_shapes_and_bounds_match_terra() {
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &GoalRequest::Local {
                x: 12.,
                y: -4.,
                yaw: None
            }
            .encode("g-1")
            .unwrap()
        )
        .unwrap(),
        json!({"frame":"local","x":12.,"y":-4.,"token":"g-1"})
    );
    assert!(
        GoalRequest::Local {
            x: 20001.,
            y: 0.,
            yaw: None
        }
        .encode("ok")
        .is_err()
    );
    assert!(
        GoalRequest::Local {
            x: f64::NAN,
            y: 0.,
            yaw: None
        }
        .encode("ok")
        .is_err()
    );
    assert!(
        GoalRequest::Geographic {
            latitude: 86.,
            longitude: 0.,
            yaw: None
        }
        .encode("ok")
        .is_err()
    );
    assert!(
        GoalRequest::Local {
            x: 0.,
            y: 0.,
            yaw: Some(7.)
        }
        .encode("ok")
        .is_err()
    );
    assert!(
        GoalRequest::Local {
            x: 0.,
            y: 0.,
            yaw: None
        }
        .encode("bad token")
        .is_err()
    );
    assert_eq!(cancel_payload(), br#"{"cancel":true}"#);
}
#[test]
fn parses_real_status_and_ignores_future_fields() {
    let status = decode_status(br#"{"state":"active","goal_id":4,"token":"g-1","distance":6.2,"x":12,"y":-4,"future":true}"#).unwrap();
    assert_eq!(status.state, GoalState::Active);
    assert_eq!(status.token.as_deref(), Some("g-1"));
    for invalid in [
        br#"{"state":"active","goal_id":0,"distance":0,"x":0,"y":0}"#.as_slice(),
        br#"{"state":"idle","goal_id":0,"distance":-1,"x":0,"y":0}"#,
        br#"{"state":"bogus","goal_id":0,"distance":0,"x":0,"y":0}"#,
    ] {
        assert!(decode_status(invalid).is_err());
    }
}
#[test]
fn extracts_body_without_decoding_pixels() {
    let mut packet = br#"{"version":1,"encoding":"32FC1_LE","rover_id":3,"sequence":9,"body":{"x":1,"y":2,"yaw":0.3}}"#.to_vec();
    packet.push(b'\n');
    packet.extend([0xff, 0x00, 0xfe]);
    let pose = decode_pose(&packet).unwrap().unwrap();
    assert_eq!((pose.rover_id, pose.x, pose.y), (3, 1., 2.));
    assert!(decode_pose(b"missing separator").is_err());
    assert!(
        decode_pose(
            br#"{"version":1,"encoding":"32FC1_LE","rover_id":3,"sequence":9}
"#
        )
        .unwrap()
        .is_none()
    );
}
#[test]
fn geographic_axes_roundtrip() {
    let anchor = Anchor {
        latitude: 38.8297,
        longitude: -77.3075,
    };
    let (lat, lon) = anchor.geographic(12., 4.).unwrap();
    assert!(lat > anchor.latitude && lon < anchor.longitude);
    let (x, y) = anchor.local(lat, lon).unwrap();
    assert!((x - 12.).abs() < 0.001 && (y - 4.).abs() < 0.001);
}
#[test]
fn validates_topics_and_fleet_membership() {
    assert_eq!(
        Topics::new("custom/rovers/").unwrap().goal(7),
        "custom/rovers/7/goal"
    );
    assert!(Topics::new("a/*").is_err());
    assert_eq!(
        decode_fleet(br#"{"count":2,"max_count":32,"ids":[0,8]}"#)
            .unwrap()
            .ids,
        vec![0, 8]
    );
    assert!(decode_fleet(br#"{"count":2,"max_count":32,"ids":[0,0]}"#).is_err());
}
