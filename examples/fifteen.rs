/*

15 puzzle on a 4×4 grid.

# Inputs

- Arrow keys: slide the tile in that direction into the empty slot

# Maximize

State space coverage. Of the 16! arrangements of {0, 1, …, 15} (0 =
empty), only half — 16!/2 ≈ 1.05e13 — are reachable from solved (the
parity invariant, see Notes). We fold that bit out and give every
reachable board a unique index 0..16!/2 − 1. The index itself *is* the
state — no bit-packing on top. ⌈log2(16!/2)⌉ = 44 bits used; top 20
unused.

(The full lex rank spans 0..16! − 1 ≈ 2.09e13, which needs 45 bits;
folding the parity bit buys back exactly one, landing at 44. 43 is a
quarter-bit out of reach — there is no second invariant to spend.)

# Encoding

| Start | Length | Description                                                |
|-------|--------|------------------------------------------------------------|
|     0 |     44 | blank_cell × 15!/2 + folded rank of the 15 numbered tiles |
|    44 |     20 | unused                                                     |

# Notes

**Parity invariant.** Counting inversions over the 15 *numbered* tiles
only (blank excluded), `inversions XOR blank_row` is conserved by every
legal slide: a horizontal slide leaves both terms unchanged; a vertical
slide flips both. The solved board has value 1, so reachable boards are
exactly those with invariant 1 — half of all arrangements.

**Folding out the redundant bit.** We factor the blank out first
(`16!/2 = 16 × 15!/2`): the blank cell goes in the high part, and the
low part ranks the numbered tiles. Written in the factorial number
system, the 15-tile lex rank's radix-2 digit — `nums[14] < nums[13]` —
carries the redundant parity. `encode` subtracts it (making the rank
even, since every higher factorial weight is a multiple of 2) and
halves; `decode` doubles, unranks with that digit forced to 0, and
swaps the last two numbered tiles if the invariant says the digit was
really 1. That swap exchanges two numbered tiles without moving the
blank, so it always toggles reachability — the invariant alone resolves
the ambiguity, with no risk of two reachable boards colliding. (Folding
the *full* 16-permutation's last digit instead would collide the solved
board with its one-slide neighbour, since swapping the last two cells
can move the blank — a legal move between two real states.) Result: a
perfect bijection between reachable boards and 0..16!/2 − 1, so a
hand-rolled u64 in range always decodes to a solvable board.

*/
use bitwise_games::draw_command::{BLUE, DARK_BLUE, DrawCommand, GREEN, WHITE};
use bitwise_games::frame_buffer::{self, FrameBuffer};
use bitwise_games::{Game, Key};
use rancor::permutation;

// The 15 numbered tiles (the blank, 0, is factored out and stored separately).
const TILE_SYMBOLS: [u8; 15] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
// 15! / 2 — folded arrangements of the numbered tiles per blank position.
const HALF_15_FACT: u64 = 653_837_184_000;

const BOARD_PX: u32 = frame_buffer::WIDTH;
const TILE: u32 = BOARD_PX / 4;
const GAP: u32 = 2;
const FONT_SCALE: u32 = 2;
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

struct State {
    tiles: [u8; 16],
}

fn solved_board() -> State {
    let mut tiles = [0u8; 16];
    for (i, slot) in tiles.iter_mut().take(15).enumerate() {
        *slot = (i + 1) as u8;
    }
    State { tiles }
}

/// The 15 numbered tiles in row-major order, with the blank skipped.
fn numbered(tiles: &[u8; 16]) -> [u8; 15] {
    let mut out = [0u8; 15];
    let mut k = 0;
    for &t in tiles {
        if t != 0 {
            out[k] = t;
            k += 1;
        }
    }
    out
}

/// Rebuild a board from a blank cell and the numbered tiles (row-major order).
fn assemble(blank: usize, numbered: &[u8; 15]) -> [u8; 16] {
    let mut tiles = [0u8; 16];
    let mut k = 0;
    for (cell, slot) in tiles.iter_mut().enumerate() {
        if cell != blank {
            *slot = numbered[k];
            k += 1;
        }
    }
    tiles
}

/// Whether a board is reachable from the solved state. The conserved invariant
/// is `inversions(numbered tiles) XOR blank_row`, counted over the 15 numbered
/// tiles only — every legal slide preserves it. The solved board has value 1
/// (0 inversions, blank in row 3), so reachable boards are exactly those with
/// invariant 1.
fn reachable(tiles: &[u8; 16]) -> bool {
    let nums = numbered(tiles);
    let mut inversions = 0u32;
    for i in 0..15 {
        for j in (i + 1)..15 {
            inversions += u32::from(nums[j] < nums[i]);
        }
    }
    let blank_row = tiles.iter().position(|&t| t == 0).unwrap() / 4;
    (inversions + blank_row as u32) & 1 == 1
}

fn encode(state: &State) -> u64 {
    // Blank position (16 choices) lives in the high part; the low part is the
    // numbered tiles' lex rank with its redundant parity bit folded out. The
    // radix-2 factorial digit `nums[14] < nums[13]` is that bit: dropping it
    // makes the rank even (every higher factorial weight is a multiple of 2),
    // so halving is lossless. Folding a *numbered-tile* swap never moves the
    // blank, so it always toggles reachability — no two reachable boards collide.
    let blank = find_empty(state) as u64;
    let nums = numbered(&state.tiles);
    let rank = permutation::rank(&nums);
    let last_digit = u64::from(nums[14] < nums[13]);
    blank * HALF_15_FACT + (rank - last_digit) / 2
}

fn decode(code: u64) -> State {
    let blank = (code / HALF_15_FACT) as usize;
    let reduced = code % HALF_15_FACT;
    // Unrank with the dropped digit forced to 0, then let the invariant decide
    // whether it was really 1 — if so, the last two numbered tiles were swapped.
    let mut nums: [u8; 15] = permutation::unrank(reduced * 2, &TILE_SYMBOLS)
        .try_into()
        .unwrap();
    let mut tiles = assemble(blank, &nums);
    if !reachable(&tiles) {
        nums.swap(13, 14);
        tiles = assemble(blank, &nums);
    }
    State { tiles }
}

fn find_empty(state: &State) -> usize {
    state.tiles.iter().position(|&t| t == 0).unwrap()
}

fn is_solved(state: &State) -> bool {
    (0u8..15).all(|i| state.tiles[usize::from(i)] == i + 1)
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

fn try_slide(state: &mut State, dir: Key) -> bool {
    let empty = find_empty(state);
    match source_slot(empty, dir) {
        Some(src) => {
            state.tiles[empty] = state.tiles[src];
            state.tiles[src] = 0;
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

fn scramble(seed: u64) -> State {
    let dirs = [Key::Up, Key::Down, Key::Left, Key::Right];
    let mut state = solved_board();
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
        if try_slide(&mut state, dir) {
            last_dir = Some(dir);
            moves_made += 1;
        }
    }
    state
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

fn render(state: &State) -> FrameBuffer {
    let mut fb = FrameBuffer::new();
    let mut commands = Vec::new();

    commands.push(DrawCommand::rect(0, 0, BOARD_PX, BOARD_PX, DARK_BLUE));

    let solved = is_solved(state);
    for slot in 0..16 {
        let value = state.tiles[slot];
        if value > 0 {
            draw_tile(&mut commands, slot, value, solved);
        }
    }

    fb.draw_list(&commands);
    fb
}

struct Fifteen;

impl Game for Fifteen {
    const NAME: &'static str = "15 Puzzle";
    const FPS: usize = 30;

    fn init(args: Vec<String>) -> (u64, FrameBuffer) {
        let seed = args.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let state = scramble(seed);
        (encode(&state), render(&state))
    }

    fn update(
        state: u64,
        _held: &[Key],
        buffered: Option<Key>,
        _mouse: Option<(u8, u8)>,
    ) -> (u64, FrameBuffer) {
        let mut state = decode(state);
        if let Some(dir @ (Key::Up | Key::Down | Key::Left | Key::Right)) = buffered {
            try_slide(&mut state, dir);
        }
        (encode(&state), render(&state))
    }
}

fn main() {
    bitwise_games::run_game::<Fifteen>();
}

#[cfg(test)]
mod tests {
    use super::*;

    // 16! / 2 — the number of reachable boards and the exclusive upper bound of
    // the encoded state. Fits in 44 bits (2^43 < this <= 2^44).
    const REACHABLE: u64 = 10_461_394_944_000;

    #[test]
    fn reachable_count_fits_in_44_bits() {
        assert!(REACHABLE > (1u64 << 43), "would fit in 43 bits");
        assert!(REACHABLE <= (1u64 << 44), "needs more than 44 bits");
    }

    #[test]
    fn solved_board_is_reachable() {
        assert!(reachable(&solved_board().tiles));
    }

    #[test]
    fn scrambles_round_trip_and_stay_in_range() {
        for seed in 0..2000u64 {
            let board = scramble(seed);
            assert!(reachable(&board.tiles), "scramble {seed} unsolvable");
            let rank = encode(&board);
            assert!(rank < REACHABLE, "seed {seed}: {rank} out of range");
            assert_eq!(decode(rank).tiles, board.tiles, "seed {seed} round-trip");
        }
    }

    #[test]
    fn every_slide_round_trips() {
        let mut board = scramble(1);
        let dirs = [Key::Up, Key::Down, Key::Left, Key::Right];
        let mut rng = 12345u64;
        for _ in 0..5000 {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            try_slide(&mut board, dirs[((rng >> 33) & 3) as usize]);
            assert!(reachable(&board.tiles));
            assert_eq!(decode(encode(&board)).tiles, board.tiles);
        }
    }

    #[test]
    fn arbitrary_ranks_decode_to_solvable_boards() {
        // Any u64 in range must decode to a valid, solvable permutation that
        // encodes back to itself — the property a hand-rolled state relies on.
        let all_symbols: [u8; 16] = std::array::from_fn(|i| i as u8);
        for &rank in &[0, 1, 2, 7, 1000, 1_000_000, REACHABLE / 2, REACHABLE - 1] {
            let board = decode(rank);
            let mut sorted = board.tiles;
            sorted.sort_unstable();
            assert_eq!(sorted, all_symbols, "rank {rank}: not a permutation");
            assert!(reachable(&board.tiles), "rank {rank}: unsolvable");
            assert_eq!(encode(&board), rank, "rank {rank}: no round-trip");
        }
    }
}
