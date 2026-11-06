//! BGRA frames for the GPUI image element. The buffer is already BGRA.

use super::VideoFrame;
use gpui_kit::RenderImage;
use smallvec::smallvec;
use std::sync::Arc;

pub fn gpu_frame(frame: &VideoFrame) -> Option<Arc<RenderImage>> {
    let bytes = frame.bgra.as_ref().to_vec();
    let image = image::RgbaImage::from_raw(frame.width, frame.height, bytes)?;
    Some(Arc::new(RenderImage::new(smallvec![image::Frame::new(
        image
    )])))
}

/// Turn a frame clockwise by `quarter_turns` × 90°. A phone mounted sideways
/// sends its picture on its side; the operator turns it upright here.
pub fn rotate_frame(frame: &VideoFrame, quarter_turns: u8) -> VideoFrame {
    let turns = quarter_turns % 4;
    if turns == 0 {
        return frame.clone();
    }
    let (w, h) = (frame.width as usize, frame.height as usize);
    let src = frame.bgra.as_ref();
    if src.len() < w * h * 4 {
        return frame.clone();
    }
    let (out_w, out_h) = if turns == 2 { (w, h) } else { (h, w) };
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = match turns {
                1 => (h - 1 - y, x),
                2 => (w - 1 - x, h - 1 - y),
                _ => (y, w - 1 - x),
            };
            let from = (y * w + x) * 4;
            let to = (ny * out_w + nx) * 4;
            out[to..to + 4].copy_from_slice(&src[from..from + 4]);
        }
    }
    VideoFrame {
        width: out_w as u32,
        height: out_h as u32,
        bgra: out.into(),
    }
}
