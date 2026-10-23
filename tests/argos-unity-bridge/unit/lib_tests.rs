use super::*;
use argos_core::{Pose, Snapshot};

fn rover(id: u64, age: f64, membership: &str) -> RoverSnapshot {
    let (x, y, yaw) = rover_layout(id);
    RoverSnapshot {
        id,
        membership: membership.into(),
        pose: Some(Pose {
            rover_id: id,
            sequence: 1,
            x,
            y,
            yaw,
        }),
        pose_age: Some(age),
        goal: None,
        goal_age: None,
        goal_received_at: None,
        command_phase: "none".into(),
        command_token: None,
    }
}

#[test]
fn snapshot_json_matches_the_unity_parser_and_the_two_and_a_half_second_window() {
    let snapshot = Snapshot {
        link: "healthy".into(),
        fleet_age: Some(0.1),
        rovers: vec![
            rover(1, 0.1, "online"),
            rover(2, 0.2, "online"),
            rover(3, 2.5, "online"),
            rover(4, 2.6, "online"),
        ],
        notice: None,
    };
    let operator = serde_json::json!({
        "2": {
            "age": 0.2,
            "status": {"safety": "hold", "reason": "obstacle_blocked", "revision": 1},
            "proposal": null
        },
        "3": {
            "age": 0.2,
            "status": {"safety": "emergency_stop", "reason": "estop", "revision": 1},
            "proposal": null
        }
    });
    let parsed: serde_json::Value =
        serde_json::from_str(&fleet_json(&snapshot, &operator, 1_000.0)).unwrap();
    assert_eq!(parsed["link"], "healthy");
    assert_eq!(parsed["snapshotUnixSeconds"], 1_000.0);
    let rovers = parsed["rovers"].as_array().unwrap();
    assert_eq!(rovers.len(), 4);
    let (x, y, yaw) = rover_layout(1);
    assert_eq!(rovers[0]["id"], "terra-01");
    assert!((rovers[0]["position"]["x"].as_f64().unwrap() - x).abs() < 1e-9);
    assert_eq!(rovers[0]["position"]["y"], 0.0);
    assert!((rovers[0]["position"]["z"].as_f64().unwrap() - y).abs() < 1e-9);
    assert!((rovers[0]["headingDegrees"].as_f64().unwrap() - yaw.to_degrees()).abs() < 1e-6);
    assert_eq!(rovers[0]["status"], "normal");
    assert!((rovers[0]["lastUpdateUnixSeconds"].as_f64().unwrap() - 999.9).abs() < 1e-9);
    assert_eq!(rovers[1]["status"], "attention");
    // Exactly 2.5 s is still fresh, matching TelemetryParser.IsStale.
    assert_eq!(rovers[2]["status"], "critical");
    assert!((rovers[2]["lastUpdateUnixSeconds"].as_f64().unwrap() - 997.5).abs() < 1e-9);
    assert_eq!(rovers[3]["status"], "critical");
    assert!((1000.0 - rovers[3]["lastUpdateUnixSeconds"].as_f64().unwrap()) > STALE_SECONDS);
}

#[test]
fn gmu_anchor_round_trips_through_argos_core() {
    let anchor = argos_core::Anchor {
        latitude: 38.8297,
        longitude: -77.3075,
    };
    let (latitude, longitude) = anchor.geographic(12.0, 4.0).unwrap();
    assert!(latitude > anchor.latitude);
    assert!(longitude < anchor.longitude);
    let (x, y) = anchor.local(latitude, longitude).unwrap();
    assert!((x - 12.0).abs() < 1e-6);
    assert!((y - 4.0).abs() < 1e-6);
}

#[test]
fn map_index_omits_cells_and_the_grid_poll_keeps_them() {
    let (origin_x, origin_y) = occupancy_origin(1);
    let snapshot = serde_json::json!({
        "1": {
            "age": 0.1,
            "grid": {
                "schema_version": 1,
                "rover_id": 1,
                "run_id": "spike",
                "sequence": 4,
                "width": OCCUPANCY_SIZE,
                "height": OCCUPANCY_SIZE,
                "resolution": OCCUPANCY_RESOLUTION,
                "origin_x": origin_x,
                "origin_y": origin_y,
                "occupancy": occupancy_cells(1),
            }
        }
    })
    .to_string();
    let index: serde_json::Value = serde_json::from_str(&map_index_json(&snapshot)).unwrap();
    assert!(index["maps"][0].get("occupancy").is_none());
    assert_eq!(index["maps"][0]["id"], "terra-01");
    assert_eq!(index["maps"][0]["sequence"], 4);
    let grid: serde_json::Value =
        serde_json::from_str(&map_grid_json(&snapshot, 1).unwrap()).unwrap();
    assert_eq!(grid["occupancy"][0], -1);
    let occupied = (12 * OCCUPANCY_SIZE + 16) as usize;
    assert_eq!(grid["occupancy"][occupied], occupancy_cell(1, 16, 12));
    assert!(occupancy_cell(1, 16, 12) >= 80);
    assert!((grid["originX"].as_f64().unwrap() - origin_x).abs() < 1e-9);
    assert!(map_grid_json(&snapshot, 9).is_none());
}

#[test]
fn fake_grids_keep_the_rover_in_free_space_and_draw_walls() {
    for id in 1..=8 {
        let (origin_x, origin_y) = occupancy_origin(id);
        let (x, y, _) = rover_layout(id);
        let half = f64::from(OCCUPANCY_SIZE) * OCCUPANCY_RESOLUTION * 0.5;
        assert!((origin_x - (x - half)).abs() < 1e-9);
        assert!((origin_y - (y - half)).abs() < 1e-9);
        for (column, row) in [(11u32, 11u32), (11, 12), (12, 11), (12, 12)] {
            assert_eq!(
                occupancy_cell(id, column, row),
                0,
                "rover {id} centre cell {column},{row} should be free"
            );
        }
        assert_eq!(occupancy_cell(id, 0, 0), -1);
        let north = (0..OCCUPANCY_SIZE)
            .filter(|row| occupancy_cell(id, 16, *row) >= 80)
            .count();
        assert!(north >= 6, "north wall for rover {id} was {north} cells");
        let walls = occupancy_cells(id)
            .into_iter()
            .filter(|cell| *cell >= 80)
            .count();
        assert!(
            walls >= 20,
            "rover {id} occupied structure was {walls} cells"
        );
    }
}
