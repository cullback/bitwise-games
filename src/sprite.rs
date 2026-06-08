use crate::draw_command::Color;
use crate::frame_buffer::{FrameBuffer, HEIGHT, WIDTH};

/// Orientation transform — the 4 rotations plus a horizontal mirror.
/// `FlipH` mirrors around the vertical axis (left↔right). Useful for sprites
/// with top/bottom asymmetric features (like a head with eyes near the top)
/// where R180 would otherwise put the asymmetric feature upside down.
#[derive(Copy, Clone, Debug)]
pub enum Rot {
    R0,
    R90,
    R180,
    R270,
    FlipH,
}

/// Map a PICO-8 RGB color back to its palette index. `None` for unknown.
pub fn rgb_to_palette_index(r: u8, g: u8, b: u8) -> Option<u8> {
    Some(match (r, g, b) {
        (0, 0, 0) => 0,
        (29, 43, 83) => 1,
        (126, 37, 83) => 2,
        (0, 135, 81) => 3,
        (171, 82, 54) => 4,
        (95, 87, 79) => 5,
        (194, 195, 199) => 6,
        (255, 241, 232) => 7,
        (255, 0, 77) => 8,
        (255, 163, 0) => 9,
        (255, 236, 39) => 10,
        (0, 228, 54) => 11,
        (41, 173, 255) => 12,
        (131, 118, 156) => 13,
        (255, 119, 168) => 14,
        (255, 204, 170) => 15,
        _ => return None,
    })
}

/// Blit a square sprite stored as palette indices, rotated CW by `rot`.
/// Index `> 15` is transparent. Pixels outside the framebuffer are clipped.
pub fn blit_square<const N: usize>(
    fb: &mut FrameBuffer,
    pixels: &[[u8; N]; N],
    x: u32,
    y: u32,
    rot: Rot,
) {
    for r in 0..N {
        for c in 0..N {
            let (sr, sc) = match rot {
                Rot::R0 => (r, c),
                Rot::R90 => (N - 1 - c, r),
                Rot::R180 => (N - 1 - r, N - 1 - c),
                Rot::R270 => (c, N - 1 - r),
                Rot::FlipH => (r, N - 1 - c),
            };
            let idx = pixels[sr][sc];
            if idx > 15 {
                continue;
            }
            let dx = x + c as u32;
            let dy = y + r as u32;
            if dx < WIDTH && dy < HEIGHT {
                fb.pixels[(dy * WIDTH + dx) as usize] = PALETTE[idx as usize];
            }
        }
    }
}

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
