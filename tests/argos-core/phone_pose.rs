use argos_core::decode_phone_pose;
#[test]
fn lightweight_phone_pose_is_validated() {
    let p = decode_phone_pose(br#"{"rover_id":7,"sequence":4,"x":2,"y":-3,"yaw":0.5}"#).unwrap();
    assert_eq!(p.rover_id, 7);
    assert_eq!(p.sequence, 4);
    assert_eq!(p.y, -3.);
    for invalid in [
        br#"{"rover_id":7,"sequence":4,"x":null,"y":0,"yaw":0}"#.as_slice(),
        br#"{"rover_id":7,"x":0,"y":0,"yaw":0}"#,
        br#"{"rover_id":-1,"sequence":4,"x":0,"y":0,"yaw":0}"#,
    ] {
        assert!(decode_phone_pose(invalid).is_err());
    }
    assert!(decode_phone_pose(&vec![b' '; 65_537]).is_err());
}
