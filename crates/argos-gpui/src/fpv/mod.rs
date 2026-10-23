//! Receive-only rover camera.
//!
//! Signaling is Terra's `fpv-signaling` crate. This module subscribes on the
//! fleet router, feeds samples to `SignalingSession`, and publishes the effects
//! it returns. Media is one video track, H.264 then VP8. This module never
//! publishes teleop, hardware, safety, autonomy, or goal payloads, and it does
//! not open a data channel.

mod config;
mod decode;
mod depacketize;
mod link;
mod pattern;
mod present;
mod sender;
mod session;
mod vpx;

use std::sync::Arc;

pub use config::FpvConfig;
pub use present::gpu_frame;
pub use sender::{SenderOptions, run_test_sender};
pub use session::{FpvController, FpvPhase, FpvView};

/// Latest decoded picture. Bytes are BGRA.
#[derive(Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Arc<[u8]>,
}

#[cfg(test)]
#[path = "../../../../tests/argos-gpui/unit/fpv_tests.rs"]
mod tests;
