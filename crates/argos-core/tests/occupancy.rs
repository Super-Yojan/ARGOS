use argos_core::{
    CellClass, OCCUPIED_THRESHOLD, cell_center, classify_cell, decode_occupancy, local_map,
};
fn packet() -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "rover_id": 8,
        "run_id": "run-a",
        "sequence": 2,
        "width": 2,
        "height": 2,
        "resolution": 0.5,
        "origin_x": -1.0,
        "origin_y": 3.0,
        "occupancy": [-1,0,65,100],
    })
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
#[test]
fn occupied_threshold_is_65_and_cell_centers_are_local() {
    assert_eq!(OCCUPIED_THRESHOLD, 65);
    assert_eq!(classify_cell(-1), CellClass::Unknown);
    assert_eq!(classify_cell(0), CellClass::Free);
    assert_eq!(classify_cell(50), CellClass::Free);
    assert_eq!(classify_cell(64), CellClass::Free);
    assert_eq!(classify_cell(65), CellClass::Occupied);
    assert_eq!(classify_cell(100), CellClass::Occupied);
    let grid = decode_occupancy(&serde_json::to_vec(&packet()).unwrap()).unwrap();
    assert_eq!(cell_center(&grid, 0, 0), Some((-0.75, 3.25)));
    assert_eq!(cell_center(&grid, 1, 1), Some((-0.25, 3.75)));
    assert!(cell_center(&grid, 2, 0).is_none());
    let map = local_map(&grid, 1);
    assert_eq!(map.cols, 1);
    assert_eq!(map.rows, 1);
    assert_eq!(map.cells, vec![CellClass::Occupied]);
}
