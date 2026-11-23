//! Save the current fleet state — points, occupancy maps, 3D mesh chunks, and
//! rendered 3D view — into a timestamped folder for building offline test sets.
//!
//! Destination defaults to `./snapshots/<YYYYMMDD-HHMMSS>/` and can be
//! overridden with `ARGOS_SNAPSHOT_DIR`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use image::RgbaImage;
use serde::Serialize;
use terra_comms::{Depth, MeshChunk, Occupancy, PlannedPath, Pose, Status, WorldMap};

use crate::components::mesh_renderer::OrbitCamera;
use crate::fleet::Fleet;

/// Everything needed to write one self-contained test-set snapshot to disk.
pub struct SnapshotInput {
    pub captured_at_unix_s: f64,
    pub endpoint: String,
    pub camera: CameraSnapshot,
    pub rovers: Vec<RoverData>,
    pub mesh_chunks: Vec<(u64, MeshChunk)>,
    pub view_3d: Option<RgbaImage>,
}

pub struct RoverData {
    pub id: u64,
    pub pose: Option<Pose>,
    pub status: Option<Status>,
    pub level: String,
    pub active_controller: String,
    pub goal: Option<[f64; 2]>,
    pub planned_path: Option<PlannedPath>,
    pub occupancy: Option<Occupancy>,
    pub remote_obstacles: Option<(f64, Vec<[f64; 2]>)>,
    pub depth: Option<Depth>,
    pub world_map: Option<WorldMap>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CameraSnapshot {
    pub target: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub hide_ceiling: bool,
}

impl CameraSnapshot {
    pub fn from_orbit(camera: &OrbitCamera, hide_ceiling: bool) -> Self {
        Self {
            target: camera.target,
            yaw: camera.yaw,
            pitch: camera.pitch,
            distance: camera.distance,
            hide_ceiling,
        }
    }
}

/// Summary returned to the UI after writing a snapshot.
#[derive(Clone, Debug)]
pub struct SavedSnapshot {
    pub dir: PathBuf,
    pub rover_count: usize,
    pub occupancy_files: usize,
    pub point_files: usize,
    pub mesh_chunks: usize,
    pub saved_view_3d: bool,
}

#[derive(Serialize)]
struct Manifest {
    captured_at_unix_s: f64,
    endpoint: String,
    camera: CameraSnapshot,
    rovers: Vec<RoverManifest>,
    files: ManifestFiles,
}

#[derive(Serialize)]
struct RoverManifest {
    id: u64,
    pose: Option<Pose>,
    status: Option<Status>,
    level: String,
    active_controller: String,
    goal: Option<[f64; 2]>,
    planned_path: Option<PlannedPath>,
    occupancy_cells: usize,
    depth_points: usize,
    world_map_points: usize,
    mesh_chunks: usize,
    mesh_vertices: usize,
}

#[derive(Serialize)]
struct ManifestFiles {
    occupancy: Vec<String>,
    points: Vec<String>,
    mesh_json: Option<String>,
    mesh_obj: Option<String>,
    view_3d_png: Option<String>,
}

#[derive(Serialize)]
struct OccupancyFile {
    rover_id: u64,
    source: &'static str,
    t: f64,
    resolution: f64,
    cells: Vec<[f32; 2]>,
}

#[derive(Serialize)]
struct PointsFile {
    rover_id: u64,
    pose: Option<Pose>,
    depth_t: Option<f64>,
    depth_sequence: Option<u64>,
    depth_points: Vec<[f32; 3]>,
    world_map_t: Option<f64>,
    world_map_sequence: Option<u64>,
    world_map_points: Vec<[f32; 3]>,
    mesh_vertices: Vec<[f32; 3]>,
    occupancy_cells: Vec<[f32; 2]>,
}

#[derive(Serialize)]
struct MeshChunkFile<'a> {
    rover_id: u64,
    id: &'a str,
    t: f64,
    vertices: &'a [[f32; 3]],
    indices: &'a [[u32; 3]],
    classes: &'a [u8],
    colors: &'a [[u8; 3]],
}

/// Root folder for snapshots: `ARGOS_SNAPSHOT_DIR` if set, otherwise `./snapshots`.
pub fn default_root() -> PathBuf {
    std::env::var("ARGOS_SNAPSHOT_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("snapshots"))
}

/// Collect the current fleet + 3D scene state into a [`SnapshotInput`].
pub fn collect(
    fleet: &Fleet,
    chunks: &BTreeMap<(u64, String), MeshChunk>,
    camera: &OrbitCamera,
    hide_ceiling: bool,
    view_3d: Option<RgbaImage>,
) -> SnapshotInput {
    let captured_at_unix_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);

    let rovers = fleet
        .rovers()
        .into_iter()
        .map(|rover| {
            let id = rover.id;
            RoverData {
                id,
                pose: rover.pose,
                status: fleet.status(id),
                level: rover.level.label(),
                active_controller: rover.active_controller,
                goal: rover.goal,
                planned_path: fleet.planned_path(id),
                occupancy: fleet.occupancy(id),
                remote_obstacles: fleet
                    .remote_state(id)
                    .map(|(state, _)| (state.t, state.obstacles)),
                depth: fleet.depth(id),
                world_map: fleet.world_map(id),
            }
        })
        .collect();

    let mesh_chunks = chunks
        .iter()
        .map(|((rover_id, _), chunk)| (*rover_id, chunk.clone()))
        .collect();

    SnapshotInput {
        captured_at_unix_s,
        endpoint: fleet.endpoint().to_string(),
        camera: CameraSnapshot::from_orbit(camera, hide_ceiling),
        rovers,
        mesh_chunks,
        view_3d,
    }
}

/// Write a snapshot into a fresh timestamped folder under `root`.
pub fn write_to_root(root: &Path, input: &SnapshotInput) -> Result<SavedSnapshot> {
    let dir = unique_snapshot_dir(root, input.captured_at_unix_s as u64);
    fs::create_dir_all(&dir)
        .with_context(|| format!("create snapshot directory {}", dir.display()))?;

    // Index mesh chunks per rover so we can include per-rover vertices in points files.
    let mut chunks_by_rover: BTreeMap<u64, Vec<&MeshChunk>> = BTreeMap::new();
    for (rover_id, chunk) in &input.mesh_chunks {
        chunks_by_rover.entry(*rover_id).or_default().push(chunk);
    }

    let mut occupancy_files = Vec::new();
    let mut point_files = Vec::new();
    let mut rover_manifests = Vec::new();

    for rover in &input.rovers {
        let rover_chunks = chunks_by_rover
            .get(&rover.id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let mesh_vertices: Vec<[f32; 3]> = rover_chunks
            .iter()
            .flat_map(|chunk| chunk.vertices.iter().copied())
            .collect();

        let (occ_file, occ_cells) = if let Some(occ) = &rover.occupancy {
            (
                Some(OccupancyFile {
                    rover_id: rover.id,
                    source: "occupancy",
                    t: occ.t,
                    resolution: occ.resolution,
                    cells: occ.cells.clone(),
                }),
                occ.cells.clone(),
            )
        } else if let Some((t, obstacles)) = &rover.remote_obstacles {
            if obstacles.is_empty() {
                (None, Vec::new())
            } else {
                let cells: Vec<[f32; 2]> = obstacles
                    .iter()
                    .map(|pt| [pt[0] as f32, pt[1] as f32])
                    .collect();
                (
                    Some(OccupancyFile {
                        rover_id: rover.id,
                        source: "remote_state",
                        t: *t,
                        resolution: 0.1,
                        cells: cells.clone(),
                    }),
                    cells,
                )
            }
        } else {
            (None, Vec::new())
        };

        if let Some(occ) = occ_file {
            let filename = format!("rover_{}_occupancy.json", rover.id);
            write_json(&dir.join(&filename), &occ)?;
            occupancy_files.push(filename);
        }

        let depth_points = rover
            .depth
            .as_ref()
            .map(|d| d.points.clone())
            .unwrap_or_default();
        let world_map_points = rover
            .world_map
            .as_ref()
            .map(|w| w.points.clone())
            .unwrap_or_default();

        if !depth_points.is_empty()
            || !world_map_points.is_empty()
            || !mesh_vertices.is_empty()
            || !occ_cells.is_empty()
        {
            let filename = format!("rover_{}_points.json", rover.id);
            let points = PointsFile {
                rover_id: rover.id,
                pose: rover.pose.clone(),
                depth_t: rover.depth.as_ref().map(|d| d.t),
                depth_sequence: rover.depth.as_ref().map(|d| d.sequence),
                depth_points: depth_points.clone(),
                world_map_t: rover.world_map.as_ref().map(|w| w.t),
                world_map_sequence: rover.world_map.as_ref().map(|w| w.sequence),
                world_map_points: world_map_points.clone(),
                mesh_vertices: mesh_vertices.clone(),
                occupancy_cells: occ_cells.clone(),
            };
            write_json(&dir.join(&filename), &points)?;
            point_files.push(filename);
        }

        rover_manifests.push(RoverManifest {
            id: rover.id,
            pose: rover.pose.clone(),
            status: rover.status.clone(),
            level: rover.level.clone(),
            active_controller: rover.active_controller.clone(),
            goal: rover.goal,
            planned_path: rover.planned_path.clone(),
            occupancy_cells: occ_cells.len(),
            depth_points: depth_points.len(),
            world_map_points: world_map_points.len(),
            mesh_chunks: rover_chunks.len(),
            mesh_vertices: mesh_vertices.len(),
        });
    }

    let (mesh_json, mesh_obj) = if input.mesh_chunks.is_empty() {
        (None, None)
    } else {
        let json_name = "mesh_3d.json".to_string();
        let entries: Vec<MeshChunkFile<'_>> = input
            .mesh_chunks
            .iter()
            .map(|(rover_id, chunk)| MeshChunkFile {
                rover_id: *rover_id,
                id: &chunk.id,
                t: chunk.t,
                vertices: &chunk.vertices,
                indices: &chunk.indices,
                classes: &chunk.classes,
                colors: &chunk.colors,
            })
            .collect();
        write_json(&dir.join(&json_name), &entries)?;

        let obj_name = "mesh_3d.obj".to_string();
        let obj = encode_obj(&input.mesh_chunks);
        fs::write(dir.join(&obj_name), obj)
            .with_context(|| format!("write {}", dir.join(&obj_name).display()))?;

        (Some(json_name), Some(obj_name))
    };

    let view_3d_png = if let Some(img) = &input.view_3d {
        let png_name = "view_3d.png".to_string();
        img.save(dir.join(&png_name))
            .with_context(|| format!("save {}", dir.join(&png_name).display()))?;
        Some(png_name)
    } else {
        None
    };

    let saved_view_3d = view_3d_png.is_some();
    let manifest = Manifest {
        captured_at_unix_s: input.captured_at_unix_s,
        endpoint: input.endpoint.clone(),
        camera: input.camera.clone(),
        rovers: rover_manifests,
        files: ManifestFiles {
            occupancy: occupancy_files.clone(),
            points: point_files.clone(),
            mesh_json,
            mesh_obj,
            view_3d_png,
        },
    };
    write_json(&dir.join("snapshot.json"), &manifest)?;

    Ok(SavedSnapshot {
        dir,
        rover_count: input.rovers.len(),
        occupancy_files: occupancy_files.len(),
        point_files: point_files.len(),
        mesh_chunks: input.mesh_chunks.len(),
        saved_view_3d,
    })
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .with_context(|| format!("serialize {}", path.display()))?;
    fs::write(path, bytes).with_context(|| format!("write {}", path.display()))
}

/// Encode all 3D mesh chunks as a Wavefront OBJ string (1-based vertex indices).
fn encode_obj(chunks: &[(u64, MeshChunk)]) -> String {
    let mut out = String::from("# ARGOS 3D mesh snapshot (Z-up, metres)\n");
    let mut vertex_offset: usize = 1;
    for (rover_id, chunk) in chunks {
        let _ = writeln!(out, "o rover_{}_{}", rover_id, chunk.id);
        let has_colors = chunk.colors.len() == chunk.vertices.len();
        for (i, v) in chunk.vertices.iter().enumerate() {
            if has_colors {
                let c = chunk.colors[i];
                let _ = writeln!(
                    out,
                    "v {} {} {} {:.4} {:.4} {:.4}",
                    v[0],
                    v[1],
                    v[2],
                    c[0] as f32 / 255.0,
                    c[1] as f32 / 255.0,
                    c[2] as f32 / 255.0,
                );
            } else {
                let _ = writeln!(out, "v {} {} {}", v[0], v[1], v[2]);
            }
        }
        let n = chunk.vertices.len();
        for tri in &chunk.indices {
            let [a, b, c] = *tri;
            if (a as usize) < n && (b as usize) < n && (c as usize) < n {
                let _ = writeln!(
                    out,
                    "f {} {} {}",
                    vertex_offset + a as usize,
                    vertex_offset + b as usize,
                    vertex_offset + c as usize,
                );
            }
        }
        vertex_offset += n;
    }
    out
}

fn unique_snapshot_dir(root: &Path, unix_secs: u64) -> PathBuf {
    let base_name = format_utc_stamp(unix_secs);
    let first = root.join(&base_name);
    if !first.exists() {
        return first;
    }
    for suffix in 1u32.. {
        let candidate = root.join(format!("{base_name}-{suffix}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("u32 suffix space exhausted")
}

/// Format a UNIX timestamp (seconds) as `YYYYMMDD-HHMMSS` in UTC using
/// Howard Hinnant's civil-from-days algorithm (no external date crate needed).
fn format_utc_stamp(unix_secs: u64) -> String {
    let secs_of_day = unix_secs % 86_400;
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;

    let z = (unix_secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };

    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn utc_stamp_formats_known_epoch_dates() {
        assert_eq!(format_utc_stamp(0), "19700101-000000");
        assert_eq!(format_utc_stamp(1_700_000_000), "20231114-221320");
    }

    #[test]
    fn write_snapshot_creates_manifest_occupancy_points_mesh_and_png() {
        let tmp = std::env::temp_dir().join(format!(
            "argos-snapshot-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);

        let mut img = RgbaImage::new(2, 2);
        img.put_pixel(0, 0, Rgba([255, 0, 0, 255]));

        let input = SnapshotInput {
            captured_at_unix_s: 1_700_000_000.0,
            endpoint: "tcp/127.0.0.1:7447".into(),
            camera: CameraSnapshot {
                target: [0.0, 0.0, 0.0],
                yaw: -0.7,
                pitch: 0.55,
                distance: 5.5,
                hide_ceiling: true,
            },
            rovers: vec![RoverData {
                id: 7,
                pose: Some(Pose {
                    t: 1.0,
                    x: 1.2,
                    y: -0.5,
                    yaw: 0.3,
                    tracked: true,
                }),
                status: None,
                level: "L3 Planned".into(),
                active_controller: "dwa".into(),
                goal: Some([3.0, 2.0]),
                planned_path: None,
                occupancy: Some(Occupancy {
                    t: 1.0,
                    resolution: 0.1,
                    cells: vec![[1.0, 2.0], [1.1, 2.0]],
                    free: vec![[0.5, 0.5]],
                }),
                remote_obstacles: None,
                depth: Some(Depth {
                    t: 1.0,
                    sequence: 4,
                    points: vec![[0.5, 0.1, 1.2]],
                }),
                world_map: Some(WorldMap {
                    t: 1.0,
                    sequence: 2,
                    points: vec![[2.0, 3.0, 0.4]],
                    data: vec![],
                }),
            }],
            mesh_chunks: vec![
                (
                    7,
                    MeshChunk {
                        t: 1.0,
                        id: "c1".into(),
                        vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                        indices: vec![[0, 1, 2]],
                        classes: vec![2],
                        colors: vec![[255, 0, 0], [0, 255, 0], [0, 0, 255]],
                    },
                ),
                (
                    7,
                    MeshChunk {
                        t: 1.0,
                        id: "c2".into(),
                        vertices: vec![[0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0]],
                        indices: vec![[0, 1, 2]],
                        classes: vec![3],
                        colors: vec![],
                    },
                ),
            ],
            view_3d: Some(img),
        };

        let first = write_to_root(&tmp, &input).expect("first snapshot");
        assert_eq!(first.rover_count, 1);
        assert_eq!(first.occupancy_files, 1);
        assert_eq!(first.point_files, 1);
        assert_eq!(first.mesh_chunks, 2);
        assert!(first.saved_view_3d);

        assert!(first.dir.join("snapshot.json").is_file());
        assert!(first.dir.join("rover_7_occupancy.json").is_file());
        assert!(first.dir.join("rover_7_points.json").is_file());
        assert!(first.dir.join("mesh_3d.json").is_file());
        assert!(first.dir.join("view_3d.png").is_file());

        let obj = fs::read_to_string(first.dir.join("mesh_3d.obj")).unwrap();
        assert!(obj.contains("o rover_7_c1"));
        assert!(obj.contains("f 1 2 3"));
        assert!(obj.contains("o rover_7_c2"));
        assert!(obj.contains("f 4 5 6"));

        // Second snapshot in the same second gets a unique suffixed directory.
        let second = write_to_root(&tmp, &input).expect("second snapshot");
        assert_ne!(first.dir, second.dir);

        let _ = fs::remove_dir_all(&tmp);
    }
}
