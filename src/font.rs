use crate::draw_command::{Color, DrawCommand};

pub const GLYPH_W: u32 = 3;
pub const GLYPH_H: u32 = 5;

/// 3x5 glyph patterns for digits 0–9, uppercase A–Z, and `=`, MSB =
/// leftmost pixel. Returns a blank pattern for unknown bytes (including
/// spaces).
pub fn glyph(ch: u8) -> [u8; 5] {
    match ch {
        b'0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        b'1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        b'2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        b'3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        b'4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        b'5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        b'6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        b'7' => [0b111, 0b001, 0b001, 0b001, 0b001],
        b'8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        b'9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        b'A' => [0b010, 0b101, 0b111, 0b101, 0b101],
        b'B' => [0b110, 0b101, 0b110, 0b101, 0b110],
        b'C' => [0b011, 0b100, 0b100, 0b100, 0b011],
        b'D' => [0b110, 0b101, 0b101, 0b101, 0b110],
        b'E' => [0b111, 0b100, 0b110, 0b100, 0b111],
        b'F' => [0b111, 0b100, 0b110, 0b100, 0b100],
        b'G' => [0b011, 0b100, 0b101, 0b101, 0b011],
        b'H' => [0b101, 0b101, 0b111, 0b101, 0b101],
        b'I' => [0b111, 0b010, 0b010, 0b010, 0b111],
        b'J' => [0b001, 0b001, 0b001, 0b101, 0b010],
        b'K' => [0b101, 0b110, 0b100, 0b110, 0b101],
        b'L' => [0b100, 0b100, 0b100, 0b100, 0b111],
        b'M' => [0b101, 0b111, 0b111, 0b101, 0b101],
        b'N' => [0b110, 0b101, 0b101, 0b101, 0b101],
        b'O' => [0b111, 0b101, 0b101, 0b101, 0b111],
        b'P' => [0b110, 0b101, 0b110, 0b100, 0b100],
        b'Q' => [0b111, 0b101, 0b101, 0b110, 0b011],
        b'R' => [0b110, 0b101, 0b110, 0b101, 0b101],
        b'S' => [0b011, 0b100, 0b010, 0b001, 0b110],
        b'T' => [0b111, 0b010, 0b010, 0b010, 0b010],
        b'U' => [0b101, 0b101, 0b101, 0b101, 0b111],
        b'V' => [0b101, 0b101, 0b101, 0b101, 0b010],
        b'W' => [0b101, 0b101, 0b111, 0b111, 0b101],
        b'X' => [0b101, 0b101, 0b010, 0b101, 0b101],
        b'Y' => [0b101, 0b101, 0b010, 0b010, 0b010],
        b'Z' => [0b111, 0b001, 0b010, 0b100, 0b111],
        b'=' => [0b000, 0b111, 0b000, 0b111, 0b000],
        _ => [0; 5],
    }
}

pub fn draw_glyph(
    commands: &mut Vec<DrawCommand>,
    ch: u8,
    x: u32,
    y: u32,
    scale: u32,
    color: Color,
) {
    let pattern = glyph(ch);
    for (row, bits) in pattern.iter().enumerate() {
        for col in 0..GLYPH_W {
            if (bits >> (GLYPH_W - 1 - col)) & 1 == 1 {
                commands.push(DrawCommand::rect(
                    x + col * scale,
                    y + row as u32 * scale,
                    scale,
                    scale,
                    color,
                ));
            }
        }
    }
}

pub fn draw_text(
    commands: &mut Vec<DrawCommand>,
    text: &[u8],
    x: u32,
    y: u32,
    scale: u32,
    color: Color,
) {
    let glyph_w = GLYPH_W * scale;
    let gap = scale;
    for (i, &ch) in text.iter().enumerate() {
        draw_glyph(
            commands,
            ch,
            x + i as u32 * (glyph_w + gap),
            y,
            scale,
            color,
        );
    }
}

pub fn text_width(len: usize, scale: u32) -> u32 {
    let len = len as u32;
    if len == 0 {
        0
    } else {
        len * GLYPH_W * scale + (len - 1) * scale
    }
}

/// Decimal digits of `n` as ASCII bytes (b'0'..=b'9'), most-significant first.
/// `n == 0` yields `[b'0']`.
pub fn digits_of(n: u32) -> Vec<u8> {
    if n == 0 {
        return vec![b'0'];
    }
    let mut out = Vec::new();
    let mut n = n;
    while n > 0 {
        out.push(b'0' + (n % 10) as u8);
        n /= 10;
    }
    out.reverse();
    out
}
