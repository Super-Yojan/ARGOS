use super::*;

#[test]
fn sequenced_clouds_reject_reordering_and_conflicting_revisions() {
    let original = serde_json::json!({
        "version": 1, "frameID": "frame", "sequence": 3, "points": [[1., 2., 3.]]
    });
    assert!(!accepts_cloud(&original, &original));
    let mut next = original.clone();
    next["sequence"] = 2.into();
    assert!(!accepts_cloud(&original, &next));
    next = original.clone();
    next["points"] = serde_json::json!([[4., 5., 6.]]);
    assert!(!accepts_cloud(&original, &next));
    next["sequence"] = 4.into();
    assert!(accepts_cloud(&original, &next));
    next["sequence"] = 1.into();
    next["frameID"] = "new-frame".into();
    assert!(accepts_cloud(&original, &next));
    next.as_object_mut().unwrap().remove("sequence");
    assert!(accepts_cloud(&original, &next));
}

#[test]
fn heartbeats_require_bounded_valid_topic_bound_identity() {
    let valid = br#"{"schema_version":1,"rover_id":8,"run_id":"run","sequence":3}"#;
    assert!(occupancy_identity(valid, 8).is_some());
    assert!(occupancy_identity(valid, 9).is_none());
    assert!(
        occupancy_identity(
            br#"{"schema_version":2,"rover_id":8,"run_id":"run","sequence":3}"#,
            8
        )
        .is_none()
    );
    assert!(
        occupancy_identity(
            br#"{"schema_version":1,"rover_id":8,"run_id":"","sequence":3}"#,
            8
        )
        .is_none()
    );
    assert!(
        occupancy_identity(
            br#"{"schema_version":1,"rover_id":8,"run_id":"run","sequence":-1}"#,
            8
        )
        .is_none()
    );
    assert!(occupancy_identity(&vec![b' '; 1025], 8).is_none());
    assert!(cloud_identity(valid, 8).is_none());
}

#[test]
fn only_matching_baselines_gain_freshness_without_changing_content() {
    let grid = OccupancyGrid {
        schema_version: 1,
        rover_id: 8,
        run_id: "run".into(),
        sequence: 3,
        width: 1,
        height: 1,
        resolution: 0.1,
        origin_x: 0.,
        origin_y: 0.,
        occupancy: vec![-1],
    };
    let mut cached = (grid, 1., Some("local-frame".into()));
    let wrong = Identity {
        frame: "other".into(),
        sequence: 3,
    };
    refresh_grid(&mut cached, &wrong, 9.);
    assert_eq!(cached.1, 1.);
    let old = Identity {
        frame: "run".into(),
        sequence: 2,
    };
    refresh_grid(&mut cached, &old, 9.);
    assert_eq!(cached.1, 1.);
    let matching = Identity {
        frame: "run".into(),
        sequence: 3,
    };
    refresh_grid(&mut cached, &matching, 9.);
    assert_eq!(cached.1, 9.);
    assert_eq!(cached.2.as_deref(), Some("local-frame"));
    assert_eq!(cached.0.occupancy, vec![-1]);
    assert_eq!(cached.0.sequence, 3);

    let before = Instant::now();
    let after = before + std::time::Duration::from_secs(1);
    let mut cloud = (
        serde_json::json!({
            "version": 1, "frameID": "run", "sequence": 3, "points": [[1., 2., 3.]]
        }),
        before,
    );
    let original = cloud.0.clone();
    refresh_cloud(&mut cloud, &wrong, after);
    assert_eq!(cloud.1, before);
    refresh_cloud(&mut cloud, &old, after);
    assert_eq!(cloud.1, before);
    refresh_cloud(&mut cloud, &matching, after);
    assert_eq!(cloud.1, after);
    assert_eq!(cloud.0, original);
}
