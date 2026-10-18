//! A heartbeat confirms a freshly observed, unchanged sensor sample. It can
//! refresh an existing matching baseline, never create content or authorize control.

use argos_core::OccupancyGrid;
use serde_json::Value;
use std::time::Instant;

pub(super) struct Identity {
    frame: String,
    sequence: u64,
}

fn identity(bytes: &[u8], id: u64, version: &str, frame: &str) -> Option<Identity> {
    if bytes.len() > 1024 {
        return None;
    }
    let value: Value = serde_json::from_slice(bytes).ok()?;
    if value[version].as_u64() != Some(1) || value["rover_id"].as_u64() != Some(id) {
        return None;
    }
    let frame = value[frame].as_str()?;
    if frame.is_empty() || frame.len() > 256 {
        return None;
    }
    Some(Identity {
        frame: frame.into(),
        sequence: value["sequence"].as_u64()?,
    })
}

pub(super) fn occupancy_identity(bytes: &[u8], id: u64) -> Option<Identity> {
    identity(bytes, id, "schema_version", "run_id")
}

pub(super) fn cloud_identity(bytes: &[u8], id: u64) -> Option<Identity> {
    identity(bytes, id, "version", "frameID")
}

/// Sequenced clouds are immutable within a frame. Older/conflicting samples
/// cannot replace the baseline that subsequent heartbeats reference.
pub(super) fn accepts_cloud(previous: &Value, next: &Value) -> bool {
    let sequenced = |value: &Value| {
        value["version"].as_u64() == Some(1)
            && value["frameID"]
                .as_str()
                .is_some_and(|frame| !frame.is_empty())
            && value["sequence"].as_u64().is_some()
    };
    if !sequenced(previous) || !sequenced(next) || previous["frameID"] != next["frameID"] {
        // Preserve legacy unsequenced publishers and permit a new coordinate frame.
        return true;
    }
    let old = previous["sequence"].as_u64().unwrap();
    let new = next["sequence"].as_u64().unwrap();
    new > old
}

pub(super) fn refresh_grid(
    cached: &mut (OccupancyGrid, f64, Option<String>),
    heartbeat: &Identity,
    now: f64,
) {
    if cached.0.run_id == heartbeat.frame && cached.0.sequence == heartbeat.sequence {
        cached.1 = now;
    }
}

pub(super) fn refresh_cloud(cached: &mut (Value, Instant), heartbeat: &Identity, now: Instant) {
    if cached.0["version"].as_u64() == Some(1)
        && cached.0["frameID"].as_str() == Some(heartbeat.frame.as_str())
        && cached.0["sequence"].as_u64() == Some(heartbeat.sequence)
    {
        cached.1 = now;
    }
}

#[cfg(test)]
mod tests {
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
}
