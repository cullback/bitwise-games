use crate::draw_command::Color;
use crate::frame_buffer::{FrameBuffer, HEIGHT, WIDTH};

/// Pixel value meaning "skip this pixel". Anything > 15 is treated the same
/// way, but use this constant in sprite data for clarity.
pub const TRANSPARENT: u8 = 0xFF;

/// Bit-packed sprite. Pixel values 0..=15 are palette indices (matching
/// `Color` discriminants); any value > 15 is transparent. Pixels are flat
/// row-major, frames concatenated: `pixels[(f * height + y) * width + x]`.
#[derive(Copy, Clone, Debug)]
pub struct Sprite {
    pub width: u8,
    pub height: u8,
    pub frames: u8,
    pub pixels: &'static [u8],
}

const PALETTE: [Color; 16] = [
    Color::Black,
    Color::DarkBlue,
    Color::DarkPurple,
    Color::DarkGreen,
    Color::Brown,
    Color::DarkGrey,
    Color::LightGrey,
    Color::White,
    Color::Red,
    Color::Orange,
    Color::Yellow,
    Color::Green,
    Color::Blue,
    Color::Lavender,
    Color::Pink,
    Color::LightPeach,
];

impl FrameBuffer {
    /// Stamp one frame of `sprite` at framebuffer coords `(x, y)`. Pixels
    /// outside the framebuffer are clipped. Palette values > 15 are skipped.
    pub fn blit(&mut self, sprite: &Sprite, frame: u8, x: i32, y: i32) {
        let w = sprite.width as usize;
        let h = sprite.height as usize;
        let frame_off = (frame as usize) * w * h;
        for row in 0..h {
            for col in 0..w {
                let px = sprite.pixels[frame_off + row * w + col];
                if px > 15 {
                    continue;
                }
                let dst_x = x + col as i32;
                let dst_y = y + row as i32;
                if dst_x < 0 || dst_y < 0 {
                    continue;
                }
                let dx = dst_x as u32;
                let dy = dst_y as u32;
                if dx >= WIDTH || dy >= HEIGHT {
                    continue;
                }
                self.pixels[(dy * WIDTH + dx) as usize] = PALETTE[px as usize];
            }
        }
    }
}
