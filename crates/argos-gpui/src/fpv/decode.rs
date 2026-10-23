//! Decode one video access unit and keep only the newest picture.

use openh264::formats::YUVSource;

use super::VideoFrame;
use super::depacketize::{H264Depacketizer, Vp8Depacketizer, timestamp_is_late};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Codec {
    H264,
    Vp8,
    Unknown,
}

pub struct VideoDecoder {
    h264_packets: H264Depacketizer,
    h264: openh264::decoder::Decoder,
    vp8_packets: Vp8Depacketizer,
    vp8: Option<super::vpx::Vp8Decoder>,
    codec: Codec,
    pending_ts: Option<u32>,
    nals: Vec<Vec<u8>>,
    last_emitted: Option<u32>,
}

impl VideoDecoder {
    pub fn new() -> Result<Self, String> {
        let h264 = openh264::decoder::Decoder::new().map_err(|error| error.to_string())?;
        let vp8 = super::vpx::Vp8Decoder::new().ok();
        Ok(Self {
            h264_packets: H264Depacketizer::default(),
            h264,
            vp8_packets: Vp8Depacketizer::default(),
            vp8,
            codec: Codec::Unknown,
            pending_ts: None,
            nals: Vec::new(),
            last_emitted: None,
        })
    }

    pub fn set_mime(&mut self, mime: &str) {
        let codec = if mime.eq_ignore_ascii_case("video/h264") {
            Codec::H264
        } else if mime.eq_ignore_ascii_case("video/vp8") {
            Codec::Vp8
        } else {
            Codec::Unknown
        };
        if codec != self.codec {
            self.h264_packets.reset();
            self.vp8_packets.reset();
            self.nals.clear();
            self.pending_ts = None;
            self.codec = codec;
        }
    }

    pub fn codec_name(&self) -> &'static str {
        match self.codec {
            Codec::H264 => "H.264",
            Codec::Vp8 => "VP8",
            Codec::Unknown => "video",
        }
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
            self.vp8_packets.reset();
        }
        if timestamp_is_late(timestamp, self.last_emitted) {
            return Ok(ready);
        }
        self.pending_ts = Some(timestamp);
        match self.codec {
            Codec::H264 => {
                self.nals.extend(self.h264_packets.push(payload));
                if marker {
                    let frame = self.finish_access_unit(timestamp)?;
                    self.pending_ts = None;
                    self.nals.clear();
                    return Ok(frame.or(ready));
                }
            }
            Codec::Vp8 => {
                if let Some(frame) = self.vp8_packets.push(payload, marker) {
                    self.pending_ts = None;
                    return Ok(self.decode_vp8(timestamp, &frame)?.or(ready));
                }
            }
            Codec::Unknown => {}
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
        self.nals.clear();
        let decoded = self
            .h264
            .decode(&annex_b)
            .map_err(|error| error.to_string())?;
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

    fn decode_vp8(&mut self, timestamp: u32, frame: &[u8]) -> Result<Option<VideoFrame>, String> {
        let Some(decoder) = self.vp8.as_mut() else {
            return Err("VP8 was negotiated but libvpx is unavailable".into());
        };
        let decoded = {
            let Some(picture) = decoder.decode(frame)? else {
                return Ok(None);
            };
            let width = picture.d_w as usize;
            let height = picture.d_h as usize;
            if width == 0 || height == 0 {
                return Ok(None);
            }
            let y_stride = picture.stride[0] as usize;
            let u_stride = picture.stride[1] as usize;
            let v_stride = picture.stride[2] as usize;
            if picture.planes[0].is_null()
                || picture.planes[1].is_null()
                || picture.planes[2].is_null()
            {
                return Ok(None);
            }
            let y = unsafe {
                std::slice::from_raw_parts(picture.planes[0], y_stride.saturating_mul(height))
            };
            let chroma_rows = height.div_ceil(2);
            let u = unsafe {
                std::slice::from_raw_parts(picture.planes[1], u_stride.saturating_mul(chroma_rows))
            };
            let v = unsafe {
                std::slice::from_raw_parts(picture.planes[2], v_stride.saturating_mul(chroma_rows))
            };
            let mut bgra = vec![0u8; width * height * 4];
            i420_to_bgra(
                y, y_stride, u, u_stride, v, v_stride, width, height, &mut bgra,
            );
            VideoFrame {
                width: width as u32,
                height: height as u32,
                bgra: Arc::from(bgra.into_boxed_slice()),
            }
        };
        self.last_emitted = Some(timestamp);
        Ok(Some(decoded))
    }
}

#[allow(clippy::too_many_arguments)]
fn i420_to_bgra(
    y: &[u8],
    y_stride: usize,
    u: &[u8],
    u_stride: usize,
    v: &[u8],
    v_stride: usize,
    width: usize,
    height: usize,
    out: &mut [u8],
) {
    for row in 0..height {
        for col in 0..width {
            let luma = y.get(row * y_stride + col).copied().unwrap_or(0) as i32;
            let chroma_u = u
                .get((row / 2) * u_stride + col / 2)
                .copied()
                .unwrap_or(128) as i32;
            let chroma_v = v
                .get((row / 2) * v_stride + col / 2)
                .copied()
                .unwrap_or(128) as i32;
            let c = luma - 16;
            let d = chroma_u - 128;
            let e = chroma_v - 128;
            let red = (298 * c + 409 * e + 128) >> 8;
            let green = (298 * c - 100 * d - 208 * e + 128) >> 8;
            let blue = (298 * c + 516 * d + 128) >> 8;
            let index = (row * width + col) * 4;
            out[index] = blue.clamp(0, 255) as u8;
            out[index + 1] = green.clamp(0, 255) as u8;
            out[index + 2] = red.clamp(0, 255) as u8;
            out[index + 3] = 255;
        }
    }
}
