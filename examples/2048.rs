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

*/
use bitwise_games::Game;
use bitwise_games::bits::{get_bits, set_bits};
use bitwise_games::draw_command::{
    BLACK, BLUE, BROWN, Color, DARK_BLUE, DARK_GREEN, DARK_GREY, DARK_PURPLE, DrawCommand, GREEN,
    LAVENDER, LIGHT_GREY, LIGHT_PEACH, ORANGE, PINK, RED, WHITE, YELLOW,
};
use bitwise_games::frame_buffer::FrameBuffer;
use bitwise_games::rng;
use minifb::Key;

const BOARD_PX: u32 = 480;
const CELL: u32 = BOARD_PX / 4;
const GAP: u32 = 4;
const FONT_SCALE: u32 = 5;
const DIGIT_W: u32 = 3 * FONT_SCALE;
const DIGIT_H: u32 = 5 * FONT_SCALE;
const DIGIT_GAP: u32 = FONT_SCALE;

// 3x5 digit font, MSB = leftmost pixel
const FONT: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

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

fn digits_of(v: u8) -> Vec<u8> {
    if v == 0 {
        return Vec::new();
    }
    let mut n: u32 = 1u32 << v;
    let mut out = Vec::new();
    while n > 0 {
        out.push((n % 10) as u8);
        n /= 10;
    }
    out.reverse();
    out
}

fn draw_digit(commands: &mut Vec<DrawCommand>, digit: u8, x: u32, y: u32, color: Color) {
    let pattern = FONT[digit as usize];
    for (row, bits) in pattern.iter().enumerate() {
        for col in 0..3u32 {
            if (bits >> (2 - col)) & 1 == 1 {
                commands.push(DrawCommand::rect(
                    x + col * FONT_SCALE,
                    y + row as u32 * FONT_SCALE,
                    FONT_SCALE,
                    FONT_SCALE,
                    color,
                ));
            }
        }
    }
}

fn draw_tile(commands: &mut Vec<DrawCommand>, r: usize, c: usize, v: u8) {
    let x = c as u32 * CELL + GAP / 2;
    let y = r as u32 * CELL + GAP / 2;
    let w = CELL - GAP;

    commands.push(DrawCommand::rect(x, y, w, w, tile_color(v)));

    let digits = digits_of(v);
    if digits.is_empty() {
        return;
    }

    let n = digits.len() as u32;
    let total_w = n * DIGIT_W + (n - 1) * DIGIT_GAP;
    let dx = x + (w - total_w) / 2;
    let dy = y + (w - DIGIT_H) / 2;

    let color = digit_color(v);
    let mut cur_x = dx;
    for &d in &digits {
        draw_digit(commands, d, cur_x, dy, color);
        cur_x += DIGIT_W + DIGIT_GAP;
    }
}

fn render(b: &Board) -> Vec<u32> {
    let mut fb = FrameBuffer::new(BOARD_PX, BOARD_PX);
    let mut commands = Vec::new();

    commands.push(DrawCommand::rect(0, 0, BOARD_PX, BOARD_PX, BLACK));

    for r in 0..4 {
        for c in 0..4 {
            draw_tile(&mut commands, r, c, b[r][c]);
        }
    }

    fb.draw_list(&commands);
    fb.pixels
}

struct Twenty48;

impl Game for Twenty48 {
    const NAME: &'static str = "2048";
    const WIDTH: usize = BOARD_PX as usize;
    const HEIGHT: usize = BOARD_PX as usize;
    const FPS: usize = 30;

    fn new(args: Vec<String>) -> (u64, Vec<u32>) {
        let seed = args.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let mut b = [[0u8; 4]; 4];
        spawn(&mut b, rng::next(seed));
        spawn(&mut b, rng::next(rng::next(seed)));
        (to_u64(&b), render(&b))
    }

    fn update(state: u64, _held: &[Key], buffered: &[Key]) -> (u64, Vec<u32>) {
        let mut b = from_u64(state);
        for dir in [Key::Up, Key::Down, Key::Left, Key::Right] {
            if buffered.contains(&dir) {
                if slide(&mut b, dir) {
                    let r = rng::next(to_u64(&b));
                    spawn(&mut b, r);
                }
                break;
            }
        }
        (to_u64(&b), render(&b))
    }
}

fn main() {
    bitwise_games::run_game::<Twenty48>();
}
