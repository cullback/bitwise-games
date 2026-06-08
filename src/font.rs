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

/// Parse a bitmap row written as ASCII: `X` = on, anything else = off. MSB =
/// leftmost. Designed for use in `const` initializers via `bitmap!`.
pub const fn bitmap_row(s: &[u8]) -> u16 {
    let mut bits: u16 = 0;
    let mut i = 0;
    while i < s.len() {
        bits = (bits << 1) | ((s[i] == b'X') as u16);
        i += 1;
    }
    bits
}

/// Declarative bitmap literal. Each row is a string of `X` and `.`.
/// Expands to an array of `u16` (MSB = leftmost pixel).
macro_rules! bitmap {
    [$($row:literal),* $(,)?] => {
        [$($crate::font::bitmap_row($row.as_bytes())),*]
    };
}

pub const BIG_GLYPH_W: u32 = 9;
pub const BIG_GLYPH_H: u32 = 11;

const BIG_DIGITS: [[u16; BIG_GLYPH_H as usize]; 10] = [
    bitmap![
        "..XXXXX..",
        ".XXXXXXX.",
        "XXXXXXXXX",
        "XXX...XXX",
        "XXX...XXX",
        "XXX...XXX",
        "XXX...XXX",
        "XXX...XXX",
        "XXXXXXXXX",
        ".XXXXXXX.",
        "..XXXXX..",
    ],
    bitmap![
        "...XXX...",
        "..XXXX...",
        ".XXXXX...",
        "XXXXXX...",
        "...XXX...",
        "...XXX...",
        "...XXX...",
        "...XXX...",
        "XXXXXXXXX",
        "XXXXXXXXX",
        "XXXXXXXXX",
    ],
    bitmap![
        ".XXXXXXX.",
        "XXXXXXXXX",
        "XX....XXX",
        "......XXX",
        ".....XXXX",
        "...XXX...",
        ".XXXX....",
        "XXXX.....",
        "XXX......",
        "XXXXXXXXX",
        "XXXXXXXXX",
    ],
    bitmap![
        ".XXXXXXX.",
        "XX.....XX",
        ".......XX",
        ".......XX",
        "....XXXXX",
        "....XXXXX",
        ".......XX",
        ".......XX",
        ".......XX",
        "XX.....XX",
        ".XXXXXXX.",
    ],
    bitmap![
        "......XX.",
        ".....XXX.",
        "....X.XX.",
        "...X..XX.",
        "..X...XX.",
        ".X....XX.",
        "X.....XX.",
        "XXXXXXXXX",
        "XXXXXXXXX",
        "......XX.",
        "......XX.",
    ],
    bitmap![
        "XXXXXXXXX",
        "XXXXXXXXX",
        "XX.......",
        "XX.......",
        "XXXXXXXXX",
        "XXXXXXXXX",
        "XX.....XX",
        "XX.....XX",
        "XX.....XX",
        "XXXXXXXXX",
        ".XXXXXXX.",
    ],
    bitmap![
        "XX.......",
        "XX.......",
        "XX.......",
        "XX.......",
        "XXXXXXXXX",
        "XXXXXXXXX",
        "XX.....XX",
        "XX.....XX",
        "XX.....XX",
        "XXXXXXXXX",
        ".XXXXXXX.",
    ],
    bitmap![
        "XXXXXXXXX",
        "XXXXXXXXX",
        ".......XX",
        "......XX.",
        ".....XX..",
        "....XX...",
        "....XX...",
        "....XX...",
        "....XX...",
        "....XX...",
        "....XX...",
    ],
    bitmap![
        ".XXXXXXX.",
        "XXXXXXXXX",
        "XX.....XX",
        "XX.....XX",
        "XXXXXXXXX",
        ".XXXXXXX.",
        "XXXXXXXXX",
        "XX.....XX",
        "XX.....XX",
        "XXXXXXXXX",
        ".XXXXXXX.",
    ],
    bitmap![
        ".XXXXXXX.",
        "XXXXXXXXX",
        "XX.....XX",
        "XX.....XX",
        "XX.....XX",
        "XXXXXXXXX",
        "XXXXXXXXX",
        ".......XX",
        ".......XX",
        ".......XX",
        ".......XX",
    ],
];

pub fn draw_big_digit(commands: &mut Vec<DrawCommand>, ch: u8, x: u32, y: u32, color: Color) {
    if !ch.is_ascii_digit() {
        return;
    }
    let pattern = &BIG_DIGITS[(ch - b'0') as usize];
    for (row, &bits) in pattern.iter().enumerate() {
        for col in 0..BIG_GLYPH_W {
            if (bits >> (BIG_GLYPH_W - 1 - col)) & 1 == 1 {
                commands.push(DrawCommand::rect(x + col, y + row as u32, 1, 1, color));
            }
        }
    }
}

pub fn draw_big_text(commands: &mut Vec<DrawCommand>, text: &[u8], x: u32, y: u32, color: Color) {
    let gap = 1u32;
    for (i, &ch) in text.iter().enumerate() {
        draw_big_digit(commands, ch, x + i as u32 * (BIG_GLYPH_W + gap), y, color);
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

/// 5×3 alphanum font loaded from `assets/5x3-alphanum.aseprite`. Each cell
/// is 4×6 (3×5 ink + 1 px gutter on the right and bottom), so chars blit
/// side-by-side with automatic letter-spacing and line-spacing. Glyphs are
/// laid out in a 16×4 grid in row-major order: digits `0`–`9` then letters
/// `A`–`Z` (36 glyphs total; the last row is unused).
pub mod tiny {
    use crate::aseprite::load_glyph_grid;
    use crate::draw_command::{Color, DrawCommand};
    use std::sync::OnceLock;

    pub const CHAR_W: u32 = 4;
    pub const CHAR_H: u32 = 6;
    const COLS: u32 = 16;
    const COUNT: usize = 64;

    static GLYPHS: OnceLock<[[u16; CHAR_H as usize]; COUNT]> = OnceLock::new();

    fn glyphs() -> &'static [[u16; CHAR_H as usize]; COUNT] {
        GLYPHS.get_or_init(|| {
            load_glyph_grid::<{ CHAR_W }, { CHAR_H as usize }, COLS, COUNT>(include_bytes!(
                "../assets/5x3-alphanum.aseprite"
            ))
        })
    }

    /// Map ASCII byte to glyph index: digits `0`–`9` → `0`–`9`,
    /// letters `A`–`Z` → `10`–`35`. Unsupported bytes (including space)
    /// draw nothing, but `draw_text` still advances the cursor so they
    /// render as blank cells in the layout.
    pub fn char_index(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'A'..=b'Z' => Some(10 + (c - b'A')),
            _ => None,
        }
    }

    pub fn draw_char(
        commands: &mut Vec<DrawCommand>,
        idx: u8,
        x: u32,
        y: u32,
        scale: u32,
        color: Color,
    ) {
        let pattern = &glyphs()[idx as usize];
        for (row, &bits) in pattern.iter().enumerate() {
            for col in 0..CHAR_W {
                if (bits >> (CHAR_W - 1 - col)) & 1 == 1 {
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

    /// Draw `text` at `(x, y)` scaled by `scale` (1 = native 4 px stride,
    /// 2 = 8 px stride, etc.). Unsupported chars are silent gaps so layout
    /// stays predictable.
    pub fn draw_text(
        commands: &mut Vec<DrawCommand>,
        text: &[u8],
        x: u32,
        y: u32,
        scale: u32,
        color: Color,
    ) {
        for (i, &c) in text.iter().enumerate() {
            if let Some(idx) = char_index(c) {
                draw_char(
                    commands,
                    idx,
                    x + i as u32 * CHAR_W * scale,
                    y,
                    scale,
                    color,
                );
            }
        }
    }
}
