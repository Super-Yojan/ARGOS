//! Decode one H.264 access unit with OpenH264 and keep only the newest picture.

use std::sync::Arc;

use openh264::formats::YUVSource;

use super::VideoFrame;
use super::depacketize::{H264Depacketizer, timestamp_is_late};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Codec {
    H264,
    Unknown,
}

pub struct VideoDecoder {
    h264_packets: H264Depacketizer,
    h264: openh264::decoder::Decoder,
    codec: Codec,
    pending_ts: Option<u32>,
    nals: Vec<Vec<u8>>,
    last_emitted: Option<u32>,
    /// After a loss or a decode error every later frame depends on missing
    /// data. Skip until an H.264 IDR keyframe arrives.
    need_keyframe: bool,
}

impl VideoDecoder {
    pub fn new() -> Result<Self, String> {
        let h264 = openh264::decoder::Decoder::new().map_err(|error| error.to_string())?;
        Ok(Self {
            h264_packets: H264Depacketizer::default(),
            h264,
            codec: Codec::Unknown,
            pending_ts: None,
            nals: Vec::new(),
            last_emitted: None,
            need_keyframe: true,
        })
    }

    pub fn set_mime(&mut self, mime: &str) {
        let codec = if mime.eq_ignore_ascii_case("video/h264") {
            Codec::H264
        } else {
            Codec::Unknown
        };
        if codec != self.codec {
            self.h264_packets.reset();
            self.nals.clear();
            self.pending_ts = None;
            self.codec = codec;
        }
    }

    /// Start the H.264 decoder over after a decode error. Its reference
    /// pictures are broken by then, so every later P-frame fails too; a fresh
    /// decoder waits for the next keyframe, which the caller asks the phone for.
    pub fn reset_h264(&mut self) -> Result<(), String> {
        self.h264 = openh264::decoder::Decoder::new().map_err(|error| error.to_string())?;
        self.h264_packets.reset();
        self.nals.clear();
        self.pending_ts = None;
        self.need_keyframe = true;
        Ok(())
    }

    pub fn codec_name(&self) -> &'static str {
        match self.codec {
            Codec::H264 => "H.264",
            Codec::Unknown => "video",
        }
    }

    /// True while the decoder waits for a keyframe; the caller asks the
    /// sender for one (RTCP PLI).
    pub fn wants_keyframe(&self) -> bool {
        self.need_keyframe
    }

    /// A packet never arrived. Drop the frame in progress and wait for a keyframe.
    pub fn lost(&mut self) {
        self.h264_packets.reset();
        self.nals.clear();
        self.pending_ts = None;
        self.need_keyframe = true;
    }

    /// `None` when the packet is incomplete, late, or not yet a picture.
    pub fn push(
        &mut self,
        timestamp: u32,
        marker: bool,
        payload: &[u8],
    ) -> Result<Option<VideoFrame>, String> {
        if timestamp_is_late(timestamp, self.last_emitted) {
            return Ok(None);
        }
        let mut ready = None;
        if self.pending_ts.is_some_and(|pending| pending != timestamp) {
            ready = self.finish_access_unit(self.pending_ts.unwrap_or(timestamp))?;
            self.pending_ts = None;
            self.nals.clear();
            self.h264_packets.reset();
        }
        if timestamp_is_late(timestamp, self.last_emitted) {
            return Ok(ready);
        }
        self.pending_ts = Some(timestamp);
        if self.codec == Codec::H264 {
            self.nals.extend(self.h264_packets.push(payload));
            if marker {
                let frame = self.finish_access_unit(timestamp)?;
                self.pending_ts = None;
                self.nals.clear();
                return Ok(frame.or(ready));
            }
        }
        Ok(ready)
    }

    fn finish_access_unit(&mut self, timestamp: u32) -> Result<Option<VideoFrame>, String> {
        if self.nals.is_empty() {
            return Ok(None);
        }
        let mut annex_b = Vec::new();
        for nal in &self.nals {
            annex_b.extend_from_slice(&[0, 0, 0, 1]);
            annex_b.extend_from_slice(nal);
        }
        let has_idr = self
            .nals
            .iter()
            .any(|nal| nal.first().is_some_and(|b| b & 0x1f == 5));
        let nals = std::mem::take(&mut self.nals);
        if self.need_keyframe && !has_idr {
            // Keep parameter sets (SPS 7, PPS 8): a sender may send them apart from the IDR.
            let mut params = Vec::new();
            for nal in nals
                .iter()
                .filter(|nal| nal.first().is_some_and(|b| matches!(b & 0x1f, 7 | 8)))
            {
                params.extend_from_slice(&[0, 0, 0, 1]);
                params.extend_from_slice(nal);
            }
            if !params.is_empty() {
                let _ = self.h264.decode(&params);
            }
            return Ok(None);
        }
        let decoded = match self.h264.decode(&annex_b) {
            Ok(decoded) => decoded,
            Err(error) => {
                // A damaged reference poisons later frames; start clean at the next IDR.
                self.reset_h264()?;
                return Err(format!("{error} (waiting for a key frame)"));
            }
        };
        if has_idr {
            self.need_keyframe = false;
        }
        let Some(picture) = decoded else {
            return Ok(None);
        };
        let (width, height) = picture.dimensions();
        if width == 0 || height == 0 {
            return Ok(None);
        }
        let mut rgba = vec![0u8; width * height * 4];
        picture.write_rgba8(&mut rgba);
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        self.last_emitted = Some(timestamp);
        Ok(Some(VideoFrame {
            width: width as u32,
            height: height as u32,
            bgra: Arc::from(rgba.into_boxed_slice()),
        }))
    }
}
