//! Ask one rover for a full labelled point capture and save it as PLY.
//!
//! [`Client::start_point_capture`] subscribes to that rover's
//! `pointcloud/full` and `pointcloud/capture`, then puts the request. The
//! subscriptions exist only while a capture runs: [`Client::point_capture`]
//! drops them once the capture is complete and writes the PLY file off the
//! caller's thread.
use crate::{Client, Error, Result, invalid};
use argos_core::point_capture::*;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use zenoh::Wait;

/// One capture's collected frames and its live subscriptions.
pub(crate) struct Capture {
    collector: Arc<Mutex<CaptureCollector>>,
    started: Instant,
    subscribers: Vec<zenoh::pubsub::Subscriber<()>>,
    path: PathBuf,
    saved: Arc<Mutex<Option<std::result::Result<PathBuf, String>>>>,
}
pub(crate) type Captures = Mutex<std::collections::BTreeMap<u64, Capture>>;

/// What the dashboard shows about a rover's latest capture.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureStatus {
    pub request_id: String,
    /// Frames received so far.
    pub received: usize,
    /// Frames the rover intends to send.
    pub expected: u16,
    /// The rover's latest reported state, if any report arrived.
    pub state: Option<CaptureState>,
    /// The rover's reason for missing frames, such as `no_depth`.
    pub detail: String,
    /// Nothing more is coming; subscriptions are closed.
    pub finished: bool,
    /// The saved PLY path, or why saving failed; `None` while writing.
    pub saved: Option<std::result::Result<PathBuf, String>>,
    /// Points per label, in [`PointLabel`] wire order.
    pub label_counts: [usize; 6],
}

impl Client {
    /// Ask rover `id` for a default capture (10 s at 1 Hz) and collect it.
    /// The PLY file goes to `directory`. Starting again replaces an unfinished
    /// capture for the same rover. Returns the request id.
    pub fn start_point_capture(&self, id: u64, directory: PathBuf) -> Result<String> {
        let request_id = format!(
            "argos-{}",
            uuid::Uuid::new_v4()
                .simple()
                .to_string()
                .get(..12)
                .unwrap_or("capture")
        );
        let request = CaptureRequest::new(request_id.clone());
        let collector = Arc::new(Mutex::new(CaptureCollector::new(id, request.clone())));
        let base = format!("{}/{id}", self.topics.prefix);
        let session = self.session.lock().unwrap();
        let session = session.as_ref().ok_or_else(|| invalid("disconnected"))?;
        let frames = collector.clone();
        let full = session
            .declare_subscriber(format!("{base}/{FRAME_TOPIC}"))
            .callback(move |sample| {
                frames.lock().unwrap().frame(&sample.payload().to_bytes());
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let reports = collector.clone();
        let progress = session
            .declare_subscriber(format!("{base}/{PROGRESS_TOPIC}"))
            .callback(move |sample| {
                reports
                    .lock()
                    .unwrap()
                    .progress(&sample.payload().to_bytes());
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        session
            .put(
                format!("{base}/{REQUEST_TOPIC}"),
                serde_json::to_vec(&request).map_err(|e| Error(e.to_string()))?,
            )
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let _ = self.recorder.record(serde_json::json!({
            "kind": "point_capture_requested",
            "rover_id": id,
            "time": self.start.elapsed().as_secs_f64(),
            "session_id": self.session_id,
            "payload": request,
        }));
        self.captures.lock().unwrap().insert(
            id,
            Capture {
                collector,
                started: Instant::now(),
                subscribers: vec![full, progress],
                path: directory.join(format!("rover-{id}-{request_id}.ply")),
                saved: Default::default(),
            },
        );
        Ok(request_id)
    }

    /// Tell rover `id` to stop its running capture. Frames already received
    /// are still saved once the rover confirms.
    pub fn cancel_point_capture(&self, id: u64) -> Result<()> {
        let request_id = self
            .captures
            .lock()
            .unwrap()
            .get(&id)
            .map(|c| c.collector.lock().unwrap().request.request_id.clone())
            .ok_or_else(|| invalid("no capture"))?;
        let request = CaptureRequest {
            cancel: true,
            ..CaptureRequest::new(request_id)
        };
        let session = self.session.lock().unwrap();
        let session = session.as_ref().ok_or_else(|| invalid("disconnected"))?;
        session
            .put(
                format!("{}/{id}/{REQUEST_TOPIC}", self.topics.prefix),
                serde_json::to_vec(&request).map_err(|e| Error(e.to_string()))?,
            )
            .wait()
            .map_err(|e| Error(e.to_string()))
    }

    /// The latest capture for rover `id`. Once complete, closes its
    /// subscriptions and starts writing the PLY file in the background.
    pub fn point_capture(&self, id: u64) -> Option<CaptureStatus> {
        let mut captures = self.captures.lock().unwrap();
        let capture = captures.get_mut(&id)?;
        let collector = capture.collector.lock().unwrap().clone();
        let finished = capture.subscribers.is_empty()
            || collector.complete(capture.started.elapsed().as_secs_f64());
        if finished && !capture.subscribers.is_empty() {
            capture.subscribers.clear();
            let saved = capture.saved.clone();
            let path = capture.path.clone();
            let collector = collector.clone();
            std::thread::spawn(move || {
                let result = save(&collector, &path).map(|()| path);
                *saved.lock().unwrap() = Some(result);
            });
        }
        Some(CaptureStatus {
            request_id: collector.request.request_id.clone(),
            received: collector.frames.len(),
            expected: collector.expected(),
            state: collector.progress.as_ref().map(|p| p.state),
            detail: collector
                .progress
                .as_ref()
                .map(|p| p.detail.clone())
                .unwrap_or_default(),
            finished,
            saved: capture.saved.lock().unwrap().clone(),
            label_counts: collector.label_counts(),
        })
    }
}

fn save(collector: &CaptureCollector, path: &std::path::Path) -> std::result::Result<(), String> {
    if collector.frames.is_empty() {
        return Err("no frames arrived".into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut out = std::io::BufWriter::new(file);
    write_ply(collector, &mut out).map_err(|e| e.to_string())?;
    std::io::Write::flush(&mut out).map_err(|e| e.to_string())
}
