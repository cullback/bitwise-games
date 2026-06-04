/*

- 4x4 board, 15 numbered tiles + 1 empty slot
- 15 tiles × 4 bits = 60 bits for tile positions (slot index 0..15)
- Empty slot is implied: the one slot not occupied by any tile
- 4 bits: previous-frame arrow keys, for rising-edge detection so
  held arrows don't slide multiple tiles per frame

*/
use bitwise_games::Game;
use bitwise_games::bits::{get_bits, set_bits};
use bitwise_games::draw_command::{BLUE, DARK_BLUE, DrawCommand, GREEN, WHITE};
use bitwise_games::frame_buffer::FrameBuffer;
use minifb::Key;
use std::time::{SystemTime, UNIX_EPOCH};

const BOARD_PX: u32 = 480;
const TILE: u32 = BOARD_PX / 4;
const GAP: u32 = 4;
const FONT_SCALE: u32 = 12;
const DIGIT_W: u32 = 3 * FONT_SCALE;
const DIGIT_H: u32 = 5 * FONT_SCALE;
const DIGIT_GAP: u32 = FONT_SCALE;

const KEY_UP: u8 = 1;
const KEY_DOWN: u8 = 2;
const KEY_LEFT: u8 = 4;
const KEY_RIGHT: u8 = 8;

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

struct Puzzle {
    tiles: [u8; 16],
    prev_keys: u8,
}

fn solved_tiles() -> [u8; 16] {
    let mut t = [0u8; 16];
    for i in 0..15 {
        t[i] = (i + 1) as u8;
    }
    t
}

fn from_u64(state: u64) -> Puzzle {
    let mut tiles = [0u8; 16];
    for tile in 1..=15u8 {
        let pos: u8 = get_bits(state, (tile - 1) * 4, 4);
        tiles[pos as usize] = tile;
    }
    let prev_keys: u8 = get_bits(state, 60, 4);
    Puzzle { tiles, prev_keys }
}

fn to_u64(p: &Puzzle) -> u64 {
    let mut result = 0u64;
    for slot in 0..16u8 {
        let tile = p.tiles[slot as usize];
        if tile > 0 {
            result = set_bits(result, slot, (tile - 1) * 4, 4);
        }
    }
    result = set_bits(result, p.prev_keys, 60, 4);
    result
}

fn find_empty(tiles: &[u8; 16]) -> usize {
    tiles.iter().position(|&t| t == 0).unwrap()
}

fn is_solved(tiles: &[u8; 16]) -> bool {
    (0..15).all(|i| tiles[i] == (i + 1) as u8)
}

// Returns the slot that should move into the empty slot, given the arrow key.
// Arrow direction = direction the tile slides.
fn source_slot(empty: usize, dir: u8) -> Option<usize> {
    let row = empty / 4;
    let col = empty % 4;
    match dir {
        KEY_UP if row < 3 => Some(empty + 4),
        KEY_DOWN if row > 0 => Some(empty - 4),
        KEY_LEFT if col < 3 => Some(empty + 1),
        KEY_RIGHT if col > 0 => Some(empty - 1),
        _ => None,
    }
}

fn try_slide(tiles: &mut [u8; 16], dir: u8) -> bool {
    let empty = find_empty(tiles);
    match source_slot(empty, dir) {
        Some(src) => {
            tiles[empty] = tiles[src];
            tiles[src] = 0;
            true
        }
        None => false,
    }
}

fn opposite(dir: u8) -> u8 {
    match dir {
        KEY_UP => KEY_DOWN,
        KEY_DOWN => KEY_UP,
        KEY_LEFT => KEY_RIGHT,
        KEY_RIGHT => KEY_LEFT,
        _ => 0,
    }
}

fn scramble(seed: u64) -> [u8; 16] {
    let mut tiles = solved_tiles();
    let mut rng = seed | 1; // avoid zero state
    let mut last_dir: u8 = 0;
    let mut moves_made = 0;
    while moves_made < 200 {
        // Numerical Recipes LCG
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let dir = match (rng >> 33) & 0x3 {
            0 => KEY_UP,
            1 => KEY_DOWN,
            2 => KEY_LEFT,
            _ => KEY_RIGHT,
        };
        if dir == opposite(last_dir) {
            continue;
        }
        if try_slide(&mut tiles, dir) {
            last_dir = dir;
            moves_made += 1;
        }
    }
    tiles
}

fn keys_mask(input: &[Key]) -> u8 {
    let mut m = 0u8;
    if input.contains(&Key::Up) {
        m |= KEY_UP;
    }
    if input.contains(&Key::Down) {
        m |= KEY_DOWN;
    }
    if input.contains(&Key::Left) {
        m |= KEY_LEFT;
    }
    if input.contains(&Key::Right) {
        m |= KEY_RIGHT;
    }
    m
}

fn draw_digit(commands: &mut Vec<DrawCommand>, digit: u8, x: u32, y: u32) {
    let pattern = FONT[digit as usize];
    for (row, bits) in pattern.iter().enumerate() {
        for col in 0..3u32 {
            if (bits >> (2 - col)) & 1 == 1 {
                commands.push(DrawCommand::rect(
                    x + col * FONT_SCALE,
                    y + row as u32 * FONT_SCALE,
                    FONT_SCALE,
                    FONT_SCALE,
                    WHITE,
                ));
            }
        }
    }
}

fn draw_tile(commands: &mut Vec<DrawCommand>, slot: usize, value: u8, solved: bool) {
    let slot_row = (slot / 4) as u32;
    let slot_col = (slot % 4) as u32;
    let tile_x = slot_col * TILE + GAP / 2;
    let tile_y = slot_row * TILE + GAP / 2;
    let tile_w = TILE - GAP;
    let tile_h = TILE - GAP;

    commands.push(DrawCommand::rect(
        tile_x,
        tile_y,
        tile_w,
        tile_h,
        if solved { GREEN } else { BLUE },
    ));

    if value < 10 {
        let dx = tile_x + (tile_w - DIGIT_W) / 2;
        let dy = tile_y + (tile_h - DIGIT_H) / 2;
        draw_digit(commands, value, dx, dy);
    } else {
        let total_w = DIGIT_W * 2 + DIGIT_GAP;
        let dx = tile_x + (tile_w - total_w) / 2;
        let dy = tile_y + (tile_h - DIGIT_H) / 2;
        draw_digit(commands, 1, dx, dy);
        draw_digit(commands, value - 10, dx + DIGIT_W + DIGIT_GAP, dy);
    }
}

fn render(puzzle: &Puzzle) -> Vec<u32> {
    let mut fb = FrameBuffer::new(BOARD_PX, BOARD_PX);
    let mut commands = Vec::new();

    commands.push(DrawCommand::rect(0, 0, BOARD_PX, BOARD_PX, DARK_BLUE));

    let solved = is_solved(&puzzle.tiles);
    for slot in 0..16 {
        let value = puzzle.tiles[slot];
        if value > 0 {
            draw_tile(&mut commands, slot, value, solved);
        }
    }

    fb.draw_list(&commands);
    fb.pixels
}

struct Fifteen;

impl Game for Fifteen {
    const NAME: &'static str = "15 Puzzle";
    const WIDTH: usize = BOARD_PX as usize;
    const HEIGHT: usize = BOARD_PX as usize;
    const FPS: usize = 30;

    fn new(args: Vec<String>) -> (u64, Vec<u32>) {
        let seed = args
            .get(1)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or_else(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_micros() as u64)
                    .unwrap_or(1)
            });
        let puzzle = Puzzle {
            tiles: scramble(seed),
            prev_keys: 0,
        };
        let fb = render(&puzzle);
        (to_u64(&puzzle), fb)
    }

    fn update(state: u64, input: &[Key]) -> (u64, Vec<u32>) {
        let mut puzzle = from_u64(state);
        let keys = keys_mask(input);
        let rising = keys & !puzzle.prev_keys;

        for dir in [KEY_UP, KEY_DOWN, KEY_LEFT, KEY_RIGHT] {
            if rising & dir != 0 {
                try_slide(&mut puzzle.tiles, dir);
                break;
            }
        }

        puzzle.prev_keys = keys;
        let fb = render(&puzzle);
        (to_u64(&puzzle), fb)
    }
}

fn main() {
    bitwise_games::run_game::<Fifteen>();
}
