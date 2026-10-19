/// CPU wall-clock stage timings. `gpu_wait_ms` includes GPU rendering, copy,
/// driver scheduling and synchronization; it is not a hardware timestamp.
#[derive(Clone, Copy, Debug, Default)]
pub struct MapMetrics {
    pub render_ms: f64,
    pub gpu_wait_ms: f64,
    pub pack_ms: f64,
    pub select_ms: f64,
    pub upload_ms: f64,
    pub download_ms: f64,
    pub decode_ms: f64,
    pub delivery_ms: f64,
    pub ui_image_ms: f64,
    pub pending: usize,
    pub resident: usize,
}
