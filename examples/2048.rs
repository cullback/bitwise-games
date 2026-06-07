/*

2048 on a 4×4 grid.

# Inputs

- Arrow keys: slide and merge tiles in that direction
- Z: restart after game-over

# Maximize

Max tile value reachable within 64 bits. Each cell stores log₂ of its
tile value in 4 bits: 0 = empty, 1 = 2, 2 = 4, …, 15 = 32768. 4 bits ×
16 cells = exactly 64 — the budget fills the u64 with no headroom for
an RNG state. Spawning is therefore derived from the board itself, so
two games passing through the same board continue identically.

# Encoding

| Start  | Length | Description                                          |
|--------|--------|------------------------------------------------------|
|      0 |     64 | 16 cells × 4 bits each, row-major                    |

Cell at (row, col) lives in bits (row*4 + col)*4 .. (row*4 + col)*4 + 4
and stores log₂ of its tile value (or 0 for empty). Value cap is 32768
(encoded as 15) — adjacent 32768s won't merge.

# Notes

**No RNG state.** The 64-bit budget fills with cell data, so spawning a
new tile after a move pulls entropy from the board itself via
`rng::next(state)`. The `args` seed only affects the two opening tiles
in `new`.

**Score and game-over are derived.** Both are pure functions of the
current board:
  - score = Σ (k − 1) · 2^k over non-empty cells, where k is the stored
    log₂. Equals the points a player would have earned assuming every
    spawn was a 2 (4-spawns push true score slightly above, but spawn
    history isn't observable).
  - game over = no empty cell AND no two 4-adjacent cells share a
    value. On game over Z resets; the reset seed is `rng::next(state)`
    so the next board varies per losing position.

*/
use bitwise_games::bits::{get_bits, set_bits};
use bitwise_games::draw_command::{
    BLACK, BROWN, Color, DARK_GREEN, DARK_GREY, DARK_PURPLE, DrawCommand, LAVENDER, LIGHT_GREY,
    LIGHT_PEACH, ORANGE, PINK, RED, WHITE, YELLOW,
};
use bitwise_games::font::{GLYPH_H, digits_of, draw_text, text_width};
use bitwise_games::frame_buffer::{self, FrameBuffer};
use bitwise_games::rng;
use bitwise_games::{Game, Key};

const BOARD_PX: u32 = frame_buffer::WIDTH;

// Layout palette (matches the original web 2048):
//   - Background: light cream (WHITE)
//   - Board base: DARK_GREY — shows through the 2-px gaps between tiles as
//     each cell's border
//   - Empty cell interior: LIGHT_GREY
//   - Yellow "2048" badge top-left, score panel top-right
const HEADER_H: u32 = 20;
const CELL: u32 = 24;
const GAP: u32 = 2;
const PAD: u32 = GAP; // DARK_GREY border around the board matches inter-cell gap
const BOARD_W: u32 = 2 * PAD + 4 * CELL + 3 * GAP; // 106
const BOARD_OFFSET_X: u32 = (BOARD_PX - BOARD_W) / 2; // 11
const BOARD_OFFSET_Y: u32 = HEADER_H + 1; // 21

const FONT_SCALE: u32 = 1;
const BANNER_SCALE: u32 = 2;

// Badge sits at the left edge of the header with "2048" in WHITE.
const BADGE_X: u32 = BOARD_OFFSET_X;
const BADGE_Y: u32 = 1;
const BADGE_SIZE: u32 = 17;
const SCORE_PANEL_H: u32 = BADGE_SIZE;

struct State {
    cells: [[u8; 4]; 4],
}

fn decode(state: u64) -> State {
    let mut cells = [[0u8; 4]; 4];
    for r in 0..4u8 {
        for c in 0..4u8 {
            let bit = (r * 4 + c) * 4;
            cells[r as usize][c as usize] = get_bits(state, bit, 4);
        }
    }
    State { cells }
}

fn encode(state: &State) -> u64 {
    let mut result = 0u64;
    for r in 0..4u8 {
        for c in 0..4u8 {
            let bit = (r * 4 + c) * 4;
            result = set_bits(result, state.cells[r as usize][c as usize], bit, 4);
        }
    }
    result
}

fn empties(state: &State) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for r in 0..4 {
        for c in 0..4 {
            if state.cells[r][c] == 0 {
                out.push((r, c));
            }
        }
    }
    out
}

fn spawn(state: &mut State, rng: u64) {
    let empt = empties(state);
    if empt.is_empty() {
        return;
    }
    let idx = (rng as usize) % empt.len();
    // 10% chance of "4" (log₂ = 2), else "2" (log₂ = 1)
    let val = if (rng >> 16).is_multiple_of(10) { 2 } else { 1 };
    let (r, c) = empt[idx];
    state.cells[r][c] = val;
}

fn slide_row_left(row: &mut [u8; 4]) -> bool {
    let filtered: Vec<u8> = row.iter().copied().filter(|&v| v != 0).collect();
    let mut merged: Vec<u8> = Vec::with_capacity(4);
    let mut i = 0;
    while i < filtered.len() {
        if i + 1 < filtered.len() && filtered[i] == filtered[i + 1] && filtered[i] < 15 {
            merged.push(filtered[i] + 1);
            i += 2;
        } else {
            merged.push(filtered[i]);
            i += 1;
        }
    }
    while merged.len() < 4 {
        merged.push(0);
    }
    let new_row: [u8; 4] = merged.try_into().unwrap();
    let changed = new_row != *row;
    *row = new_row;
    changed
}

fn transpose(state: &mut State) {
    for r in 0..4 {
        for c in (r + 1)..4 {
            let t = state.cells[r][c];
            state.cells[r][c] = state.cells[c][r];
            state.cells[c][r] = t;
        }
    }
}

fn slide(state: &mut State, dir: Key) -> bool {
    let mut moved = false;
    match dir {
        Key::Left => {
            for row in state.cells.iter_mut() {
                if slide_row_left(row) {
                    moved = true;
                }
            }
        }
        Key::Right => {
            for row in state.cells.iter_mut() {
                row.reverse();
                if slide_row_left(row) {
                    moved = true;
                }
                row.reverse();
            }
        }
        Key::Up => {
            transpose(state);
            for row in state.cells.iter_mut() {
                if slide_row_left(row) {
                    moved = true;
                }
            }
            transpose(state);
        }
        Key::Down => {
            transpose(state);
            for row in state.cells.iter_mut() {
                row.reverse();
                if slide_row_left(row) {
                    moved = true;
                }
                row.reverse();
            }
            transpose(state);
        }
        _ => {}
    }
    moved
}

fn score(state: &State) -> u32 {
    let mut s = 0u32;
    for row in &state.cells {
        for &v in row {
            if v >= 2 {
                s += (v as u32 - 1) * (1u32 << v);
            }
        }
    }
    s
}

fn has_moves(state: &State) -> bool {
    for r in 0..4 {
        for c in 0..4 {
            if state.cells[r][c] == 0 {
                return true;
            }
            if c < 3 && state.cells[r][c] == state.cells[r][c + 1] {
                return true;
            }
            if r < 3 && state.cells[r][c] == state.cells[r + 1][c] {
                return true;
            }
        }
    }
    false
}

fn fresh_board(seed: u64) -> State {
    let mut state = State {
        cells: [[0u8; 4]; 4],
    };
    spawn(&mut state, rng::next(seed));
    spawn(&mut state, rng::next(rng::next(seed)));
    state
}

fn tile_color(v: u8) -> Color {
    // Roughly mirrors the original 2048 palette: light cream for the small
    // tiles, warm orange-red ramp through 32–64, then a yellow series at the
    // 128+ tier with a few off-palette accents for the late game.
    match v {
        0 => LIGHT_GREY,   // empty
        1 => WHITE,        // 2
        2 => LIGHT_PEACH,  // 4
        3 => ORANGE,       // 8
        4 => PINK,         // 16
        5 => RED,          // 32
        6 => DARK_PURPLE,  // 64
        7 => YELLOW,       // 128
        8 => YELLOW,       // 256
        9 => YELLOW,       // 512
        10 => BROWN,       // 1024
        11 => YELLOW,      // 2048 (special)
        12 => DARK_GREEN,  // 4096
        13 => LAVENDER,    // 8192
        14 => DARK_PURPLE, // 16384
        _ => WHITE,        // 32768 cap
    }
}

fn digit_color(v: u8) -> Color {
    // Light tiles get dark digits, dark tiles get light digits.
    match v {
        1 | 2 | 7 | 8 | 9 | 11 | 15 => BLACK,
        _ => WHITE,
    }
}

fn tile_digits(v: u8) -> Vec<u8> {
    if v == 0 {
        Vec::new()
    } else {
        digits_of(1u32 << v)
    }
}

fn draw_tile(commands: &mut Vec<DrawCommand>, r: usize, c: usize, v: u8) {
    let x = BOARD_OFFSET_X + PAD + c as u32 * (CELL + GAP);
    let y = BOARD_OFFSET_Y + PAD + r as u32 * (CELL + GAP);

    commands.push(DrawCommand::rect(x, y, CELL, CELL, tile_color(v)));

    let digits = tile_digits(v);
    if digits.is_empty() {
        return;
    }

    let total_w = text_width(digits.len(), FONT_SCALE);
    let glyph_h = GLYPH_H * FONT_SCALE;
    let dx = x + (CELL - total_w) / 2;
    let dy = y + (CELL - glyph_h) / 2;

    draw_text(commands, &digits, dx, dy, FONT_SCALE, digit_color(v));
}

/// Yellow "2048" badge at the left of the header with DARK_GREY text —
/// softer than BLACK while staying readable against the saturated YELLOW.
fn draw_badge(commands: &mut Vec<DrawCommand>) {
    commands.push(DrawCommand::rect(
        BADGE_X, BADGE_Y, BADGE_SIZE, BADGE_SIZE, YELLOW,
    ));
    let label = b"2048";
    let scale = 1u32;
    let w = text_width(label.len(), scale);
    let h = GLYPH_H * scale;
    let tx = BADGE_X + (BADGE_SIZE - w) / 2;
    let ty = BADGE_Y + (BADGE_SIZE - h) / 2;
    draw_text(commands, label, tx, ty, scale, DARK_GREY);
}

/// Score panel to the right of the badge: DARK_GREY background with the
/// current score in WHITE digits, right-aligned.
fn draw_score(commands: &mut Vec<DrawCommand>, state: &State) {
    let panel_y = BADGE_Y;
    // Panel spans from a small gap right of the badge to a matching gap
    // before the board's right edge.
    let panel_x = BADGE_X + BADGE_SIZE + 3;
    let panel_w = BOARD_OFFSET_X + BOARD_W - panel_x;
    commands.push(DrawCommand::rect(
        panel_x,
        panel_y,
        panel_w,
        SCORE_PANEL_H,
        DARK_GREY,
    ));

    let digits = digits_of(score(state));
    let scale = 1u32;
    let w = text_width(digits.len(), scale);
    let h = GLYPH_H * scale;
    let tx = panel_x + panel_w - w - 2;
    let ty = panel_y + (SCORE_PANEL_H - h) / 2;
    draw_text(commands, &digits, tx, ty, scale, WHITE);
}

fn draw_game_over_banner(commands: &mut Vec<DrawCommand>) {
    let line1 = b"GAME OVER";
    let line2 = b"PRESS Z";
    let line1_w = text_width(line1.len(), BANNER_SCALE);
    let line2_w = text_width(line2.len(), BANNER_SCALE);
    let banner_w = line1_w.max(line2_w) + 8;
    let banner_h = 36;
    let banner_x = (BOARD_PX - banner_w) / 2;
    let banner_y = (BOARD_PX - banner_h) / 2;

    let color = RED;
    commands.push(DrawCommand::rect(
        banner_x, banner_y, banner_w, banner_h, BLACK,
    ));
    commands.push(DrawCommand::rect(banner_x, banner_y, banner_w, 1, color));
    commands.push(DrawCommand::rect(
        banner_x,
        banner_y + banner_h - 1,
        banner_w,
        1,
        color,
    ));
    commands.push(DrawCommand::rect(banner_x, banner_y, 1, banner_h, color));
    commands.push(DrawCommand::rect(
        banner_x + banner_w - 1,
        banner_y,
        1,
        banner_h,
        color,
    ));

    let line1_x = banner_x + (banner_w - line1_w) / 2;
    let line2_x = banner_x + (banner_w - line2_w) / 2;
    draw_text(commands, line1, line1_x, banner_y + 6, BANNER_SCALE, color);
    draw_text(commands, line2, line2_x, banner_y + 22, BANNER_SCALE, WHITE);
}

fn render(state: &State) -> FrameBuffer {
    let mut fb = FrameBuffer::new();
    let mut commands = Vec::new();

    // Light cream background (closest PICO-8 colour to the original's #faf8ef).
    commands.push(DrawCommand::rect(0, 0, BOARD_PX, BOARD_PX, WHITE));

    // Board base: DARK_GREY block under the tiles; the 2-px gaps between
    // tiles show through as the per-cell borders.
    commands.push(DrawCommand::rect(
        BOARD_OFFSET_X,
        BOARD_OFFSET_Y,
        BOARD_W,
        BOARD_W,
        DARK_GREY,
    ));

    draw_badge(&mut commands);
    draw_score(&mut commands, state);

    for r in 0..4 {
        for c in 0..4 {
            draw_tile(&mut commands, r, c, state.cells[r][c]);
        }
    }

    if !has_moves(state) {
        draw_game_over_banner(&mut commands);
    }

    fb.draw_list(&commands);
    fb
}

struct Twenty48;

impl Game for Twenty48 {
    const NAME: &'static str = "2048";
    const FPS: usize = 30;

    fn init(args: Vec<String>) -> (u64, FrameBuffer) {
        let seed = args.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let state = fresh_board(seed);
        (encode(&state), render(&state))
    }

    fn update(
        state: u64,
        _held: &[Key],
        buffered: Option<Key>,
        _mouse: Option<(u8, u8)>,
    ) -> (u64, FrameBuffer) {
        let mut state = decode(state);

        if !has_moves(&state) {
            // Game over: Z resets, anything else holds the frozen view.
            if buffered == Some(Key::Z) {
                let new_state = fresh_board(rng::next(encode(&state)));
                return (encode(&new_state), render(&new_state));
            }
            return (encode(&state), render(&state));
        }

        if let Some(dir @ (Key::Up | Key::Down | Key::Left | Key::Right)) = buffered
            && slide(&mut state, dir)
        {
            let r = rng::next(encode(&state));
            spawn(&mut state, r);
        }

        (encode(&state), render(&state))
    }
}

fn main() {
    bitwise_games::run_game::<Twenty48>();
}
