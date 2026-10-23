//! Local stand-in for TerraPhone. It answers a dashboard `start` with an H.264 offer
//! and a generated test pattern. Zenoh publishes only effects from
//! `SignalingSession::phone`. It never publishes a motion command.

use super::config::FpvConfig;
use super::link::Bus;
use super::pattern::test_pattern_yuv;
use super::session::{build_peer, preferred_video_codecs};
use bytes::Bytes;
use fpv_signaling::{CallState, Effect, SignalingSession, Topic};
use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use webrtc::api::media_engine::MIME_TYPE_H264;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::media::Sample;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

const FRAME: Duration = Duration::from_millis(66);

pub struct SenderOptions {
    pub endpoint: String,
    pub prefix: String,
    pub phone_id: u64,
    pub ice: FpvConfig,
    pub announce: bool,
    pub user: Option<String>,
    pub password: Option<String>,
    pub cancel: Arc<AtomicBool>,
}

pub fn run_test_sender(options: SenderOptions) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("argos-fpv-sender")
        .worker_threads(2)
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(run_sender(options))
}

async fn run_sender(options: SenderOptions) -> Result<(), String> {
    let credentials = options.user.as_deref().zip(options.password.as_deref());
    let mut bus = Bus::connect(&options.endpoint, &options.prefix, credentials)?;
    let mut requests = bus.subscribe(options.phone_id, Topic::Request);
    let mut answers = bus.subscribe(options.phone_id, Topic::Answer);
    let mut ice_in = bus.subscribe(options.phone_id, Topic::IceDashboard);
    tokio::time::sleep(Duration::from_millis(200)).await;
    eprintln!(
        "fpv-test-sender: waiting for start on {}/{}/webrtc/request",
        options.prefix, options.phone_id
    );

    let mut machine = SignalingSession::phone();
    let mut live: Option<Live> = None;
    let mut pending_ice = Vec::new();
    let mut session = String::new();
    let clock = Instant::now();
    let mut sequence = 0u64;
    let mut next_announce = Instant::now();
    while !options.cancel.load(Ordering::Relaxed) {
        if options.announce && next_announce.elapsed() >= Duration::from_millis(200) {
            sequence = sequence.saturating_add(1);
            bus.publish_announce(options.phone_id, sequence)?;
            next_announce = Instant::now();
        }
        let now = elapsed_ms(clock);
        let mut effects = Vec::new();
        while let Ok(bytes) = requests.try_recv() {
            if let Ok(text) = String::from_utf8(bytes) {
                effects.extend(machine.receive(Topic::Request.suffix(), &text, now));
            }
        }
        while let Ok(bytes) = answers.try_recv() {
            if let Ok(text) = String::from_utf8(bytes) {
                effects.extend(machine.receive(Topic::Answer.suffix(), &text, now));
            }
        }
        while let Ok(bytes) = ice_in.try_recv() {
            if let Ok(text) = String::from_utf8(bytes) {
                effects.extend(machine.receive(Topic::IceDashboard.suffix(), &text, now));
            }
        }
        if let Some(current) = live.as_mut() {
            let mut local = Vec::new();
            while let Ok(init) = current.local_ice.try_recv() {
                local.push(init);
            }
            let mut states = Vec::new();
            while let Ok(state) = current.states.try_recv() {
                states.push(state);
            }
            for init in local {
                let (candidate, sdp_mid, index) = ice_parts(init);
                effects.extend(machine.note_local_ice(&session, &candidate, &sdp_mid, index, now));
            }
            for state in states {
                if let Some(call) = call_state(state) {
                    effects.extend(machine.note_media(&session, call, now));
                }
            }
        }
        effects.extend(machine.tick(now));
        apply_phone(
            &mut machine,
            &bus,
            &options,
            &mut live,
            &mut pending_ice,
            &mut session,
            &options.cancel,
            now,
            effects,
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    if machine.has_session() {
        let now = elapsed_ms(clock);
        let effects = machine.stop(true);
        apply_phone(
            &mut machine,
            &bus,
            &options,
            &mut live,
            &mut pending_ice,
            &mut session,
            &options.cancel,
            now,
            effects,
        )
        .await?;
    }
    Ok(())
}

struct Live {
    peer: Arc<RTCPeerConnection>,
    track: Arc<TrackLocalStaticSample>,
    local_ice: std::sync::mpsc::Receiver<Option<RTCIceCandidateInit>>,
    states: std::sync::mpsc::Receiver<RTCPeerConnectionState>,
    generation: Arc<AtomicU64>,
    encoding: AtomicBool,
    remote_ready: bool,
}

#[allow(clippy::too_many_arguments)]
async fn apply_phone(
    machine: &mut SignalingSession,
    bus: &Bus,
    options: &SenderOptions,
    live: &mut Option<Live>,
    pending_ice: &mut Vec<RTCIceCandidateInit>,
    session: &mut String,
    cancel: &Arc<AtomicBool>,
    now: u64,
    effects: Vec<Effect>,
) -> Result<(), String> {
    let mut queue = VecDeque::from(effects);
    while let Some(effect) = queue.pop_front() {
        match effect {
            Effect::Publish { topic, payload } => {
                bus.publish(options.phone_id, topic, &payload)?;
            }
            Effect::CreateOffer { session: id } => {
                *session = id.clone();
                match open_offer(options, &id).await {
                    Ok((created, sdp)) => {
                        *live = Some(created);
                        queue.extend(machine.note_local_description(&id, &sdp, now));
                    }
                    Err(error) => {
                        eprintln!("fpv-test-sender: offer failed: {error}");
                        queue.extend(machine.note_media(&id, CallState::Failed, now));
                    }
                }
            }
            Effect::ApplyAnswer { sdp, .. } => {
                if let Some(current) = live.as_mut() {
                    let remote =
                        RTCSessionDescription::answer(sdp).map_err(|error| error.to_string())?;
                    current
                        .peer
                        .set_remote_description(remote)
                        .await
                        .map_err(|error| error.to_string())?;
                    current.remote_ready = true;
                    flush_ice(&current.peer, pending_ice).await;
                }
            }
            Effect::AddIce {
                candidate,
                sdp_mid,
                sdp_m_line_index,
                ..
            } => {
                pending_ice.push(candidate_init(candidate, sdp_mid, sdp_m_line_index));
                if let Some(current) = live.as_ref().filter(|current| current.remote_ready) {
                    flush_ice(&current.peer, pending_ice).await;
                }
            }
            Effect::EndOfCandidates { .. } => {}
            Effect::StopMedia { session: id } => {
                pending_ice.clear();
                if let Some(current) = live.take() {
                    current.generation.fetch_add(1, Ordering::Relaxed);
                    let _ = current.peer.close().await;
                    eprintln!("fpv-test-sender: stopped {id}");
                }
            }
            Effect::Show { state, detail, .. } => {
                if state == CallState::Connected {
                    if let Some(current) = live.as_ref() {
                        encoder_once(current, cancel);
                    }
                } else if state == CallState::Failed {
                    eprintln!("fpv-test-sender: {detail}");
                }
            }
            Effect::AcceptOffer { .. } => {}
        }
    }
    Ok(())
}

async fn open_offer(options: &SenderOptions, _session: &str) -> Result<(Live, String), String> {
    let peer = build_peer(&options.ice).await?;
    let track = Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_H264.to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                .into(),
            rtcp_feedback: vec![],
        },
        "fpv".into(),
        "argos-fpv".into(),
    ));
    peer.add_track(track.clone() as Arc<dyn TrackLocal + Send + Sync>)
        .await
        .map_err(|error| error.to_string())?;
    let transceivers = peer.get_transceivers().await;
    if let Some(transceiver) = transceivers.into_iter().next() {
        transceiver
            .set_codec_preferences(preferred_video_codecs())
            .await
            .map_err(|error| error.to_string())?;
        transceiver
            .set_direction(RTCRtpTransceiverDirection::Sendonly)
            .await;
    }
    let (local_ice_tx, local_ice) = std::sync::mpsc::sync_channel(64);
    peer.on_ice_candidate(Box::new(move |candidate| {
        let init = candidate.and_then(|candidate| candidate.to_json().ok());
        let _ = local_ice_tx.try_send(init);
        Box::pin(async {})
    }));
    let (state_tx, states) = std::sync::mpsc::sync_channel(8);
    peer.on_peer_connection_state_change(Box::new(move |state| {
        let _ = state_tx.try_send(state);
        Box::pin(async {})
    }));
    let offer = peer
        .create_offer(None)
        .await
        .map_err(|error| error.to_string())?;
    let sdp = offer.sdp.clone();
    peer.set_local_description(offer)
        .await
        .map_err(|error| error.to_string())?;
    let h264_at = sdp.to_ascii_lowercase().find("h264");
    let vp8_at = sdp.to_ascii_lowercase().find("vp8");
    if h264_at.is_none() || vp8_at.is_some_and(|vp8| h264_at.is_some_and(|h264| vp8 < h264)) {
        return Err("test offer must list H.264 before VP8".into());
    }
    if !sdp.contains("sendonly") {
        return Err("test offer must be sendonly".into());
    }
    eprintln!("fpv-test-sender: local offer ready ({} bytes)", sdp.len());
    Ok((
        Live {
            peer,
            track,
            local_ice,
            states,
            generation: Arc::new(AtomicU64::new(1)),
            encoding: AtomicBool::new(false),
            remote_ready: false,
        },
        sdp,
    ))
}

fn encoder_once(active: &Live, cancel: &Arc<AtomicBool>) {
    if active.encoding.swap(true, Ordering::Relaxed) {
        return;
    }
    let track = active.track.clone();
    let generation = active.generation.clone();
    let seen = generation.load(Ordering::Relaxed);
    let cancel = cancel.clone();
    tokio::spawn(async move {
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(1_200_000))
            .max_frame_rate(FrameRate::from_hz(15.0))
            .skip_frames(false)
            .intra_frame_period(IntraFramePeriod::from_num_frames(8))
            .profile(Profile::Baseline);
        let mut encoder =
            match Encoder::with_api_config(openh264::OpenH264API::from_source(), config) {
                Ok(encoder) => encoder,
                Err(error) => {
                    eprintln!("fpv-test-sender: encoder failed: {error}");
                    return;
                }
            };
        let mut frame = 0u32;
        let mut next = Instant::now();
        loop {
            if cancel.load(Ordering::Relaxed) || generation.load(Ordering::Relaxed) != seen {
                break;
            }
            if frame.is_multiple_of(8) {
                encoder.force_intra_frame();
            }
            let yuv = test_pattern_yuv(frame);
            let encoded = match encoder.encode(&yuv) {
                Ok(encoded) => encoded.to_vec(),
                Err(error) => {
                    eprintln!("fpv-test-sender: encode failed: {error}");
                    break;
                }
            };
            if !encoded.is_empty() {
                let sample = Sample {
                    data: Bytes::from(encoded),
                    duration: FRAME,
                    ..Default::default()
                };
                let _ = track.write_sample(&sample).await;
            }
            frame = frame.wrapping_add(1);
            next += FRAME;
            let now = Instant::now();
            if next > now {
                tokio::time::sleep(next - now).await;
            } else {
                next = now;
            }
        }
    });
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

fn ice_parts(init: Option<RTCIceCandidateInit>) -> (String, String, i32) {
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
) -> RTCIceCandidateInit {
    RTCIceCandidateInit {
        candidate,
        sdp_mid: (!sdp_mid.is_empty()).then_some(sdp_mid),
        sdp_mline_index: u16::try_from(sdp_m_line_index).ok(),
        username_fragment: None,
    }
}

async fn flush_ice(peer: &RTCPeerConnection, pending: &mut Vec<RTCIceCandidateInit>) {
    for init in pending.drain(..) {
        let _ = peer.add_ice_candidate(init).await;
    }
}

fn elapsed_ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}
