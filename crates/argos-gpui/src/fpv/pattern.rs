//! Moving color bars so a local sender can prove the decode path.

use openh264::formats::{RgbSliceU8, YUVBuffer};

pub const TEST_PATTERN_WIDTH: u32 = 640;
pub const TEST_PATTERN_HEIGHT: u32 = 360;

pub fn test_pattern_yuv(frame: u32) -> YUVBuffer {
    let width = TEST_PATTERN_WIDTH as usize;
    let height = TEST_PATTERN_HEIGHT as usize;
    let mut rgb = vec![0u8; width * height * 3];
    let bars: [[u8; 3]; 8] = [
        [235, 235, 235],
        [235, 235, 16],
        [16, 235, 235],
        [16, 235, 16],
        [235, 16, 235],
        [235, 16, 16],
        [16, 16, 235],
        [16, 16, 16],
    ];
    let square_x = ((frame as usize) * 12) % (width - 64);
    let square_y = 48 + ((frame as usize) * 5) % (height - 112);
    for y in 0..height {
        for x in 0..width {
            let mut color = bars[(x / 80) % bars.len()];
            if (square_x..square_x + 64).contains(&x) && (square_y..square_y + 64).contains(&y) {
                color = [255, 96, 16];
            }
            if y < 20 && x < 320 {
                let bit = (frame >> (x / 20)) & 1;
                if bit == 1 {
                    color = [250, 250, 250];
                }
            }
            let index = (y * width + x) * 3;
            rgb[index] = color[0];
            rgb[index + 1] = color[1];
            rgb[index + 2] = color[2];
        }
    }
    YUVBuffer::from_rgb8_source(RgbSliceU8::new(&rgb, (width, height)))
}
