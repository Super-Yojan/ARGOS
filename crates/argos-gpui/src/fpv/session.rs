//! Dashboard side of one FPV call. The phone creates the offer; we answer.
//!
//! Zenoh publishes only what `SignalingSession::dashboard` returns. A timeout
//! or an operator stop emits `webrtc/request` action `stop`. This task does
//! not publish a drive, arm, or safety payload.

use super::config::FpvConfig;
use super::decode::VideoDecoder;
use super::link::Bus;
use super::{VideoFrame, gpu_frame};
use crate::DashboardConfig;
use fpv_signaling::{CallState, Effect, SignalingSession, Topic};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MIME_TYPE_H264, MIME_TYPE_VP8, MediaEngine};
use webrtc::api::setting_engine::SettingEngine;
use webrtc::ice::mdns::MulticastDnsMode;
use webrtc::ice::network_type::NetworkType;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::RTCPFeedback;
use webrtc::rtp_transceiver::rtp_codec::{
    RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpvPhase {
    Idle,
    Starting,
    Connecting,
    Connected,
    Failed,
    Closed,
}

impl FpvPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Starting => "Starting",
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Failed => "Failed",
            Self::Closed => "Closed",
        }
    }
}

#[derive(Clone)]
pub struct FpvView {
    pub phone_id: Option<u64>,
    pub session: Option<String>,
    pub phase: FpvPhase,
    pub detail: String,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub bitrate_kbps: f32,
    pub rtt_ms: Option<f32>,
    pub frame_seq: u64,
    pub frame: Option<VideoFrame>,
    pub image: Option<Arc<gpui_kit::RenderImage>>,
    /// Frames the UI thread should remove from the sprite atlas.
    pub retired: Vec<Arc<gpui_kit::RenderImage>>,
}

impl Default for FpvView {
    fn default() -> Self {
        Self {
            phone_id: None,
            session: None,
            phase: FpvPhase::Idle,
            detail: "Select a rover, then start FPV".into(),
            codec: String::new(),
            width: 0,
            height: 0,
            fps: 0.,
            bitrate_kbps: 0.,
            rtt_ms: None,
            frame_seq: 0,
            frame: None,
            image: None,
            retired: Vec::new(),
        }
    }
}

enum Command {
    Start(u64),
    Stop,
    Shutdown,
}

struct Running {
    cancel: Arc<AtomicBool>,
    handle: tokio::task::JoinHandle<()>,
}

pub struct FpvController {
    tx: UnboundedSender<Command>,
    view: Arc<Mutex<FpvView>>,
}

impl FpvController {
    pub fn start(config: DashboardConfig) -> Self {
        let (tx, rx) = unbounded_channel();
        let view = Arc::new(Mutex::new(FpvView::default()));
        let shared = view.clone();
        std::thread::Builder::new()
            .name("argos-fpv".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_name("argos-fpv-rt")
                    .worker_threads(2)
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        shared.lock().unwrap().phase = FpvPhase::Failed;
                        shared.lock().unwrap().detail = error.to_string();
                        return;
                    }
                };
                runtime.block_on(worker(rx, shared, config));
            })
            .expect("argos-fpv thread");
        Self { tx, view }
    }

    pub fn start_phone(&self, phone_id: u64) {
        let _ = self.tx.send(Command::Start(phone_id));
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Command::Stop);
    }

    pub fn view(&self) -> FpvView {
        let current = self.view.lock().unwrap();
        FpvView {
            retired: Vec::new(),
            ..current.clone()
        }
    }

    /// Images replaced by a newer frame. The UI drops them from the sprite atlas.
    pub fn take_retired(&self) -> Vec<Arc<gpui_kit::RenderImage>> {
        std::mem::take(&mut self.view.lock().unwrap().retired)
    }

    pub fn frame_seq(&self) -> u64 {
        self.view.lock().unwrap().frame_seq
    }
}

impl Drop for FpvController {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
    }
}

async fn worker(
    mut rx: UnboundedReceiver<Command>,
    view: Arc<Mutex<FpvView>>,
    config: DashboardConfig,
) {
    let mut running: Option<Running> = None;
    while let Some(command) = rx.recv().await {
        match command {
            Command::Shutdown => {
                finish(&mut running).await;
                break;
            }
            Command::Stop => {
                finish(&mut running).await;
                let mut current = view.lock().unwrap();
                current.phase = FpvPhase::Closed;
                current.detail = "FPV stopped".into();
                current.session = None;
            }
            Command::Start(phone_id) => {
                finish(&mut running).await;
                if let Some(notice) = &config.connect_notice {
                    let mut current = view.lock().unwrap();
                    current.phone_id = Some(phone_id);
                    current.phase = FpvPhase::Failed;
                    current.detail = notice.clone();
                    continue;
                }
                let cancel = Arc::new(AtomicBool::new(false));
                let flag = cancel.clone();
                let view_task = view.clone();
                let config = config.clone();
                let handle = tokio::spawn(async move {
                    if let Err(error) =
                        answer_phone(&config, phone_id, flag.clone(), view_task.clone()).await
                        && !flag.load(Ordering::Relaxed)
                    {
                        let mut current = view_task.lock().unwrap();
                        current.phase = FpvPhase::Failed;
                        current.detail = error;
                    }
                });
                running = Some(Running { cancel, handle });
            }
        }
    }
}

async fn finish(running: &mut Option<Running>) {
    if let Some(running) = running.take() {
        running.cancel.store(true, Ordering::Relaxed);
        let _ = tokio::time::timeout(Duration::from_secs(3), running.handle).await;
    }
}

async fn answer_phone(
    config: &DashboardConfig,
    phone_id: u64,
    cancel: Arc<AtomicBool>,
    view: Arc<Mutex<FpvView>>,
) -> Result<(), String> {
    {
        let mut current = view.lock().unwrap();
        *current = FpvView {
            phone_id: Some(phone_id),
            phase: FpvPhase::Starting,
            detail: format!("Asking rover {phone_id} for video"),
            ..FpvView::default()
        };
    }
    let credentials = config
        .credentials
        .as_ref()
        .map(|(user, password)| (user.as_str(), password.as_str()));
    let mut bus = Bus::connect(&config.endpoint, &config.prefix, credentials)?;
    // Subscribe before start. Offers are not replayed.
    let mut offers = bus.subscribe(phone_id, Topic::Offer);
    let mut ice_in = bus.subscribe(phone_id, Topic::IcePhone);
    let mut states = bus.subscribe(phone_id, Topic::State);
    tokio::time::sleep(Duration::from_millis(200)).await;
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }

    let mut machine = SignalingSession::dashboard();
    let session = uuid::Uuid::new_v4().to_string();
    let clock = Instant::now();
    let started = machine.start(&session, 0);
    if started.is_empty() {
        return Err("FPV session id was not a UUID".into());
    }
    view.lock().unwrap().session = Some(session.clone());

    let mut media: Option<Media> = None;
    let mut pending_ice = Vec::new();
    let reader_stop = Arc::new(AtomicBool::new(false));
    let mut last_rtt = Instant::now() - Duration::from_secs(5);
    let mut failure: Option<String> = None;
    let applied = apply_dashboard(
        &mut machine,
        &bus,
        phone_id,
        &config.fpv,
        &mut media,
        &reader_stop,
        &cancel,
        &view,
        &mut pending_ice,
        0,
        started,
    )
    .await?;
    if let Some(detail) = applied {
        failure = Some(detail);
    }

    let result = 'call: loop {
        if cancel.load(Ordering::Relaxed) {
            break 'call Ok(());
        }
        let now = elapsed_ms(clock);
        let mut effects = Vec::new();
        while let Ok(bytes) = offers.try_recv() {
            if let Some(text) = utf8(&bytes) {
                effects.extend(machine.receive(Topic::Offer.suffix(), &text, now));
            }
        }
        while let Ok(bytes) = ice_in.try_recv() {
            if let Some(text) = utf8(&bytes) {
                effects.extend(machine.receive(Topic::IcePhone.suffix(), &text, now));
            }
        }
        while let Ok(bytes) = states.try_recv() {
            if let Some(text) = utf8(&bytes) {
                effects.extend(machine.receive(Topic::State.suffix(), &text, now));
            }
        }
        if let Some(media) = media.as_mut() {
            let mut local = Vec::new();
            while let Ok(init) = media.local_ice.try_recv() {
                local.push(init);
            }
            let mut peer_states = Vec::new();
            while let Ok(state) = media.states.try_recv() {
                peer_states.push(state);
            }
            for init in local {
                let (candidate, sdp_mid, index) = ice_parts(init);
                effects.extend(machine.note_local_ice(&session, &candidate, &sdp_mid, index, now));
            }
            for state in peer_states {
                if let Some(call) = call_state(state) {
                    effects.extend(machine.note_media(&session, call, now));
                }
            }
        }
        effects.extend(machine.tick(now));
        let applied = apply_dashboard(
            &mut machine,
            &bus,
            phone_id,
            &config.fpv,
            &mut media,
            &reader_stop,
            &cancel,
            &view,
            &mut pending_ice,
            now,
            effects,
        )
        .await?;
        if let Some(detail) = applied {
            failure = Some(detail);
        }
        if machine.is_failed() {
            break 'call Err(failure.unwrap_or_else(|| "Video call failed".into()));
        }
        if !machine.has_session() {
            break 'call Ok(());
        }
        if media
            .as_ref()
            .is_some_and(|media| media.peer.connection_state() == RTCPeerConnectionState::Connected)
            && last_rtt.elapsed() >= Duration::from_secs(1)
        {
            last_rtt = Instant::now();
            if let Some(media) = media.as_ref()
                && let Ok(Some(rtt)) =
                    tokio::time::timeout(Duration::from_millis(200), current_rtt(&media.peer)).await
            {
                view.lock().unwrap().rtt_ms = Some(rtt);
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    if cancel.load(Ordering::Relaxed) && machine.has_session() {
        let now = elapsed_ms(clock);
        let effects = machine.stop(true);
        apply_dashboard(
            &mut machine,
            &bus,
            phone_id,
            &config.fpv,
            &mut media,
            &reader_stop,
            &cancel,
            &view,
            &mut pending_ice,
            now,
            effects,
        )
        .await?;
    }
    reader_stop.store(true, Ordering::Relaxed);
    if let Some(media) = media.as_ref() {
        let _ = media.peer.close().await;
    }
    result
}

struct Media {
    peer: Arc<RTCPeerConnection>,
    local_ice: std::sync::mpsc::Receiver<
        Option<webrtc::ice_transport::ice_candidate::RTCIceCandidateInit>,
    >,
    states: UnboundedReceiver<RTCPeerConnectionState>,
    remote_ready: bool,
}

#[allow(clippy::too_many_arguments)]
async fn apply_dashboard(
    machine: &mut SignalingSession,
    bus: &Bus,
    phone_id: u64,
    ice: &FpvConfig,
    media: &mut Option<Media>,
    reader_stop: &Arc<AtomicBool>,
    cancel: &Arc<AtomicBool>,
    view: &Arc<Mutex<FpvView>>,
    pending_ice: &mut Vec<webrtc::ice_transport::ice_candidate::RTCIceCandidateInit>,
    now: u64,
    effects: Vec<Effect>,
) -> Result<Option<String>, String> {
    let mut queue = VecDeque::from(effects);
    let mut failure = None;
    while let Some(effect) = queue.pop_front() {
        match effect {
            Effect::Publish { topic, payload } => bus.publish(phone_id, topic, &payload)?,
            Effect::AcceptOffer { session, sdp } => {
                match accept_offer(ice, media, reader_stop, cancel, view, &sdp).await {
                    Ok(answer) => {
                        queue.extend(machine.note_local_description(&session, &answer, now));
                        if let Some(media) = media.as_ref() {
                            flush_ice(&media.peer, pending_ice).await;
                        }
                    }
                    Err(error) => {
                        failure = Some(error);
                        queue.extend(machine.note_media(&session, CallState::Failed, now));
                    }
                }
            }
            Effect::AddIce {
                candidate,
                sdp_mid,
                sdp_m_line_index,
                ..
            } => {
                pending_ice.push(candidate_init(candidate, sdp_mid, sdp_m_line_index));
                if let Some(media) = media.as_ref().filter(|media| media.remote_ready) {
                    flush_ice(&media.peer, pending_ice).await;
                }
            }
            Effect::EndOfCandidates { .. } => {}
            Effect::StopMedia { .. } => {
                reader_stop.store(true, Ordering::Relaxed);
                if let Some(media) = media.as_ref() {
                    let _ = media.peer.close().await;
                }
            }
            Effect::Show { state, detail, .. } => {
                let text = show_text(state, detail, media.is_some(), failure.as_deref());
                if state == CallState::Failed && failure.is_none() {
                    failure = Some(text.clone());
                }
                set_phase(view, phase_of(state), &text);
            }
            Effect::CreateOffer { .. } | Effect::ApplyAnswer { .. } => {}
        }
    }
    Ok(failure)
}

async fn accept_offer(
    ice: &FpvConfig,
    media: &mut Option<Media>,
    reader_stop: &Arc<AtomicBool>,
    cancel: &Arc<AtomicBool>,
    view: &Arc<Mutex<FpvView>>,
    sdp: &str,
) -> Result<String, String> {
    let peer = build_peer(ice).await?;
    let (local_ice_tx, local_ice) = std::sync::mpsc::sync_channel(64);
    peer.on_ice_candidate(Box::new(move |candidate| {
        let init = candidate.and_then(|candidate| candidate.to_json().ok());
        let _ = local_ice_tx.try_send(init);
        Box::pin(async {})
    }));
    let (state_tx, states) = unbounded_channel();
    peer.on_peer_connection_state_change(Box::new(move |state: RTCPeerConnectionState| {
        let _ = state_tx.send(state);
        Box::pin(async {})
    }));
    let view_track = view.clone();
    let cancel_track = cancel.clone();
    let stop_track = reader_stop.clone();
    let peer_pli = peer.clone();
    peer.on_track(Box::new(move |track, _, _| {
        let view = view_track.clone();
        let cancel = cancel_track.clone();
        let reader_stop = stop_track.clone();
        let peer_pli = peer_pli.clone();
        Box::pin(async move {
            if track.kind() != RTPCodecType::Video {
                return;
            }
            let mime = track.codec().capability.mime_type.clone();
            let mut decoder = match VideoDecoder::new() {
                Ok(decoder) => decoder,
                Err(error) => {
                    view.lock().unwrap().detail = error;
                    return;
                }
            };
            decoder.set_mime(&mime);
            view.lock().unwrap().codec = decoder.codec_name().to_string();
            let ssrc = track.ssrc();
            let _ = peer_pli
                .write_rtcp(&[Box::new(
                    webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication {
                        sender_ssrc: 0,
                        media_ssrc: ssrc,
                    },
                )])
                .await;
            let mut rates = Rates::default();
            if let Err(error) =
                read_track(&track, &mut decoder, &view, &mut rates, &cancel, &reader_stop).await
            {
                view.lock().unwrap().detail = error;
            }
        })
    }));
    let remote =
        RTCSessionDescription::offer(sdp.to_string()).map_err(|error| error.to_string())?;
    peer.set_remote_description(remote)
        .await
        .map_err(|error| error.to_string())?;
    let answer = peer
        .create_answer(None)
        .await
        .map_err(|error| error.to_string())?;
    let answer_sdp = answer.sdp.clone();
    peer.set_local_description(answer)
        .await
        .map_err(|error| error.to_string())?;
    *media = Some(Media {
        peer,
        local_ice,
        states,
        remote_ready: true,
    });
    Ok(answer_sdp)
}

fn show_text(state: CallState, detail: &str, have_media: bool, failure: Option<&str>) -> String {
    match state {
        CallState::Connecting => {
            if have_media {
                "Connecting video".into()
            } else {
                "Waiting for the offer".into()
            }
        }
        CallState::Connected => "Video call connected".into(),
        CallState::Failed => failure
            .unwrap_or(match detail {
                "timed_out" => "The video call timed out",
                "peer_failed" => "The phone reported the video call failed",
                "media_failed" => "WebRTC connection failed",
                "" => "Video call failed",
                other => other,
            })
            .to_string(),
        CallState::Closed => "FPV stopped".into(),
    }
}

fn phase_of(state: CallState) -> FpvPhase {
    match state {
        CallState::Connecting => FpvPhase::Connecting,
        CallState::Connected => FpvPhase::Connected,
        CallState::Failed => FpvPhase::Failed,
        CallState::Closed => FpvPhase::Closed,
    }
}

fn call_state(state: RTCPeerConnectionState) -> Option<CallState> {
    match state {
        RTCPeerConnectionState::Connected => Some(CallState::Connected),
        RTCPeerConnectionState::Failed => Some(CallState::Failed),
        RTCPeerConnectionState::Closed => Some(CallState::Closed),
        RTCPeerConnectionState::Disconnected => Some(CallState::Connecting),
        _ => None,
    }
}

fn ice_parts(
    init: Option<webrtc::ice_transport::ice_candidate::RTCIceCandidateInit>,
) -> (String, String, i32) {
    match init {
        Some(init) => (
            init.candidate,
            init.sdp_mid.unwrap_or_default(),
            i32::from(init.sdp_mline_index.unwrap_or(0)),
        ),
        None => (String::new(), String::new(), 0),
    }
}

fn candidate_init(
    candidate: String,
    sdp_mid: String,
    sdp_m_line_index: i32,
) -> webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
    webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
        candidate,
        sdp_mid: (!sdp_mid.is_empty()).then_some(sdp_mid),
        sdp_mline_index: u16::try_from(sdp_m_line_index).ok(),
        username_fragment: None,
    }
}

async fn flush_ice(
    peer: &RTCPeerConnection,
    pending: &mut Vec<webrtc::ice_transport::ice_candidate::RTCIceCandidateInit>,
) {
    for init in pending.drain(..) {
        let _ = peer.add_ice_candidate(init).await;
    }
}

fn utf8(bytes: &[u8]) -> Option<String> {
    String::from_utf8(bytes.to_vec()).ok()
}

fn elapsed_ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

async fn read_track(
    track: &webrtc::track::track_remote::TrackRemote,
    decoder: &mut VideoDecoder,
    view: &Mutex<FpvView>,
    rates: &mut Rates,
    cancel: &AtomicBool,
    reader_stop: &AtomicBool,
) -> Result<(), String> {
    let mut buffer = vec![0u8; 1500];
    loop {
        if cancel.load(Ordering::Relaxed) || reader_stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let packet = tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                if cancel.load(Ordering::Relaxed) || reader_stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                continue;
            }
            incoming = track.read(&mut buffer) => incoming,
        };
        let (packet, _) = match packet {
            Ok(packet) => packet,
            Err(_) => return Ok(()),
        };
        let size = packet.payload.len();
        rates.add_bytes(size as u64);
        match decoder.push(
            packet.header.timestamp,
            packet.header.marker,
            &packet.payload,
        ) {
            Ok(Some(frame)) => {
                rates.add_frame();
                store_frame(view, frame, rates);
            }
            Ok(None) => {}
            Err(error) => {
                view.lock().unwrap().detail = error;
            }
        }
    }
}

fn store_frame(view: &Mutex<FpvView>, frame: VideoFrame, rates: &Rates) {
    let mut current = view.lock().unwrap();
    current.width = frame.width;
    current.height = frame.height;
    current.fps = rates.fps;
    current.bitrate_kbps = rates.kbps;
    current.frame_seq = current.frame_seq.saturating_add(1);
    if let Some(previous) = current.image.take() {
        current.retired.push(previous);
    }
    current.image = gpu_frame(&frame);
    current.frame = Some(frame);
    if current.phase != FpvPhase::Connected {
        current.phase = FpvPhase::Connected;
        current.detail = "Receiving video".into();
    }
}

fn set_phase(view: &Mutex<FpvView>, phase: FpvPhase, detail: &str) {
    let mut current = view.lock().unwrap();
    current.phase = phase;
    current.detail = detail.to_string();
}

pub(crate) async fn build_peer(ice: &FpvConfig) -> Result<Arc<RTCPeerConnection>, String> {
    let mut media = MediaEngine::default();
    register_video(&mut media).map_err(|error| error.to_string())?;
    let registry = register_default_interceptors(Registry::new(), &mut media)
        .map_err(|error| error.to_string())?;
    let mut setting = SettingEngine::default();
    // The phone opens no data channel and ignores one. This peer never creates
    // one. `detach_data_channels` only applies if a remote channel arrives.
    setting.detach_data_channels();
    setting.set_ice_multicast_dns_mode(MulticastDnsMode::Disabled);
    setting.set_network_types(vec![NetworkType::Udp4]);
    setting.set_host_acceptance_min_wait(Some(Duration::ZERO));
    let local_only = ice.stun_urls.is_empty() && ice.turn.is_none();
    setting.set_include_loopback_candidate(local_only);
    if local_only {
        // Docker and link-local addresses on this host pass an initial check and
        // then fail ICE consent. A host-only call stays on loopback.
        setting.set_ip_filter(Box::new(|ip: std::net::IpAddr| ip.is_loopback()));
    }
    let api = APIBuilder::new()
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_setting_engine(setting)
        .build();
    Ok(Arc::new(
        api.new_peer_connection(RTCConfiguration {
            ice_servers: ice_servers(ice),
            ..RTCConfiguration::default()
        })
        .await
        .map_err(|error| error.to_string())?,
    ))
}

fn ice_servers(ice: &FpvConfig) -> Vec<RTCIceServer> {
    let mut servers = Vec::new();
    if !ice.stun_urls.is_empty() {
        servers.push(RTCIceServer {
            urls: ice.stun_urls.clone(),
            ..RTCIceServer::default()
        });
    }
    if let Some(turn) = &ice.turn {
        servers.push(RTCIceServer {
            urls: vec![turn.url.clone()],
            username: turn.username.clone(),
            credential: turn.credential.clone(),
        });
    }
    servers
}

fn register_video(media: &mut MediaEngine) -> webrtc::error::Result<()> {
    for codec in preferred_video_codecs() {
        media.register_codec(codec, RTPCodecType::Video)?;
    }
    Ok(())
}

/// H.264 profiles first, then VP8. Matches the phone's offer order.
pub(crate) fn preferred_video_codecs() -> Vec<RTCRtpCodecParameters> {
    let feedback = vec![
        RTCPFeedback {
            typ: "goog-remb".into(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: "ccm".into(),
            parameter: "fir".into(),
        },
        RTCPFeedback {
            typ: "nack".into(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: "nack".into(),
            parameter: "pli".into(),
        },
    ];
    let profiles = [
        (
            102,
            "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f",
        ),
        (
            125,
            "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f",
        ),
        (
            108,
            "level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42e01f",
        ),
    ];
    let mut codecs = Vec::new();
    for (payload_type, fmtp) in profiles {
        codecs.push(RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_owned(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line: fmtp.to_owned(),
                rtcp_feedback: feedback.clone(),
            },
            payload_type,
            ..RTCRtpCodecParameters::default()
        });
    }
    codecs.push(RTCRtpCodecParameters {
        capability: RTCRtpCodecCapability {
            mime_type: MIME_TYPE_VP8.to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: String::new(),
            rtcp_feedback: feedback,
        },
        payload_type: 96,
        ..RTCRtpCodecParameters::default()
    });
    codecs
}

async fn current_rtt(peer: &RTCPeerConnection) -> Option<f32> {
    let report = peer.get_stats().await;
    report.reports.values().find_map(|stats| match stats {
        webrtc::stats::StatsReportType::CandidatePair(pair)
            if pair.nominated && pair.current_round_trip_time > 0. =>
        {
            Some((pair.current_round_trip_time * 1000.) as f32)
        }
        _ => None,
    })
}

#[derive(Default)]
struct Rates {
    window_start: Option<Instant>,
    frames: u32,
    bytes: u64,
    fps: f32,
    kbps: f32,
}

impl Rates {
    fn add_bytes(&mut self, bytes: u64) {
        self.bytes = self.bytes.saturating_add(bytes);
        self.roll();
    }

    fn add_frame(&mut self) {
        self.frames = self.frames.saturating_add(1);
        self.roll();
    }

    fn roll(&mut self) {
        let start = *self.window_start.get_or_insert_with(Instant::now);
        let elapsed = start.elapsed().as_secs_f32();
        if elapsed >= 1. {
            self.fps = self.frames as f32 / elapsed;
            self.kbps = (self.bytes as f32 * 8. / 1000.) / elapsed;
            self.frames = 0;
            self.bytes = 0;
            self.window_start = Some(Instant::now());
        }
    }
}
