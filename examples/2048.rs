/*

2048 state packed into a u64.

16 cells × 4 bits per cell, storing log₂ of the tile value:
  0 = empty
  1 = 2
  2 = 4
  3 = 8
   ...
  11 = 2048
   ...
  15 = 32768 (cap — adjacent 32768s won't merge)

Cell at (row, col) lives in bits (row*4 + col)*4 .. (row*4 + col)*4 + 4.
That fills all 64 bits, so there's no room for an RNG state. Spawning
is therefore derived from the board itself via `rng::next` — two games
that pass through the same board continue identically. The args seed
only affects the two opening tiles in `new`.

Score and game-over are likewise pure functions of the board:
  - score = Σ (k - 1) · 2^k over non-empty cells, where k = stored log₂.
    Equals the points a player would have earned to produce the board
    assuming every spawn was a 2 (4-spawns push true score slightly
    above this, but we can't observe spawn history).
  - game over = no empty cell AND no two adjacent cells (4-neighbour)
    share a value. On game over, Z resets; the reset seed is
    `rng::next(state)` so the next board still varies per losing position.

*/
use bitwise_games::bits::{get_bits, set_bits};
use bitwise_games::draw_command::{
    BLACK, BLUE, BROWN, Color, DARK_BLUE, DARK_GREEN, DARK_GREY, DARK_PURPLE, DrawCommand, GREEN,
    LAVENDER, LIGHT_GREY, LIGHT_PEACH, ORANGE, PINK, RED, WHITE, YELLOW,
};
use bitwise_games::font::{GLYPH_H, digits_of, draw_text, text_width};
use bitwise_games::frame_buffer::{self, FrameBuffer};
use bitwise_games::rng;
use bitwise_games::{Game, Key};

const BOARD_PX: u32 = frame_buffer::WIDTH;

// Layout: 10-pixel top header (score), then a centred 118×118 grid of 4 cells
// of 28 px each separated by 2-pixel gaps. Vertically the grid sits flush
// with the bottom (10 + 4·28 + 3·2 = 128).
const HEADER_H: u32 = 10;
const CELL: u32 = 28;
const GAP: u32 = 2;
const BOARD_W: u32 = 4 * CELL + 3 * GAP;
const BOARD_OFFSET_X: u32 = (BOARD_PX - BOARD_W) / 2;
const BOARD_OFFSET_Y: u32 = HEADER_H;

const FONT_SCALE: u32 = 1;
const BANNER_SCALE: u32 = 2;

type Board = [[u8; 4]; 4];

fn from_u64(state: u64) -> Board {
    let mut b = [[0u8; 4]; 4];
    for r in 0..4u8 {
        for c in 0..4u8 {
            let bit = (r * 4 + c) * 4;
            b[r as usize][c as usize] = get_bits(state, bit, 4);
        }
    }
    b
}

fn to_u64(b: &Board) -> u64 {
    let mut result = 0u64;
    for r in 0..4u8 {
        for c in 0..4u8 {
            let bit = (r * 4 + c) * 4;
            result = set_bits(result, b[r as usize][c as usize], bit, 4);
        }
    }
    result
}

fn empties(b: &Board) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for r in 0..4 {
        for c in 0..4 {
            if b[r][c] == 0 {
                out.push((r, c));
            }
        }
    }
    out
}

fn spawn(b: &mut Board, rng: u64) {
    let empt = empties(b);
    if empt.is_empty() {
        return;
    }
    let idx = (rng as usize) % empt.len();
    // 10% chance of "4" (log₂ = 2), else "2" (log₂ = 1)
    let val = if (rng >> 16) % 10 == 0 { 2 } else { 1 };
    let (r, c) = empt[idx];
    b[r][c] = val;
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

fn transpose(b: &mut Board) {
    for r in 0..4 {
        for c in (r + 1)..4 {
            let t = b[r][c];
            b[r][c] = b[c][r];
            b[c][r] = t;
        }
    }
}

fn slide(b: &mut Board, dir: Key) -> bool {
    let mut moved = false;
    match dir {
        Key::Left => {
            for row in b.iter_mut() {
                if slide_row_left(row) {
                    moved = true;
                }
            }
        }
        Key::Right => {
            for row in b.iter_mut() {
                row.reverse();
                if slide_row_left(row) {
                    moved = true;
                }
                row.reverse();
            }
        }
        Key::Up => {
            transpose(b);
            for row in b.iter_mut() {
                if slide_row_left(row) {
                    moved = true;
                }
            }
            transpose(b);
        }
        Key::Down => {
            transpose(b);
            for row in b.iter_mut() {
                row.reverse();
                if slide_row_left(row) {
                    moved = true;
                }
                row.reverse();
            }
            transpose(b);
        }
        _ => {}
    }
    moved
}

fn score(b: &Board) -> u32 {
    let mut s = 0u32;
    for row in b {
        for &v in row {
            if v >= 2 {
                s += (v as u32 - 1) * (1u32 << v);
            }
        }
    }
    s
}

fn has_moves(b: &Board) -> bool {
    for r in 0..4 {
        for c in 0..4 {
            if b[r][c] == 0 {
                return true;
            }
            if c < 3 && b[r][c] == b[r][c + 1] {
                return true;
            }
            if r < 3 && b[r][c] == b[r + 1][c] {
                return true;
            }
        }
    }
    false
}

fn fresh_board(seed: u64) -> Board {
    let mut b = [[0u8; 4]; 4];
    spawn(&mut b, rng::next(seed));
    spawn(&mut b, rng::next(rng::next(seed)));
    b
}

fn tile_color(v: u8) -> Color {
    match v {
        0 => DARK_BLUE,
        1 => LIGHT_GREY,
        2 => LIGHT_PEACH,
        3 => ORANGE,
        4 => PINK,
        5 => RED,
        6 => YELLOW,
        7 => GREEN,
        8 => BLUE,
        9 => LAVENDER,
        10 => DARK_PURPLE,
        11 => DARK_GREEN,
        12 => BROWN,
        13 => DARK_GREY,
        14 => RED,
        _ => WHITE,
    }
}

fn digit_color(v: u8) -> Color {
    // Light tiles get dark digits, dark tiles get light digits.
    match v {
        1 | 2 | 3 | 4 | 6 | 7 | 15 => BLACK,
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
    let x = BOARD_OFFSET_X + c as u32 * (CELL + GAP);
    let y = BOARD_OFFSET_Y + r as u32 * (CELL + GAP);

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

fn draw_score(commands: &mut Vec<DrawCommand>, b: &Board) {
    let digits = digits_of(score(b));
    let w = text_width(digits.len(), FONT_SCALE);
    // Right-aligned with 1-pixel margin from the right edge.
    let x = BOARD_PX - w - 1;
    draw_text(commands, &digits, x, 2, FONT_SCALE, WHITE);
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

fn render(b: &Board) -> FrameBuffer {
    let mut fb = FrameBuffer::new();
    let mut commands = Vec::new();

    commands.push(DrawCommand::rect(0, 0, BOARD_PX, BOARD_PX, BLACK));

    draw_score(&mut commands, b);

    for r in 0..4 {
        for c in 0..4 {
            draw_tile(&mut commands, r, c, b[r][c]);
        }
    }

    if !has_moves(b) {
        draw_game_over_banner(&mut commands);
    }

    fb.draw_list(&commands);
    fb
}

struct Twenty48;

impl Game for Twenty48 {
    const NAME: &'static str = "2048";
    const FPS: usize = 30;

    fn new(args: Vec<String>) -> (u64, FrameBuffer) {
        let seed = args.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let b = fresh_board(seed);
        (to_u64(&b), render(&b))
    }

    fn update(state: u64, _held: &[Key], buffered: Option<Key>) -> (u64, FrameBuffer) {
        let mut b = from_u64(state);

        if !has_moves(&b) {
            // Game over: Z resets, anything else holds the frozen view.
            if buffered == Some(Key::Z) {
                let new_b = fresh_board(rng::next(state));
                return (to_u64(&new_b), render(&new_b));
            }
            return (state, render(&b));
        }

        if let Some(dir @ (Key::Up | Key::Down | Key::Left | Key::Right)) = buffered {
            if slide(&mut b, dir) {
                let r = rng::next(to_u64(&b));
                spawn(&mut b, r);
            }
        }

        (to_u64(&b), render(&b))
    }
}

fn main() {
    bitwise_games::run_game::<Twenty48>();
}
