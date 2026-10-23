//! Enough of libvpx to decode one VP8 picture.
//!
//! `env-libvpx-sys` 5.1.3 has no pregenerated binding for libvpx 1.14. The
//! decoder ABI this file uses is 12, which is `VPX_DECODER_ABI_VERSION` for
//! libvpx 1.13 and 1.14 (`VPX_IMAGE_ABI_VERSION` 5).

use std::os::raw::{c_char, c_int, c_long, c_uint, c_void};

const VPX_DECODER_ABI_VERSION: c_int = 12;

#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum VpxCodecErr {
    Ok = 0,
    Error = 1,
    MemError = 2,
    AbiMismatch = 3,
    Incapable = 4,
    UnsupBitstream = 5,
    UnsupFeature = 6,
    CorruptFrame = 7,
    InvalidParam = 8,
    ListEnd = 9,
}

#[repr(C)]
struct VpxIface {
    _private: [u8; 0],
}

#[repr(C)]
struct VpxPriv {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VpxDecCfg {
    pub threads: c_uint,
    pub w: c_uint,
    pub h: c_uint,
}

#[repr(C)]
union VpxConfig {
    dec: *const VpxDecCfg,
    raw: *const c_void,
}

#[repr(C)]
pub struct VpxCodecCtx {
    name: *const c_char,
    iface: *const VpxIface,
    err: VpxCodecErr,
    err_detail: *const c_char,
    init_flags: c_long,
    config: VpxConfig,
    priv_: *mut VpxPriv,
}

#[repr(C)]
pub struct VpxImage {
    pub fmt: u32,
    pub cs: u32,
    pub range: u32,
    pub w: c_uint,
    pub h: c_uint,
    pub bit_depth: c_uint,
    pub d_w: c_uint,
    pub d_h: c_uint,
    pub r_w: c_uint,
    pub r_h: c_uint,
    pub x_chroma_shift: c_uint,
    pub y_chroma_shift: c_uint,
    pub planes: [*mut u8; 4],
    pub stride: [c_int; 4],
    pub bps: c_int,
    pub user_priv: *mut c_void,
    pub img_data: *mut u8,
    pub img_data_owner: c_int,
    pub self_allocd: c_int,
    pub fb_priv: *mut c_void,
}

#[link(name = "vpx")]
unsafe extern "C" {
    fn vpx_codec_vp8_dx() -> *const VpxIface;
    fn vpx_codec_dec_init_ver(
        ctx: *mut VpxCodecCtx,
        iface: *const VpxIface,
        cfg: *const VpxDecCfg,
        flags: c_long,
        ver: c_int,
    ) -> VpxCodecErr;
    fn vpx_codec_decode(
        ctx: *mut VpxCodecCtx,
        data: *const u8,
        data_sz: c_uint,
        user_priv: *mut c_void,
        deadline: c_long,
    ) -> VpxCodecErr;
    fn vpx_codec_get_frame(ctx: *mut VpxCodecCtx, iter: *mut *const c_void) -> *mut VpxImage;
    fn vpx_codec_destroy(ctx: *mut VpxCodecCtx) -> VpxCodecErr;
}

pub struct Vp8Decoder {
    ctx: VpxCodecCtx,
}

impl Vp8Decoder {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let mut ctx = std::mem::zeroed();
            let config = VpxDecCfg {
                threads: 1,
                w: 0,
                h: 0,
            };
            let err = vpx_codec_dec_init_ver(
                &mut ctx,
                vpx_codec_vp8_dx(),
                &config,
                0,
                VPX_DECODER_ABI_VERSION,
            );
            if err != VpxCodecErr::Ok {
                return Err(format!("VP8 decoder init failed ({err:?})"));
            }
            Ok(Self { ctx })
        }
    }

    pub fn decode(&mut self, frame: &[u8]) -> Result<Option<&VpxImage>, String> {
        unsafe {
            let err = vpx_codec_decode(
                &mut self.ctx,
                frame.as_ptr(),
                frame.len() as c_uint,
                std::ptr::null_mut(),
                0,
            );
            if err != VpxCodecErr::Ok {
                return Err(format!("VP8 decode failed ({err:?})"));
            }
            let mut iter = std::ptr::null();
            let image = vpx_codec_get_frame(&mut self.ctx, &mut iter);
            if image.is_null() {
                return Ok(None);
            }
            Ok(Some(&*image))
        }
    }
}

unsafe impl Send for Vp8Decoder {}

impl Drop for Vp8Decoder {
    fn drop(&mut self) {
        unsafe {
            vpx_codec_destroy(&mut self.ctx);
        }
    }
}
