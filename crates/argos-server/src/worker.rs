use crate::{
    engine::{Engine, Fleet},
    store::Store,
    types::*,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant},
};
use tokio::sync::{oneshot, watch};

pub enum Action {
    Drive(String, DriveRequest, Instant),
    EmergencyStop(String),
    Claim,
    Release,
    Submit(String, WaypointRequest),
    Cancel(String, String),
    Hardware(String, HardwareRequest),
    Autonomy(String, AutonomyRequest),
    Operation(String),
    Occupancy(String),
    Navigation(String, NavigationRequest),
    Source(String, SourceRequest),
}
pub struct Request {
    pub user: Identity,
    pub owner: String,
    pub action: Action,
    pub reply: oneshot::Sender<Result<serde_json::Value, String>>,
}
#[derive(Clone)]
pub struct Worker {
    pub sender: SyncSender<Request>,
    pub snapshots: watch::Receiver<FleetSnapshot>,
    stop: Arc<AtomicBool>,
}
impl Worker {
    pub async fn start(fleet: Box<dyn Fleet>, store: Store) -> Result<Self, String> {
        let recovered = store.recover().await?;
        let (sender, receiver) = mpsc::sync_channel::<Request>(64);
        let (snapshots, rx) = watch::channel(FleetSnapshot::default());
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let runtime = tokio::runtime::Handle::current();
        std::thread::Builder::new()
            .name("argos-fleet".into())
            .spawn(move || {
                let mut engine = Engine::new(fleet);
                for op in recovered {
                    engine.operations.insert(op.id.clone(), op);
                }
                engine.journal(Box::new(move |op| runtime.block_on(store.record(op))));
                let start = Instant::now();
                while !stopped.load(Ordering::Relaxed) {
                    let at = Instant::now();
                    for _ in 0..32 {
                        let Ok(r) = receiver.try_recv() else { break };
                        let now = start.elapsed().as_secs_f64();
                        let result = match r.action {
                            Action::Drive(id, request, received) => {
                                if received.elapsed() >= Duration::from_millis(250)
                                    || r.reply.is_closed()
                                {
                                    Err("Drive input expired in queue".into())
                                } else {
                                    engine
                                        .drive(&r.user, &r.owner, &id, request, now)
                                        .map(|_| serde_json::json!({"published":true}))
                                }
                            }
                            Action::EmergencyStop(id) => engine
                                .emergency_stop(&r.user, &r.owner, &id, now)
                                .map(|_| serde_json::json!({"published":true})),
                            Action::Claim => engine
                                .claim(&r.user, &r.owner, now)
                                .and_then(|c| serde_json::to_value(c).map_err(|e| e.to_string())),
                            Action::Release => {
                                engine.release(&r.owner);
                                Ok(serde_json::json!({"released":true}))
                            }
                            Action::Submit(id, request) => engine
                                .submit(&r.user, &r.owner, &id, request, now)
                                .and_then(|o| serde_json::to_value(o).map_err(|e| e.to_string())),
                            Action::Cancel(id, request) => engine
                                .cancel(&r.user, &r.owner, &id, &request, now)
                                .and_then(|o| serde_json::to_value(o).map_err(|e| e.to_string())),
                            Action::Hardware(id, request) => engine
                                .hardware(&r.user, &r.owner, &id, request, now)
                                .and_then(|o| serde_json::to_value(o).map_err(|e| e.to_string())),
                            Action::Autonomy(id, request) => engine
                                .autonomy(&r.user, &r.owner, &id, request, now)
                                .and_then(|o| serde_json::to_value(o).map_err(|e| e.to_string())),
                            Action::Occupancy(id) => engine
                                .fleet
                                .occupancy(&id, now)
                                .and_then(|o| serde_json::to_value(o).map_err(|e| e.to_string())),
                            Action::Navigation(id, request) => engine
                                .navigation(&r.user, &r.owner, &id, request, now)
                                .and_then(|o| serde_json::to_value(o).map_err(|e| e.to_string())),
                            Action::Source(id, request) => engine
                                .select_source(&r.user, &r.owner, &id, request, now)
                                .and_then(|o| serde_json::to_value(o).map_err(|e| e.to_string())),
                            Action::Operation(id) => engine
                                .operations
                                .get(&id)
                                .ok_or("Unknown operation".into())
                                .and_then(|o| serde_json::to_value(o).map_err(|e| e.to_string())),
                        };
                        let _ = r.reply.send(result);
                    }
                    let now = start.elapsed().as_secs_f64();
                    engine.tick(now);
                    snapshots.send_replace(engine.snapshot(now));
                    std::thread::sleep(Duration::from_millis(200).saturating_sub(at.elapsed()));
                }
                engine.claim = None;
                engine.tick(start.elapsed().as_secs_f64());
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            sender,
            snapshots: rx,
            stop,
        })
    }
    pub async fn request(
        &self,
        user: Identity,
        owner: String,
        action: Action,
    ) -> Result<serde_json::Value, String> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .try_send(Request {
                user,
                owner,
                action,
                reply,
            })
            .map_err(|_| "Fleet worker busy or unavailable")?;
        tokio::time::timeout(Duration::from_secs(10), rx)
            .await
            .map_err(|_| "Fleet operation outcome uncertain; check status before acting again")?
            .map_err(|_| "Fleet worker stopped")?
    }
    pub fn release(&self, user: Identity, owner: String) {
        let (reply, _) = oneshot::channel();
        let _ = self.sender.try_send(Request {
            user,
            owner,
            action: Action::Release,
            reply,
        });
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
