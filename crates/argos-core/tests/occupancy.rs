use argos_core::decode_occupancy;
fn packet() -> serde_json::Value {
    serde_json::json!({"schema_version":1,"rover_id":8,"run_id":"run-a","sequence":2,"width":2,"height":2,"resolution":0.5,"origin_x":-1.0,"origin_y":3.0,"occupancy":[-1,0,65,100]})
}
#[test]
fn accepts_row_major_observed_cells_and_world_origin() {
    let grid = decode_occupancy(&serde_json::to_vec(&packet()).unwrap()).unwrap();
    assert_eq!(grid.occupancy, vec![-1, 0, 65, 100]);
    assert_eq!(grid.origin_y, 3.0);
}
#[test]
fn rejects_invalid_dimensions_values_and_geometry() {
    for (key, value) in [
        ("width", serde_json::json!(0)),
        ("height", serde_json::json!(500000)),
        ("resolution", serde_json::json!(-1)),
        ("occupancy", serde_json::json!([0, 0])),
        ("occupancy", serde_json::json!([-2, 0, 0, 0])),
        ("occupancy", serde_json::json!([101, 0, 0, 0])),
        ("schema_version", serde_json::json!(2)),
    ] {
        let mut p = packet();
        p[key] = value;
        assert!(
            decode_occupancy(&serde_json::to_vec(&p).unwrap()).is_err(),
            "{key}"
        );
    }
}
