/*

Connect Four — two-player local hot-seat on the classic 6×7 board. Every
byte of game state fits in a single u64.

The board's visual design is lifted from `connect4.aseprite` — a hand-drawn
PICO-8 template that we load once and stamp as the background each frame.
Cell holes, bevels, and the "slot opening" at the top of every column come
straight from the sprite. We overlay disc colours on top.

# Inputs

- Mouse: hover picks the column for the next drop. A floating 14×14 disc
  with a BLACK outline appears in the active player's colour above the
  hovered column (or no preview if the column is full).
- Z: drop a disc in the hovered column (post-game it doubles as "rematch").
- X: reset to a fresh empty board (works mid-game or post-game).

# Turn-taking

Whose turn = parity of the total piece count.
  - 0 (or even) pieces dropped → red (P1, who always drops first)
  - odd → yellow (P2)

# Encoding (64 bits)

| Bits   | Field                                          |
| ------ | ---------------------------------------------- |
| 0..21  | 7 column heights × 3 bits each (low col first) |
| 21..63 | per-cell color, row-major top-down (1 bit)     |
| 63     | unused                                         |

Color bit at index `21 + row*7 + col`:
  - 1 = red (P1)
  - 0 = yellow (P2)

Only meaningful for cells under the stack (`row >= ROWS - column_height(col)`).
Cells above the stack must hold 0, an invariant maintained by `drop_disc`:
the bit starts at 0 and is only ever written when stacking up from the bottom.

# Layout (128 × 128, matching the template)

  y =   0..20    preview row — DARK_GREY "table". When playing, a 14×14
                 disc in the active player's colour hovers above the
                 column the mouse is over.
  y =  20..120   board frame, blitted from the .aseprite template.
                 6 rows × 7 cols of recessed circular holes, each 15×15
                 in a 16-pitch grid (left bevel: DARK_BLUE shadow on the
                 upper-left rim, LIGHT_PEACH highlight on the lower-right
                 rim — reads as a recess lit from below-right).
  y = 120..128   DARK_GREY ground.

A status banner appears centred over the board when the game ends.

*/

use bitwise_games::aseprite::load_color_grid;
use bitwise_games::draw_command::{BLACK, Color, DARK_GREY, DrawCommand, RED, WHITE, YELLOW};
use bitwise_games::font::{GLYPH_H, draw_text, text_width};
use bitwise_games::frame_buffer::{self, FrameBuffer};
use bitwise_games::sprite::{Rot, TRANSPARENT, blit_square};
use bitwise_games::{Game, Key};
use std::sync::OnceLock;

// ---- Board dimensions ----

const ROWS: usize = 6;
const COLS: usize = 7;

// ---- State encoding ----

const HEIGHT_BITS: u32 = 3;
const HEIGHT_MASK: u64 = (1u64 << HEIGHT_BITS) - 1;
const HEIGHTMAP_BITS: u32 = HEIGHT_BITS * COLS as u32; // 21

fn column_height(state: u64, col: usize) -> u32 {
    ((state >> (col as u32 * HEIGHT_BITS)) & HEIGHT_MASK) as u32
}

fn set_column_height(state: u64, col: usize, h: u32) -> u64 {
    let shift = col as u32 * HEIGHT_BITS;
    (state & !(HEIGHT_MASK << shift)) | (((h as u64) & HEIGHT_MASK) << shift)
}

fn cell_bit(row: usize, col: usize) -> u32 {
    HEIGHTMAP_BITS + (row * COLS + col) as u32
}

fn read_cell_color(state: u64, row: usize, col: usize) -> u64 {
    (state >> cell_bit(row, col)) & 1
}

fn write_cell_color(state: u64, row: usize, col: usize, bit: u64) -> u64 {
    let idx = cell_bit(row, col);
    (state & !(1u64 << idx)) | ((bit & 1) << idx)
}

// ---- Piece / status ----

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Piece {
    Empty,
    P1,
    P2,
}

fn piece_at(state: u64, row: usize, col: usize) -> Piece {
    let h = column_height(state, col) as usize;
    if row + h < ROWS {
        Piece::Empty
    } else if read_cell_color(state, row, col) == 1 {
        Piece::P1
    } else {
        Piece::P2
    }
}

fn total_pieces(state: u64) -> u32 {
    (0..COLS).map(|c| column_height(state, c)).sum()
}

fn active_player(state: u64) -> Piece {
    if total_pieces(state) % 2 == 0 {
        Piece::P1
    } else {
        Piece::P2
    }
}

fn player_color(p: Piece) -> Color {
    match p {
        Piece::P1 => RED,
        Piece::P2 => YELLOW,
        Piece::Empty => DARK_GREY,
    }
}

// ---- Move logic ----

fn drop_disc(state: u64, col: usize) -> u64 {
    let h = column_height(state, col);
    if h >= ROWS as u32 {
        return state;
    }
    let row = ROWS - 1 - h as usize;
    let bit = match active_player(state) {
        Piece::P1 => 1,
        Piece::P2 => 0,
        Piece::Empty => unreachable!(),
    };
    let s = write_cell_color(state, row, col, bit);
    set_column_height(s, col, h + 1)
}

// ---- Win / draw ----

const DIRECTIONS: [(i32, i32); 4] = [(0, 1), (1, 0), (1, 1), (1, -1)];

fn find_winner(state: u64) -> Option<(Piece, [(usize, usize); 4])> {
    for r in 0..ROWS {
        for c in 0..COLS {
            let start = piece_at(state, r, c);
            if start == Piece::Empty {
                continue;
            }
            for (dr, dc) in DIRECTIONS {
                let end_r = r as i32 + 3 * dr;
                let end_c = c as i32 + 3 * dc;
                if end_r < 0 || end_r >= ROWS as i32 || end_c < 0 || end_c >= COLS as i32 {
                    continue;
                }
                let mut quartet = [(0, 0); 4];
                let mut all = true;
                for (k, slot) in quartet.iter_mut().enumerate() {
                    let rr = (r as i32 + k as i32 * dr) as usize;
                    let cc = (c as i32 + k as i32 * dc) as usize;
                    *slot = (rr, cc);
                    if piece_at(state, rr, cc) != start {
                        all = false;
                        break;
                    }
                }
                if all {
                    return Some((start, quartet));
                }
            }
        }
    }
    None
}

fn is_full(state: u64) -> bool {
    (0..COLS).all(|c| column_height(state, c) >= ROWS as u32)
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Status {
    Playing,
    Win(Piece, [(usize, usize); 4]),
    Draw,
}

fn status_of(state: u64) -> Status {
    if let Some((p, q)) = find_winner(state) {
        return Status::Win(p, q);
    }
    if is_full(state) {
        return Status::Draw;
    }
    Status::Playing
}

// ---- Layout (from the .aseprite template) ----

/// Column pitch (cells are uniform horizontally).
const CELL_PITCH_X: u32 = 16;

/// Disc-centre X for (row, col)=(*, 0). Other columns are at this + col*pitch.
const DISC_CENTER_X0: u32 = 15;

/// Disc-centre Y per row. The sprite isn't strictly uniform vertically —
/// row 3 is one pixel shorter than the others, so rows 4 and 5 sit one
/// pixel higher than a flat-pitch formula would predict. We mirror the
/// sprite's actual layout pixel-for-pixel rather than over-painting.
const ROW_CENTER_Y: [u32; ROWS] = [32, 48, 64, 80, 95, 111];

/// Y of the topmost fill row per cell row (one below the D top arc).
const ROW_FILL_TOP_Y: [u32; ROWS] = [26, 42, 58, 74, 89, 105];

/// Preview disc (top of screen) — drawn procedurally with a BLACK outline.
const PREVIEW_DISC_X0: u32 = 9; // left edge of preview disc for col 0
const PREVIEW_DISC_Y: u32 = 5;

fn disc_center(row: usize, col: usize) -> (u32, u32) {
    (
        DISC_CENTER_X0 + col as u32 * CELL_PITCH_X,
        ROW_CENTER_Y[row],
    )
}

fn hovered_column(mouse: Option<(u8, u8)>) -> Option<usize> {
    let (x, y) = mouse?;
    let x = x as u32;
    let y = y as u32;
    // The hover band covers the preview row plus the board (anywhere above the ground strip).
    if y >= ROW_CENTER_Y[ROWS - 1] + 8 {
        return None;
    }
    // Column N spans absolute x = (PREVIEW_DISC_X0 + N*16) .. + 14.
    if x < PREVIEW_DISC_X0 {
        return None;
    }
    let col = (x - PREVIEW_DISC_X0) / CELL_PITCH_X;
    if col >= COLS as u32 {
        return None;
    }
    Some(col as usize)
}

// ---- Disc fill shape (centred on the disc centre) ----

/// Per-row run of fill pixels inside a 15×15 cell hole. 13 rows of fill
/// sit between the top D arc and the bottom P arc; each entry is
/// `(sx_offset_from_centre, width)`.
const CELL_FILL_RUNS: [(i32, u32); 13] = [
    (-4, 9),  // y_offset = -6
    (-5, 11), // y_offset = -5
    (-6, 13), // y_offset = -4
    (-6, 13), // y_offset = -3
    (-6, 13), // y_offset = -2
    (-6, 13), // y_offset = -1
    (-6, 13), // y_offset = 0
    (-6, 13), // y_offset = +1
    (-6, 13), // y_offset = +2
    (-6, 13), // y_offset = +3
    (-6, 13), // y_offset = +4
    (-5, 11), // y_offset = +5
    (-4, 9),  // y_offset = +6
];

fn paint_cell_fill(commands: &mut Vec<DrawCommand>, row: usize, col: usize, color: Color) {
    let cx = DISC_CENTER_X0 + col as u32 * CELL_PITCH_X;
    let top_y = ROW_FILL_TOP_Y[row];
    for (i, &(sx, w)) in CELL_FILL_RUNS.iter().enumerate() {
        let y = top_y + i as u32;
        let x = (cx as i32) + sx;
        commands.push(DrawCommand::rect(x as u32, y, w, 1, color));
    }
}

/// Outline rims of a 15×15 cell hole, used for win-highlight overpaint.
/// Per row: (top-arc full width OR (left rim x_offset, right rim x_offset)).
/// We emit two 1-px outline rects per "body" row plus one full-width row
/// for the top and bottom arcs.
fn paint_cell_outline(commands: &mut Vec<DrawCommand>, row: usize, col: usize, color: Color) {
    let (cx, cy) = disc_center(row, col);
    // Top arc (y_offset = -7): cols cx-4..cx+4 (9 wide)
    commands.push(DrawCommand::rect(
        (cx as i32 - 4) as u32,
        cy - 7,
        9,
        1,
        color,
    ));
    // Bottom arc (y_offset = +7): cols cx-4..cx+4 (9 wide)
    commands.push(DrawCommand::rect(
        (cx as i32 - 4) as u32,
        cy + 7,
        9,
        1,
        color,
    ));
    // Rims for the 13 body rows (offsets -6..+6).
    let rim_offsets: [u32; 13] = [5, 6, 7, 7, 7, 7, 7, 7, 7, 7, 7, 6, 5];
    for (i, &r) in rim_offsets.iter().enumerate() {
        let y = (cy as i32) - 6 + i as i32;
        let lx = (cx as i32) - (r as i32);
        let rx = (cx as i32) + (r as i32);
        commands.push(DrawCommand::rect(lx as u32, y as u32, 1, 1, color));
        commands.push(DrawCommand::rect(rx as u32, y as u32, 1, 1, color));
    }
}

// ---- Preview disc (14×14 with BLACK outline) ----

/// 14×14 preview disc rows, per row `(outline-left-sx, outline-left-w,
/// fill-sx, fill-w, outline-right-sx, outline-right-w)`. Offsets are from
/// the disc's top-left corner.
const PREVIEW_ROWS: [(u32, u32, u32, u32, u32, u32); 14] = [
    (5, 4, 0, 0, 0, 0),   // row 0: 4 K-only at center
    (3, 2, 5, 4, 9, 2),   // row 1: 2K + 4 fill + 2K
    (2, 1, 3, 8, 11, 1),  // row 2: 1K + 8 fill + 1K
    (1, 1, 2, 10, 12, 1), // row 3: 1K + 10 fill + 1K
    (1, 1, 2, 10, 12, 1), // row 4
    (0, 1, 1, 12, 13, 1), // row 5: 1K + 12 fill + 1K
    (0, 1, 1, 12, 13, 1), // row 6
    (0, 1, 1, 12, 13, 1), // row 7
    (0, 1, 1, 12, 13, 1), // row 8
    (1, 1, 2, 10, 12, 1), // row 9
    (1, 1, 2, 10, 12, 1), // row 10
    (2, 1, 3, 8, 11, 1),  // row 11
    (3, 2, 5, 4, 9, 2),   // row 12
    (5, 4, 0, 0, 0, 0),   // row 13: 4 K-only at center
];

fn paint_preview_disc(commands: &mut Vec<DrawCommand>, col: usize, fill: Color) {
    let x0 = PREVIEW_DISC_X0 + col as u32 * CELL_PITCH_X;
    let y0 = PREVIEW_DISC_Y;
    for (i, &(out_l, out_l_w, fl, fl_w, out_r, out_r_w)) in PREVIEW_ROWS.iter().enumerate() {
        let y = y0 + i as u32;
        if out_l_w > 0 {
            commands.push(DrawCommand::rect(x0 + out_l, y, out_l_w, 1, BLACK));
        }
        if fl_w > 0 {
            commands.push(DrawCommand::rect(x0 + fl, y, fl_w, 1, fill));
        }
        if out_r_w > 0 && (fl_w > 0) {
            commands.push(DrawCommand::rect(x0 + out_r, y, out_r_w, 1, BLACK));
        }
    }
}

// ---- Template loading ----

/// Load the hand-drawn 128×128 template once. Pre-process: clear the
/// preview row (we paint a fresh preview disc per frame) and strip RED /
/// YELLOW pixels out of the board's sample cells (we paint fills per
/// frame from game state). The result is an "empty board" stencil ready
/// to blit as the base layer.
fn template() -> &'static [[u8; 128]; 128] {
    static TEMPLATE: OnceLock<[[u8; 128]; 128]> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let raw = load_color_grid::<128, 128, 1, 1>(include_bytes!("connect4.aseprite"));
        let mut img = raw[0];
        // Preview row: 20 px tall at the top — wipe to transparent, will be
        // repainted each frame.
        for row in img.iter_mut().take(20) {
            for px in row.iter_mut() {
                *px = TRANSPARENT;
            }
        }
        // Strip sample disc fills (R / Y) anywhere in the image.
        let red = Color::Red as u8;
        let yellow = Color::Yellow as u8;
        for row in img.iter_mut() {
            for px in row.iter_mut() {
                if *px == red || *px == yellow {
                    *px = TRANSPARENT;
                }
            }
        }
        img
    })
}

// ---- Rendering ----

fn winning_cells(st: Status) -> Option<[(usize, usize); 4]> {
    match st {
        Status::Win(_, q) => Some(q),
        _ => None,
    }
}

fn status_text(st: Status) -> (&'static [u8], Color) {
    match st {
        Status::Win(Piece::P1, _) => (b"P1 WINS", RED),
        Status::Win(Piece::P2, _) => (b"P2 WINS", YELLOW),
        Status::Draw => (b"DRAW", WHITE),
        _ => (b"", WHITE),
    }
}

/// A centred banner over the board: black box + 1 px white border + text.
fn draw_banner(commands: &mut Vec<DrawCommand>, text: &[u8], color: Color) {
    let scale = 2;
    let tw = text_width(text.len(), scale);
    let th = GLYPH_H * scale;
    let pad = 6;
    let bw = tw + 2 * pad;
    let bh = th + 2 * pad;
    let bx = (frame_buffer::WIDTH - bw) / 2;
    let by = 56; // roughly the centre of the board
    commands.push(DrawCommand::rect(bx, by, bw, bh, BLACK));
    // 1 px border in the banner color.
    commands.push(DrawCommand::rect(bx, by, bw, 1, color));
    commands.push(DrawCommand::rect(bx, by + bh - 1, bw, 1, color));
    commands.push(DrawCommand::rect(bx, by, 1, bh, color));
    commands.push(DrawCommand::rect(bx + bw - 1, by, 1, bh, color));
    let tx = bx + pad;
    let ty = by + pad;
    draw_text(commands, text, tx, ty, scale, color);
}

fn render(state: u64, hover_col: Option<usize>) -> FrameBuffer {
    let mut fb = FrameBuffer::new();
    let st = status_of(state);
    let active = active_player(state);
    let winning = winning_cells(st);

    // Layer 1: clear to DARK_GREY (the "table" surface around the board).
    let mut bg = Vec::with_capacity(8);
    bg.push(DrawCommand::rect(
        0,
        0,
        frame_buffer::WIDTH,
        frame_buffer::HEIGHT,
        DARK_GREY,
    ));
    fb.draw_list(&bg);

    // Layer 2: blit the hand-drawn board template (bevels, slot openings,
    // disc rims). Transparent pixels show the DARK_GREY beneath.
    blit_square::<128>(&mut fb, template(), 0, 0, Rot::R0);

    // Layer 3: paint each cell's interior with the appropriate disc
    // colour. Empty cells get DARK_GREY (matching the "see through" look);
    // filled cells get RED / YELLOW.
    let mut overlay = Vec::with_capacity(128);
    for r in 0..ROWS {
        for c in 0..COLS {
            let fill = player_color(piece_at(state, r, c));
            paint_cell_fill(&mut overlay, r, c, fill);
        }
    }

    // Layer 4: winning quartet — repaint the disc rim in WHITE for a glow.
    if let Some(q) = winning {
        for &(r, c) in q.iter() {
            paint_cell_outline(&mut overlay, r, c, WHITE);
        }
    }

    // Layer 5: preview disc above the hovered column (only when playing
    // and the column has room).
    if matches!(st, Status::Playing) {
        if let Some(col) = hover_col {
            if column_height(state, col) < ROWS as u32 {
                let fill = match active {
                    Piece::P1 => RED,
                    Piece::P2 => YELLOW,
                    Piece::Empty => unreachable!(),
                };
                paint_preview_disc(&mut overlay, col, fill);
            }
        }
    }

    fb.draw_list(&overlay);

    // Layer 6: game-over banner.
    if !matches!(st, Status::Playing) {
        let (text, color) = status_text(st);
        if !text.is_empty() {
            let mut banner = Vec::with_capacity(16);
            draw_banner(&mut banner, text, color);
            fb.draw_list(&banner);
        }
    }

    fb
}

// ---- Game impl ----

struct ConnectFour;

impl Game for ConnectFour {
    const NAME: &'static str = "Connect Four";
    const FPS: usize = 30;

    fn init(_args: Vec<String>) -> (u64, FrameBuffer) {
        let state = 0u64;
        (state, render(state, None))
    }

    fn update(
        state: u64,
        _held: &[Key],
        buffered: Option<Key>,
        mouse: Option<(u8, u8)>,
    ) -> (u64, FrameBuffer) {
        let hover = hovered_column(mouse);
        let st = status_of(state);

        if buffered == Some(Key::X) {
            let fresh = 0u64;
            return (fresh, render(fresh, hover));
        }

        match st {
            Status::Playing => {
                if buffered == Some(Key::Z) {
                    if let Some(col) = hover {
                        let new = drop_disc(state, col);
                        return (new, render(new, hover));
                    }
                }
            }
            _ => {
                if buffered == Some(Key::Z) {
                    let fresh = 0u64;
                    return (fresh, render(fresh, hover));
                }
            }
        }

        (state, render(state, hover))
    }
}

fn main() {
    bitwise_games::run_game::<ConnectFour>();
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_state_is_p1_turn() {
        assert_eq!(active_player(0), Piece::P1);
        assert_eq!(total_pieces(0), 0);
        assert_eq!(status_of(0), Status::Playing);
        for r in 0..ROWS {
            for c in 0..COLS {
                assert_eq!(piece_at(0, r, c), Piece::Empty);
            }
        }
    }

    #[test]
    fn drop_increments_height_and_alternates_turn() {
        let s0 = 0u64;
        let s1 = drop_disc(s0, 3);
        assert_eq!(column_height(s1, 3), 1);
        assert_eq!(piece_at(s1, ROWS - 1, 3), Piece::P1);
        assert_eq!(active_player(s1), Piece::P2);

        let s2 = drop_disc(s1, 3);
        assert_eq!(column_height(s2, 3), 2);
        assert_eq!(piece_at(s2, ROWS - 2, 3), Piece::P2);
        assert_eq!(active_player(s2), Piece::P1);
    }

    #[test]
    fn full_column_drop_is_noop() {
        let mut s = 0u64;
        for _ in 0..ROWS {
            s = drop_disc(s, 0);
        }
        assert_eq!(column_height(s, 0), ROWS as u32);
        let s_after = drop_disc(s, 0);
        assert_eq!(s, s_after);
    }

    #[test]
    fn horizontal_win_detected() {
        let mut s = 0u64;
        for c in 0..3 {
            s = drop_disc(s, c);
            s = drop_disc(s, c);
        }
        s = drop_disc(s, 3);
        match status_of(s) {
            Status::Win(Piece::P1, _) => {}
            other => panic!("expected P1 horizontal win, got {other:?}"),
        }
    }

    #[test]
    fn vertical_win_detected() {
        let mut s = 0u64;
        for i in 0..4 {
            s = drop_disc(s, 0);
            if i < 3 {
                assert_eq!(status_of(s), Status::Playing);
                s = drop_disc(s, 1);
            }
        }
        match status_of(s) {
            Status::Win(Piece::P1, _) => {}
            other => panic!("expected P1 vertical win, got {other:?}"),
        }
    }

    #[test]
    fn diagonal_win_detected() {
        let mut s = 0u64;
        s = set_column_height(s, 0, 1);
        s = set_column_height(s, 1, 2);
        s = set_column_height(s, 2, 3);
        s = set_column_height(s, 3, 4);
        s = write_cell_color(s, 5, 0, 1);
        s = write_cell_color(s, 4, 1, 1);
        s = write_cell_color(s, 3, 2, 1);
        s = write_cell_color(s, 2, 3, 1);
        match status_of(s) {
            Status::Win(Piece::P1, _) => {}
            other => panic!("expected P1 / diagonal win, got {other:?}"),
        }
    }

    #[test]
    fn full_board_is_full() {
        let mut s = 0u64;
        for c in 0..COLS {
            s = set_column_height(s, c, ROWS as u32);
        }
        assert!(is_full(s));
    }

    #[test]
    fn state_fits_in_63_bits() {
        let mut s = 0u64;
        for c in 0..COLS {
            for _ in 0..ROWS {
                s = drop_disc(s, c);
            }
        }
        assert_eq!(s >> 63, 0, "bit 63 should remain free: {s:064b}");
        for c in 0..COLS {
            assert_eq!(column_height(s, c), ROWS as u32);
        }
    }

    #[test]
    fn render_at_each_status_does_not_panic() {
        let _ = render(0, None);
        let _ = render(0, Some(3));
        let mut s = 0u64;
        for i in 0..4 {
            s = drop_disc(s, 0);
            if i < 3 {
                s = drop_disc(s, 1);
            }
        }
        let _ = render(s, Some(0));
        let _ = render(s, Some(6));
    }

    #[test]
    fn hovered_column_maps_mouse_x() {
        // Centre of each column should yield that column index.
        for c in 0..COLS {
            let x = (PREVIEW_DISC_X0 + c as u32 * CELL_PITCH_X + CELL_PITCH_X / 2) as u8;
            let y = 10;
            assert_eq!(hovered_column(Some((x, y))), Some(c));
        }
        // Off to either side returns None.
        assert_eq!(hovered_column(Some((0, 30))), None);
        assert_eq!(hovered_column(Some((127, 30))), None);
        // Below the board returns None.
        assert_eq!(hovered_column(Some((64, 125))), None);
    }

    #[test]
    fn template_loads() {
        // Smoke check: the template parses, is 128×128, and at least some
        // pixels are non-transparent after preprocessing.
        let t = template();
        let mut non_transparent = 0;
        for row in t.iter() {
            for px in row.iter() {
                if *px != TRANSPARENT {
                    non_transparent += 1;
                }
            }
        }
        assert!(
            non_transparent > 1000,
            "template should retain board bevels"
        );
    }
}
