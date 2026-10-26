//! Ask one rover for a full labelled point capture and save it as PLY.
//!
//! [`Client::start_point_capture`] subscribes to that rover's
//! `pointcloud/full` and `pointcloud/capture`, then puts the request. The
//! subscriptions exist only while a capture runs: [`Client::point_capture`]
//! drops them once the capture is complete and writes the PLY file off the
//! caller's thread.
//!
//! [`Client::start_live_points`] asks for a decimated live stream instead and
//! keeps only the newest frame for drawing. A background thread renews the
//! rover's lease while the stream is wanted; stopping it, disconnecting, or
//! the rover ending the stream stops the renewals, so the rover stops within
//! one lease even if the cancel is lost.
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

/// Live stream frame rate asked of the rover.
pub const LIVE_RATE_HZ: f64 = 2.0;
/// Live stream pixel stride: 256 × 192 LiDAR depth becomes 86 × 64.
pub const LIVE_STRIDE: u32 = 3;
/// Lease the rover grants per request; renewed every [`LIVE_RENEW_SECONDS`].
const LIVE_LEASE_SECONDS: f64 = 6.0;
const LIVE_RENEW_SECONDS: f64 = 2.0;

/// One rover's live stream: newest frame, rover state, and renewal control.
pub(crate) struct Live {
    request_id: String,
    started: Instant,
    latest: Arc<Mutex<Option<(LiveCloud, Instant)>>>,
    progress: Arc<Mutex<Option<CaptureProgress>>>,
    frames: Arc<std::sync::atomic::AtomicU64>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    _subscribers: Vec<zenoh::pubsub::Subscriber<()>>,
}
impl Drop for Live {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
    }
}
pub(crate) type Lives = Mutex<std::collections::BTreeMap<u64, Live>>;

/// What the dashboard shows and draws for a rover's live stream.
#[derive(Clone, Debug)]
pub struct LiveStatus {
    /// Newest frame and when it arrived, if any has.
    pub cloud: Option<(LiveCloud, Instant)>,
    /// Frames received since the stream started.
    pub frames: u64,
    /// Seconds since the stream was asked for.
    pub elapsed: f64,
    /// Set once the rover ended the stream, with its reason, such as
    /// `lease_expired`, `replaced` or `cancelled`.
    pub ended: Option<String>,
}

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
    pub label_counts: [usize; LABELS],
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

impl Client {
    /// Ask rover `id` for a live labelled stream at [`LIVE_RATE_HZ`] and
    /// [`LIVE_STRIDE`], replacing any earlier stream for that rover.
    pub fn start_live_points(&self, id: u64) -> Result<()> {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        let request_id = format!(
            "live-{}",
            uuid::Uuid::new_v4()
                .simple()
                .to_string()
                .get(..12)
                .unwrap_or("stream")
        );
        let request = CaptureRequest::live(
            request_id.clone(),
            LIVE_RATE_HZ,
            LIVE_STRIDE,
            LIVE_LEASE_SECONDS,
        );
        let payload = serde_json::to_vec(&request).map_err(|e| Error(e.to_string()))?;
        let base = format!("{}/{id}", self.topics.prefix);
        let session = self
            .session
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| invalid("disconnected"))?;
        let latest = Arc::new(Mutex::new(None));
        let progress = Arc::new(Mutex::new(None));
        let frames = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (sink, count, wanted) = (latest.clone(), frames.clone(), request_id.clone());
        let full = session
            .declare_subscriber(format!("{base}/{FRAME_TOPIC}"))
            .callback(move |sample| {
                let Ok(frame) = CaptureFrame::decode(&sample.payload().to_bytes()) else {
                    return;
                };
                if frame.rover_id == id && frame.request_id == wanted {
                    *sink.lock().unwrap() = Some((LiveCloud::from_frame(&frame), Instant::now()));
                    count.fetch_add(1, Ordering::Relaxed);
                }
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let (reports, ended, wanted) = (progress.clone(), stop.clone(), request_id.clone());
        let state = session
            .declare_subscriber(format!("{base}/{PROGRESS_TOPIC}"))
            .callback(move |sample| {
                let Ok(report) =
                    serde_json::from_slice::<CaptureProgress>(&sample.payload().to_bytes())
                else {
                    return;
                };
                if report.rover_id == id && report.request_id == wanted {
                    // A renewal after the rover ended the stream would start
                    // a new one and could replace an operator's recorded capture.
                    if report.state != CaptureState::Running {
                        ended.store(true, Ordering::Release);
                    }
                    *reports.lock().unwrap() = Some(report);
                }
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let key = format!("{base}/{REQUEST_TOPIC}");
        session
            .put(&key, payload.clone())
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let _ = self.recorder.record(serde_json::json!({
            "kind": "point_live_requested",
            "rover_id": id,
            "time": self.start.elapsed().as_secs_f64(),
            "session_id": self.session_id,
            "payload": request,
        }));
        let renew = stop.clone();
        std::thread::spawn(move || {
            let period = std::time::Duration::from_secs_f64(LIVE_RENEW_SECONDS);
            let tick = std::time::Duration::from_millis(100);
            let mut last = Instant::now();
            while !renew.load(Ordering::Acquire) {
                std::thread::sleep(tick);
                if last.elapsed() >= period && !renew.load(Ordering::Acquire) {
                    last = Instant::now();
                    let _ = session.put(&key, payload.clone()).wait();
                }
            }
        });
        self.lives.lock().unwrap().insert(
            id,
            Live {
                request_id,
                started: Instant::now(),
                latest,
                progress,
                frames,
                stop,
                _subscribers: vec![full, state],
            },
        );
        Ok(())
    }

    /// Stop rover `id`'s live stream and forget its frames.
    pub fn stop_live_points(&self, id: u64) -> Result<()> {
        let Some(live) = self.lives.lock().unwrap().remove(&id) else {
            return Ok(());
        };
        let request = CaptureRequest {
            cancel: true,
            ..CaptureRequest::new(live.request_id.clone())
        };
        drop(live);
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

    /// Rover `id`'s live stream, while one is wanted.
    pub fn live_points(&self, id: u64) -> Option<LiveStatus> {
        let lives = self.lives.lock().unwrap();
        let live = lives.get(&id)?;
        Some(LiveStatus {
            cloud: live.latest.lock().unwrap().clone(),
            frames: live.frames.load(std::sync::atomic::Ordering::Relaxed),
            elapsed: live.started.elapsed().as_secs_f64(),
            ended: live
                .progress
                .lock()
                .unwrap()
                .as_ref()
                .filter(|p| p.state != CaptureState::Running)
                .map(|p| {
                    if p.detail.is_empty() {
                        format!("{:?}", p.state).to_lowercase()
                    } else {
                        p.detail.clone()
                    }
                }),
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
