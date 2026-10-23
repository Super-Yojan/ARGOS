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
