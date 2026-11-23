//! Receive-only rover FPV video over WebRTC.
//!
//! Signaling uses Terra's `fpv-signaling` state machine over the fleet's
//! existing `terra_comms::BaseLink` (the six `webrtc/*` topics). Media is one
//! H.264 video track decoded with OpenH264 into BGRA frames for GPUI.

mod config;
mod decode;
mod depacketize;
mod present;
mod session;

use std::sync::Arc;

pub use config::FpvConfig;
pub use present::{gpu_frame, rotate_frame};
pub use session::{FpvController, FpvPhase, FpvView};

/// Latest decoded picture. Bytes are BGRA.
#[derive(Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Arc<[u8]>,
}
