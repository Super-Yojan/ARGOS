//! Collect a rover's full point capture and write it as a labelled PLY file.
//!
//! The rover sends `TPC1` frames ([`CaptureFrame`]) on `pointcloud/full` and
//! [`CaptureProgress`] on `pointcloud/capture` after the dashboard puts a
//! [`CaptureRequest`] on `pointcloud/request`. [`CaptureCollector`] keeps the
//! frames of one request; [`write_ply`] colours every point by its label.
use std::io::Write;
pub use terra_types::point_capture::{
    CaptureClassifier, CaptureFrame, CapturePoint, CaptureProgress, CaptureRequest, CaptureState,
    FRAME_TOPIC, PROGRESS_TOPIC, PointLabel, REQUEST_TOPIC,
};

/// Number of point labels; label counts are indexed by wire byte.
pub const LABELS: usize = PointLabel::ALL.len();

/// Seconds after the requested duration before the dashboard stops waiting.
pub const CAPTURE_GRACE_SECONDS: f64 = 15.0;

/// Display colour for a label, sRGB.
pub fn label_color(label: PointLabel) -> [u8; 3] {
    match label {
        PointLabel::Free => [46, 160, 67],
        PointLabel::Obstacle => [218, 54, 51],
        PointLabel::Overhead => [56, 132, 244],
        PointLabel::OutOfRange => [140, 140, 140],
        PointLabel::OutsideBand => [230, 180, 34],
        PointLabel::NoReturn => [0, 0, 0],
        PointLabel::FloorVeto => [32, 178, 170],
        PointLabel::Barrier => [255, 127, 14],
    }
}

/// Frames and progress for one request from one rover.
#[derive(Clone, Debug)]
pub struct CaptureCollector {
    /// Rover the request went to.
    pub rover_id: u64,
    /// The request as sent.
    pub request: CaptureRequest,
    /// Received frames, one per index, in arrival order.
    pub frames: Vec<CaptureFrame>,
    /// Latest progress report, if any arrived.
    pub progress: Option<CaptureProgress>,
}
impl CaptureCollector {
    pub fn new(rover_id: u64, request: CaptureRequest) -> Self {
        Self {
            rover_id,
            request,
            frames: Vec::new(),
            progress: None,
        }
    }
    /// Decode and keep a frame that belongs to this request. Returns false for
    /// a malformed frame, another rover or request, or a repeated index.
    pub fn frame(&mut self, bytes: &[u8]) -> bool {
        let Ok(frame) = CaptureFrame::decode(bytes) else {
            return false;
        };
        if frame.rover_id != self.rover_id
            || frame.request_id != self.request.request_id
            || self.frames.iter().any(|f| f.index == frame.index)
        {
            return false;
        }
        self.frames.push(frame);
        true
    }
    /// Keep a progress report that belongs to this request.
    pub fn progress(&mut self, bytes: &[u8]) -> bool {
        let Ok(progress) = serde_json::from_slice::<CaptureProgress>(bytes) else {
            return false;
        };
        if progress.rover_id != self.rover_id || progress.request_id != self.request.request_id {
            return false;
        }
        self.progress = Some(progress);
        true
    }
    /// Frames the rover said it would send, from progress or the request.
    pub fn expected(&self) -> u16 {
        self.progress
            .as_ref()
            .map(|p| p.frames_total)
            .unwrap_or_else(|| self.request.frame_count())
    }
    /// Whether nothing more is coming: every frame is in, or the rover has
    /// finished and every frame it sent is in, or `elapsed` seconds since the
    /// request passed the duration plus [`CAPTURE_GRACE_SECONDS`].
    pub fn complete(&self, elapsed: f64) -> bool {
        let received = self.frames.len();
        received >= self.expected() as usize
            || self.progress.as_ref().is_some_and(|p| {
                p.state != CaptureState::Running && received >= p.frames_sent as usize
            })
            || elapsed > self.request.duration_seconds + CAPTURE_GRACE_SECONDS
    }
    /// Count of points per label across every frame.
    pub fn label_counts(&self) -> [usize; LABELS] {
        let mut counts = [0; LABELS];
        for point in self.frames.iter().flat_map(|f| &f.points) {
            counts[point.label as usize] += 1;
        }
        counts
    }
}

/// Most points a live cloud keeps for drawing; denser frames are thinned evenly.
pub const MAX_LIVE_POINTS: usize = 12_000;

/// One live frame reduced for drawing: the points with a return, and the
/// ground height under the camera so they can stand on the map surface.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveCloud {
    pub rover_id: u64,
    pub request_id: String,
    /// Wrapping frame index from the rover.
    pub index: u16,
    /// Depth exposure time, rover monotonic seconds.
    pub timestamp: f64,
    /// Camera optical origin, world metres.
    pub camera: [f64; 3],
    /// World z of the ground under the camera: the classifier's plane there,
    /// or a low percentile of the returns before a plane is found. Heights
    /// are drawn relative to this one level, so a tilted plane stays visible.
    pub ground_z: f64,
    /// World position and label of each drawn point.
    pub points: Vec<([f32; 3], PointLabel)>,
    /// Points per label in the whole frame, in [`PointLabel`] wire order.
    pub label_counts: [usize; LABELS],
}
impl LiveCloud {
    pub fn from_frame(frame: &CaptureFrame) -> Self {
        let mut label_counts = [0; LABELS];
        for point in &frame.points {
            label_counts[point.label as usize] += 1;
        }
        let returns: Vec<_> = frame
            .points
            .iter()
            .filter(|p| p.label != PointLabel::NoReturn && p.position.iter().all(|v| v.is_finite()))
            .map(|p| (p.position, p.label))
            .collect();
        let step = returns.len().div_ceil(MAX_LIVE_POINTS).max(1);
        let points: Vec<_> = returns.into_iter().step_by(step).collect();
        let [x, y, _] = frame.camera_position;
        let plane = match frame.classifier {
            CaptureClassifier::GroundPlane { normal, offset } if normal[2].abs() > 0.1 => {
                Some((offset - normal[0] * x - normal[1] * y) / normal[2])
            }
            _ => None,
        };
        let ground_z = plane.filter(|z| z.is_finite()).unwrap_or_else(|| {
            let mut heights: Vec<f32> = points.iter().map(|(p, _)| p[2]).collect();
            heights.sort_by(f32::total_cmp);
            heights
                .get(heights.len() / 20)
                .map_or(frame.camera_position[2], |z| f64::from(*z))
        });
        Self {
            rover_id: frame.rover_id,
            request_id: frame.request_id.clone(),
            index: frame.index,
            timestamp: frame.timestamp,
            camera: frame.camera_position,
            ground_z,
            points,
            label_counts,
        }
    }
}

/// Write every point with a return as binary little-endian PLY: `x y z`
/// (float, world metres), `red green blue` from [`label_color`], `label`
/// (the wire byte) and `frame` (index within the capture). Points without a
/// return are left out. Header comments name the labels and capture settings.
pub fn write_ply(collector: &CaptureCollector, mut out: impl Write) -> std::io::Result<()> {
    let points = || {
        collector.frames.iter().flat_map(|frame| {
            frame
                .points
                .iter()
                .filter(|p| {
                    p.label != PointLabel::NoReturn && p.position.iter().all(|v| v.is_finite())
                })
                .map(move |p| (frame.index, p))
        })
    };
    let count = points().count();
    writeln!(out, "ply")?;
    writeln!(out, "format binary_little_endian 1.0")?;
    writeln!(
        out,
        "comment ARGOS point capture rover {} request {}",
        collector.rover_id, collector.request.request_id
    )?;
    writeln!(out, "comment world frame metres, +Z up")?;
    for label in PointLabel::ALL {
        let [r, g, b] = label_color(label);
        writeln!(
            out,
            "comment label {} {} rgb {r} {g} {b}",
            label as u8,
            label.name()
        )?;
    }
    if let Some(frame) = collector.frames.first() {
        writeln!(
            out,
            "comment obstacle height {:.3}..{:.3} m, max range {:.2} m, classifier {:?}",
            frame.min_obstacle_height, frame.max_obstacle_height, frame.max_range, frame.classifier
        )?;
    }
    writeln!(out, "element vertex {count}")?;
    for name in ["x", "y", "z"] {
        writeln!(out, "property float {name}")?;
    }
    for name in ["red", "green", "blue", "label"] {
        writeln!(out, "property uchar {name}")?;
    }
    writeln!(out, "property ushort frame")?;
    writeln!(out, "end_header")?;
    let mut buffer = Vec::with_capacity(count * 18);
    for (index, point) in points() {
        for axis in point.position {
            buffer.extend_from_slice(&axis.to_le_bytes());
        }
        buffer.extend_from_slice(&label_color(point.label));
        buffer.push(point.label as u8);
        buffer.extend_from_slice(&index.to_le_bytes());
    }
    out.write_all(&buffer)
}
