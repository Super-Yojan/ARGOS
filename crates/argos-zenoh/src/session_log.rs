use serde_json::Value;
use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
enum Message {
    Event(Value),
    Flush(std::sync::mpsc::Sender<()>),
    Close,
}
pub struct RunRecorder {
    sender: SyncSender<Message>,
    worker: std::sync::Mutex<Option<JoinHandle<io::Result<()>>>>,
    failed: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
}
impl RunRecorder {
    /// Recording failure must not prevent controls from connecting.
    pub fn unavailable() -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(receiver);
        Self {
            sender,
            worker: std::sync::Mutex::new(None),
            failed: Arc::new(AtomicBool::new(true)),
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }
    pub fn create(path: &Path, mut manifest: Value, capacity: usize) -> io::Result<Self> {
        if !(1..=65536).contains(&capacity) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid recording capacity",
            ));
        }
        let file = File::create(path)?;
        manifest["schema_version"] = 1.into();
        manifest["kind"] = "manifest".into();
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let failed = Arc::new(AtomicBool::new(false));
        let state = failed.clone();
        let dropped = Arc::new(AtomicU64::new(0));
        let drops = dropped.clone();
        let worker = thread::spawn(move || {
            let result = (|| {
                let mut w = BufWriter::new(file);
                line(&mut w, &manifest)?;
                w.flush()?;
                loop {
                    match receiver.recv_timeout(Duration::from_secs(1)) {
                        Ok(Message::Event(v)) => line(&mut w, &v)?,
                        Ok(Message::Flush(done)) => {
                            w.flush()?;
                            let _ = done.send(());
                        }
                        Ok(Message::Close) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                            line(
                                &mut w,
                                &serde_json::json!({"kind":"run_end","complete":!state.load(Ordering::Acquire),"dropped_events":drops.load(Ordering::Acquire)}),
                            )?;
                            w.flush()?;
                            return Ok(());
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => w.flush()?,
                    }
                }
            })();
            if result.is_err() {
                state.store(true, Ordering::Release);
            }
            result
        });
        Ok(Self {
            sender,
            worker: std::sync::Mutex::new(Some(worker)),
            failed,
            dropped,
        })
    }
    pub fn record(&self, event: Value) -> io::Result<()> {
        if self.failed() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return Err(io::Error::other("recording incomplete"));
        }
        self.sender.try_send(Message::Event(event)).map_err(|e| {
            self.failed.store(true, Ordering::Release);
            self.dropped.fetch_add(1, Ordering::Relaxed);
            io::Error::other(match e {
                TrySendError::Full(_) => "recording queue full",
                TrySendError::Disconnected(_) => "recording worker stopped",
            })
        })
    }
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
    pub fn flush(&self) -> io::Result<()> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.sender
            .try_send(Message::Flush(tx))
            .map_err(|_| io::Error::other("worker stopped"))?;
        rx.recv_timeout(Duration::from_secs(3))
            .map_err(|_| io::Error::other("flush timeout"))
    }
    pub fn close(&self) -> io::Result<()> {
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = self.sender.send(Message::Close);
            worker
                .join()
                .map_err(|_| io::Error::other("recording worker panicked"))??;
        }
        if self.failed() {
            Err(io::Error::other("recording incomplete"))
        } else {
            Ok(())
        }
    }
}
impl Drop for RunRecorder {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
fn line(w: &mut impl Write, v: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *w, v)?;
    w.write_all(b"\n")
}
/// Ignore only an interrupted final line, never malformed complete records.
pub fn read_records(bytes: &[u8]) -> serde_json::Result<Vec<Value>> {
    let mut records = Vec::new();
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        if !line.ends_with(b"\n") {
            break;
        }
        records.push(serde_json::from_slice(line)?);
    }
    Ok(records)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_recorder_exposes_incomplete_without_control_dependencies() {
        let log = RunRecorder::unavailable();
        assert!(log.failed());
        assert!(log.record(serde_json::json!({"kind":"action"})).is_err());
        assert!(log.flush().is_err());
    }
    #[test]
    fn export_barrier_keeps_complete_lines_and_tail_recovery() {
        let path =
            std::env::temp_dir().join(format!("argos-writer-test-{}.jsonl", std::process::id()));
        let log = RunRecorder::create(&path, serde_json::json!({}), 16).unwrap();
        log.record(serde_json::json!({"kind":"action","token":"correlated"}))
            .unwrap();
        log.flush().unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"{broken");
        assert_eq!(read_records(&bytes).unwrap().len(), 2);
        log.close().unwrap();
        assert!(
            read_records(&std::fs::read(&path).unwrap())
                .unwrap()
                .last()
                .unwrap()["complete"]
                == true
        );
        std::fs::remove_file(path).unwrap();
    }
}
