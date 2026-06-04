/*

15 puzzle state packed into a u64.

The board is a permutation of the 16 values {0, 1, ..., 15} across 16
slots, where 0 is the empty slot and 1..15 are the numbered tiles.
Every reachable (and unreachable) board is one such permutation, so
the state is simply that permutation's lexicographic rank — computed
via the permutation module, no bit-packing required.

There are 16! ≈ 2.09e13 permutations, so the state occupies ≈ 44 bits
of the u64; the top 20 bits are zero.

A nice property of the 15 puzzle: only *half* of those 16! permutations
are actually reachable from the solved state. Every legal slide flips
both the permutation's parity (it's an odd transposition with the
empty cell) and the parity of the empty cell's row index, so
`perm_parity XOR empty_row_parity` is a conserved invariant. We always
start from solved and apply legal moves, so the scramble only lands
in solvable states — but a hand-rolled u64 < 16! would be unsolvable
~50% of the time.

*/
use bitwise_games::Game;
use bitwise_games::draw_command::{BLUE, DARK_BLUE, DrawCommand, GREEN, WHITE};
use bitwise_games::frame_buffer::FrameBuffer;
use bitwise_games::permutation::{from_permutation, to_permutation};
use minifb::Key;

const SYMBOLS: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

const BOARD_PX: u32 = 480;
const TILE: u32 = BOARD_PX / 4;
const GAP: u32 = 4;
const FONT_SCALE: u32 = 12;
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

fn solved_tiles() -> [u8; 16] {
    let mut t = [0u8; 16];
    for i in 0..15 {
        t[i] = (i + 1) as u8;
    }
    t
}

fn from_u64(state: u64) -> [u8; 16] {
    to_permutation(state, &SYMBOLS).try_into().unwrap()
}

fn to_u64(tiles: &[u8; 16]) -> u64 {
    from_permutation(tiles)
}

fn find_empty(tiles: &[u8; 16]) -> usize {
    tiles.iter().position(|&t| t == 0).unwrap()
}

fn is_solved(tiles: &[u8; 16]) -> bool {
    (0..15).all(|i| tiles[i] == (i + 1) as u8)
}

// Returns the slot that should move into the empty slot for the given arrow.
// Arrow direction = direction the tile slides.
fn source_slot(empty: usize, dir: Key) -> Option<usize> {
    let row = empty / 4;
    let col = empty % 4;
    match dir {
        Key::Up if row < 3 => Some(empty + 4),
        Key::Down if row > 0 => Some(empty - 4),
        Key::Left if col < 3 => Some(empty + 1),
        Key::Right if col > 0 => Some(empty - 1),
        _ => None,
    }
}

fn try_slide(tiles: &mut [u8; 16], dir: Key) -> bool {
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

fn opposite(dir: Key) -> Option<Key> {
    match dir {
        Key::Up => Some(Key::Down),
        Key::Down => Some(Key::Up),
        Key::Left => Some(Key::Right),
        Key::Right => Some(Key::Left),
        _ => None,
    }
}

fn scramble(seed: u64) -> [u8; 16] {
    let dirs = [Key::Up, Key::Down, Key::Left, Key::Right];
    let mut tiles = solved_tiles();
    let mut rng = seed | 1; // avoid zero state
    let mut last_dir: Option<Key> = None;
    let mut moves_made = 0;
    while moves_made < 200 {
        // Numerical Recipes LCG
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let dir = dirs[((rng >> 33) & 0x3) as usize];
        if last_dir.and_then(opposite) == Some(dir) {
            continue;
        }
        if try_slide(&mut tiles, dir) {
            last_dir = Some(dir);
            moves_made += 1;
        }
    }
    tiles
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

fn render(tiles: &[u8; 16]) -> Vec<u32> {
    let mut fb = FrameBuffer::new(BOARD_PX, BOARD_PX);
    let mut commands = Vec::new();

    commands.push(DrawCommand::rect(0, 0, BOARD_PX, BOARD_PX, DARK_BLUE));

    let solved = is_solved(tiles);
    for slot in 0..16 {
        let value = tiles[slot];
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
        let seed = args.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let tiles = scramble(seed);
        (to_u64(&tiles), render(&tiles))
    }

    fn update(state: u64, _held: &[Key], pressed: &[Key]) -> (u64, Vec<u32>) {
        let mut tiles = from_u64(state);
        for dir in [Key::Up, Key::Down, Key::Left, Key::Right] {
            if pressed.contains(&dir) {
                try_slide(&mut tiles, dir);
                break;
            }
        }
        (to_u64(&tiles), render(&tiles))
    }
}

fn main() {
    bitwise_games::run_game::<Fifteen>();
}
