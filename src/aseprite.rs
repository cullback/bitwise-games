use asefile::AsepriteFile;

use crate::sprite::rgb_to_palette_index;

/// Parse an .aseprite file (frame 0) as a `COLS` × `ROWS` grid of color
/// sprites, each `W` × `H` pixels (row-major). Each pixel becomes a PICO-8
/// palette index 0–15, or `0xFF` if alpha is 0 (transparent). Panics on
/// dimension mismatch or unknown (non-PICO-8) color.
pub fn load_color_grid<const W: usize, const H: usize, const COLS: usize, const COUNT: usize>(
    bytes: &[u8],
) -> [[[u8; W]; H]; COUNT] {
    let ase = AsepriteFile::read(bytes).expect("parse .aseprite");
    let img = ase.frame(0).image();
    let rows = COUNT / COLS;
    assert_eq!(COUNT, COLS * rows, "COUNT must equal COLS * ROWS");
    assert_eq!(img.width() as usize, W * COLS, "width mismatch");
    assert_eq!(img.height() as usize, H * rows, "height mismatch");
    let mut out = [[[0xFFu8; W]; H]; COUNT];
    for (i, sprite) in out.iter_mut().enumerate() {
        let cell_row = i / COLS;
        let cell_col = i % COLS;
        let x0 = (cell_col * W) as u32;
        let y0 = (cell_row * H) as u32;
        for (y, row) in sprite.iter_mut().enumerate() {
            for (x, px) in row.iter_mut().enumerate() {
                let p = img.get_pixel(x0 + x as u32, y0 + y as u32);
                if p.0[3] == 0 {
                    *px = 0xFF;
                } else {
                    let [r, g, b, _a] = p.0;
                    *px = rgb_to_palette_index(r, g, b).unwrap_or_else(|| {
                        panic!(
                            "unknown color ({},{},{}) at cell {} pixel ({},{})",
                            r, g, b, i, x, y
                        )
                    });
                }
            }
        }
    }
    out
}

/// Parse an .aseprite file (frame 0) as a `COLS` × `ROWS` grid of glyphs, each
/// `W` × `H` pixels (row-major). Returns one bit-row per scanline (MSB =
/// leftmost pixel). Any pixel with alpha > 0 is "on". `COUNT` must equal
/// `COLS * ROWS` (passed separately because Rust const-generic arithmetic
/// isn't stable).
pub fn load_glyph_grid<const W: u32, const H: usize, const COLS: u32, const COUNT: usize>(
    bytes: &[u8],
) -> [[u16; H]; COUNT] {
    assert!(W <= 16, "glyph width must fit in u16");
    let ase = AsepriteFile::read(bytes).expect("parse .aseprite");
    let img = ase.frame(0).image();
    let rows = COUNT as u32 / COLS;
    assert_eq!(COUNT as u32, COLS * rows, "COUNT must equal COLS * ROWS");
    assert_eq!(
        img.width(),
        W * COLS,
        "width {} doesn't match W*COLS = {}",
        img.width(),
        W * COLS
    );
    assert_eq!(
        img.height(),
        H as u32 * rows,
        "height {} doesn't match H*ROWS = {}",
        img.height(),
        H as u32 * rows,
    );
    let mut out = [[0u16; H]; COUNT];
    for (i, glyph) in out.iter_mut().enumerate() {
        let cell_row = i as u32 / COLS;
        let cell_col = i as u32 % COLS;
        let x0 = cell_col * W;
        let y0 = cell_row * H as u32;
        for (y, row) in glyph.iter_mut().enumerate() {
            let mut bits: u16 = 0;
            for x in 0..W {
                let p = img.get_pixel(x0 + x, y0 + y as u32);
                if p.0[3] > 0 {
                    bits |= 1 << (W - 1 - x);
                }
            }
            *row = bits;
        }
    }
    out
}

/// Parse an .aseprite file (frame 0) as a horizontal strip of `COUNT` glyphs,
/// each `W` × `H` pixels. Returns one bit-row per scanline (MSB = leftmost
/// pixel). Any pixel with alpha > 0 is "on".
///
/// `W` must be ≤ 16. The image must be exactly `W * COUNT` wide and `H` tall.
/// Panics on dimension mismatch or parse failure — assets are source files,
/// not user input.
pub fn load_glyph_strip<const W: u32, const H: usize, const COUNT: usize>(
    bytes: &[u8],
) -> [[u16; H]; COUNT] {
    assert!(W <= 16, "glyph width must fit in u16");
    let ase = AsepriteFile::read(bytes).expect("parse .aseprite");
    let img = ase.frame(0).image();
    assert_eq!(
        img.width(),
        W * COUNT as u32,
        "expected width {} ({}×{}), got {}",
        W * COUNT as u32,
        W,
        COUNT,
        img.width()
    );
    assert_eq!(
        img.height() as usize,
        H,
        "expected height {}, got {}",
        H,
        img.height()
    );
    let mut out = [[0u16; H]; COUNT];
    for (i, glyph) in out.iter_mut().enumerate() {
        let x0 = i as u32 * W;
        for (y, row) in glyph.iter_mut().enumerate() {
            let mut bits: u16 = 0;
            for x in 0..W {
                let p = img.get_pixel(x0 + x, y as u32);
                if p.0[3] > 0 {
                    bits |= 1 << (W - 1 - x);
                }
            }
            *row = bits;
        }
    }
    out
}
