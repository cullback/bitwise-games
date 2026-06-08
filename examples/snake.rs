/*

Snake on a 9×8 grid, with the whole game state packed into one u64.

The snake's body is a directed self-avoiding walk (head → tail). We rank that
walk with `rancor::saw` and store the integer; the apple rides in the low bits.

| Field      | Bits | Description                                              |
|------------|------|----------------------------------------------------------|
| apple_bits | 3    | apple entropy; the value 7 doubles as the "dead" marker  |
| body rank  | ~57  | rancor::saw rank of the head→tail walk (any length ≥ 1)  |

Layout (low → high): apple_bits | body rank. The board has 72 cells, so the
full SAW count (~9.3×10¹⁶ ≈ 2^56.4) plus 3 apple bits fits a u64 with room to
spare — the snake can grow to fill the entire board, no length cap.

`rancor::saw::Saw<9, 8>` builds its ranking table once (~70 ms) on first use;
every rank/unrank after that is microseconds, far inside the 5 FPS budget.
Direction is implicit — head facing = direction(body[0] → head), no bits spent.

*/
use bitwise_games::aseprite::load_color_grid;
use bitwise_games::draw_command::{BLACK, DARK_BLUE, DrawCommand, GREEN, RED, WHITE};
use bitwise_games::font::{digits_of, draw_big_text, tiny};
use bitwise_games::frame_buffer::FrameBuffer;
use bitwise_games::rng;
use bitwise_games::sprite::{Rot, blit_square};
use bitwise_games::{Game, Key};
use rancor::saw::Saw;
use std::sync::OnceLock;

// --- Board / display ---
//
// 9 wide × 8 tall board, 14 px cells, 1 px border all around (score zone
// included). The "missing 9th row" at the top is the score zone. Math:
//   - Outer top:    y =   0      (1 px)
//   - Score:        y =   1..13  (13 px tall, fits a 9×11 digit at y=2..12)
//   - Separator:    y =  14      (1 px, divides score from board)
//   - Board:        y =  15..126 (8 rows × 14 = 112 px)
//   - Bottom:       y = 127      (1 px)
//   - Left border:  x =   0      (1 px, full height)
//   - Board:        x =   1..126 (9 cols × 14 = 126 px)
//   - Right border: x = 127      (1 px, full height)
// → 128 × 128 exactly, no slack.

const BOARD_W: u32 = 9;
const BOARD_H: u32 = 8;
const COLS: usize = BOARD_W as usize;
const ROWS: usize = BOARD_H as usize;
const DISPLAY_PX: u32 = 128;
const CELL_PX: u32 = 14;
const SCORE_H: u32 = 14;
const GAME_W_PX: u32 = BOARD_W * CELL_PX;
const GAME_H_PX: u32 = BOARD_H * CELL_PX;
const GAME_X: u32 = 1;
const GAME_Y: u32 = SCORE_H + 1;

// Maximum body length (cells beyond head). Snake total = MAX_LEN + 1 = full
// board, since the whole SAW-rank space fits a u64 — no count-driven cap.
const MAX_LEN: usize = COLS * ROWS - 1;

// --- Directions (absolute) ---

const DIR_UP: u8 = 0;
const DIR_RIGHT: u8 = 1;
const DIR_DOWN: u8 = 2;
const DIR_LEFT: u8 = 3;

fn opposite(dir: u8) -> u8 {
    (dir + 2) & 3
}

fn step(pos: u8, dir: u8) -> Option<u8> {
    let w = BOARD_W as i32;
    let h = BOARD_H as i32;
    let row = (pos as i32) / w;
    let col = (pos as i32) % w;
    let (dr, dc) = match dir {
        DIR_UP => (-1, 0),
        DIR_RIGHT => (0, 1),
        DIR_DOWN => (1, 0),
        DIR_LEFT => (0, -1),
        _ => return None,
    };
    let nr = row + dr;
    let nc = col + dc;
    if !(0..h).contains(&nr) || !(0..w).contains(&nc) {
        None
    } else {
        Some((nr * w + nc) as u8)
    }
}

fn direction_between(from: u8, to: u8) -> Option<u8> {
    let w = BOARD_W as i32;
    let (fr, fc) = ((from as i32) / w, (from as i32) % w);
    let (tr, tc) = ((to as i32) / w, (to as i32) % w);
    match (tr - fr, tc - fc) {
        (-1, 0) => Some(DIR_UP),
        (1, 0) => Some(DIR_DOWN),
        (0, 1) => Some(DIR_RIGHT),
        (0, -1) => Some(DIR_LEFT),
        _ => None,
    }
}

// --- SAW rank/unrank (via rancor) ---
//
// The body is a directed self-avoiding walk; `rancor::saw` is a bijection
// between such walks and `0..count`. The diagram-order rank isn't sorted by
// length, but we don't need it to be — the rank is an opaque handle, and
// decoding recovers the full body (head, length, and shape) directly.

/// The walk-ranking table for this board, built once on first use (~70 ms).
fn saw() -> &'static Saw<COLS, ROWS> {
    static SAW: OnceLock<Saw<COLS, ROWS>> = OnceLock::new();
    SAW.get_or_init(Saw::new)
}

fn to_coords(cells: &[u8]) -> Vec<(usize, usize)> {
    cells
        .iter()
        .map(|&c| (c as usize / COLS, c as usize % COLS))
        .collect()
}

fn from_coords(coords: &[(usize, usize)]) -> Vec<u8> {
    coords.iter().map(|&(r, c)| (r * COLS + c) as u8).collect()
}

// --- State ---
//
// Packed `u64`:
//   bits 0..APPLE_BITS — apple_bits (7 == dead sentinel)
//   bits APPLE_BITS..  — body rank
//
// A dead snake keeps its full body (so the corpse stays on screen); only the
// apple-bits slot changes, to the DEAD sentinel.

const APPLE_BITS: u32 = 3;
const APPLE_MASK: u64 = (1u64 << APPLE_BITS) - 1;
const DEAD_APPLE_BITS: u8 = 7;
const APPLE_CANDIDATES: u8 = 7;

struct State {
    apple_bits: u8,
    /// Snake cells in head→tail order; `cells[0]` is the head. Always ≥ 2 cells
    /// (the spawn length); a dead snake keeps its full body for the corpse.
    cells: Vec<u8>,
    dead: bool,
}

impl State {
    fn head(&self) -> u8 {
        self.cells[0]
    }

    fn body_len(&self) -> usize {
        self.cells.len().saturating_sub(1)
    }

    /// Direction the head is currently facing (the direction it will move next
    /// tick if no input). Derived from body[0]'s position relative to head.
    fn facing(&self) -> u8 {
        if self.cells.len() < 2 {
            return DIR_RIGHT; // arbitrary fallback for dead state
        }
        // facing = direction from body[0] → head
        direction_between(self.cells[1], self.cells[0]).unwrap_or(DIR_RIGHT)
    }
}

// --- Encoding ---

fn encode(state: &State) -> u64 {
    // Alive and dead use the same walk encoding; a dead snake just stores the
    // DEAD sentinel in the apple slot so the corpse body survives the round-trip.
    let rank = saw().rank(&to_coords(&state.cells));
    let apple = if state.dead {
        DEAD_APPLE_BITS
    } else {
        state.apple_bits
    };
    (rank << APPLE_BITS) | (apple as u64 & APPLE_MASK)
}

fn decode(packed: u64) -> State {
    let apple_bits = (packed & APPLE_MASK) as u8;
    let cells = from_coords(&saw().unrank(packed >> APPLE_BITS));
    State {
        apple_bits,
        cells,
        dead: apple_bits == DEAD_APPLE_BITS,
    }
}

// --- Apple ---

const N_CELLS: u32 = BOARD_W * BOARD_H;

fn apple_cell(snake_length: usize, apple_bits: u8) -> u8 {
    let seed = ((snake_length as u64) << 3) | (apple_bits as u64);
    (rng::next(seed) % N_CELLS as u64) as u8
}

fn pick_apple_bits(seed: u64, snake_cells: &[u8]) -> u8 {
    let mask = snake_cells.iter().fold(0u128, |acc, &c| acc | (1u128 << c));
    let start = (rng::next(seed) % APPLE_CANDIDATES as u64) as u8;
    for offset in 0..APPLE_CANDIDATES {
        let bits = (start + offset) % APPLE_CANDIDATES;
        let cell = apple_cell(snake_cells.len(), bits);
        if (mask >> cell) & 1 == 0 {
            return bits;
        }
    }
    start
}

// --- Rendering ---

fn cell_xy(cell: u8) -> (u32, u32) {
    let row = (cell as u32) / BOARD_W;
    let col = (cell as u32) % BOARD_W;
    (GAME_X + col * CELL_PX, GAME_Y + row * CELL_PX)
}

// --- Snake sprites (loaded from assets/snake.aseprite) ---
//
// Strip of 6 cells, 14×14 each, in order: tail, body, body turn (curving
// from left to down), dead head, alive head, apple. All "directional"
// sprites are drawn at their base orientation and rotated at draw time.

const SPR_TAIL: usize = 0;
const SPR_BODY: usize = 1;
const SPR_TURN: usize = 2;
const SPR_HEAD_DEAD: usize = 3;
const SPR_HEAD_ALIVE: usize = 4;
const SPR_APPLE: usize = 5;

static SNAKE_SPRITES: OnceLock<[[[u8; 14]; 14]; 6]> = OnceLock::new();

fn snake_sprites() -> &'static [[[u8; 14]; 14]; 6] {
    SNAKE_SPRITES
        .get_or_init(|| load_color_grid::<14, 14, 6, 6>(include_bytes!("../assets/snake.aseprite")))
}

/// Rotation for sprites whose base orientation points RIGHT (head + tail).
/// DIR_LEFT uses a horizontal mirror instead of R180 — for a head whose eyes
/// sit near the top, R180 puts the eyes at the bottom (upside-down face).
fn rot_for_right_facing(dir: u8) -> Rot {
    match dir {
        DIR_RIGHT => Rot::R0,
        DIR_DOWN => Rot::R90,
        DIR_LEFT => Rot::FlipH,
        DIR_UP => Rot::R270,
        _ => Rot::R0,
    }
}

/// Rotation for the straight body. Base is horizontal; vertical needs R90.
fn rot_for_body(dir: u8) -> Rot {
    match dir {
        DIR_LEFT | DIR_RIGHT => Rot::R0,
        DIR_UP | DIR_DOWN => Rot::R90,
        _ => Rot::R0,
    }
}

/// Rotation for the corner sprite. Base = body fills LEFT+BOTTOM of the cell
/// (entered from the LEFT side, exits at the BOTTOM). The walking convention:
/// `arrival` is the direction the head→tail walk took *into* this cell, so
/// entering from the LEFT side means `arrival = DIR_RIGHT`.
fn rot_for_turn(arrival: u8, departure: u8) -> Rot {
    match (arrival, departure) {
        // body in LEFT+BOTTOM — base
        (DIR_RIGHT, DIR_DOWN) | (DIR_UP, DIR_LEFT) => Rot::R0,
        // body in TOP+LEFT
        (DIR_DOWN, DIR_LEFT) | (DIR_RIGHT, DIR_UP) => Rot::R90,
        // body in RIGHT+TOP
        (DIR_LEFT, DIR_UP) | (DIR_DOWN, DIR_RIGHT) => Rot::R180,
        // body in BOTTOM+RIGHT
        (DIR_UP, DIR_RIGHT) | (DIR_LEFT, DIR_DOWN) => Rot::R270,
        _ => Rot::R0,
    }
}

fn draw_body_cell(fb: &mut FrameBuffer, cell: u8, arr: u8, dep: u8) {
    let (x, y) = cell_xy(cell);
    let sprites = snake_sprites();
    if arr == dep {
        blit_square(fb, &sprites[SPR_BODY], x, y, rot_for_body(dep));
    } else {
        blit_square(fb, &sprites[SPR_TURN], x, y, rot_for_turn(arr, dep));
    }
}

fn draw_tail_cell(fb: &mut FrameBuffer, cell: u8, body_dir: u8) {
    let (x, y) = cell_xy(cell);
    blit_square(
        fb,
        &snake_sprites()[SPR_TAIL],
        x,
        y,
        rot_for_right_facing(body_dir),
    );
}

fn draw_head_cell(fb: &mut FrameBuffer, cell: u8, dir: u8, dead: bool) {
    let (x, y) = cell_xy(cell);
    let sprite_idx = if dead { SPR_HEAD_DEAD } else { SPR_HEAD_ALIVE };
    blit_square(
        fb,
        &snake_sprites()[sprite_idx],
        x,
        y,
        rot_for_right_facing(dir),
    );
}

fn draw_apple(fb: &mut FrameBuffer, cell: u8) {
    let (x, y) = cell_xy(cell);
    blit_square(fb, &snake_sprites()[SPR_APPLE], x, y, Rot::R0);
}

fn render(state: &State) -> FrameBuffer {
    let mut fb = FrameBuffer::new();
    let mut bg = Vec::new();
    bg.push(DrawCommand::rect(0, 0, DISPLAY_PX, DISPLAY_PX, BLACK));

    let dead = state.dead;
    let cells = &state.cells;
    let facing = state.facing();

    // Score = apples eaten = body_len − 1 (the starting body length is 1).
    // Always drawn so the final score stays visible on the death screen.
    // Right-aligned: rightmost on-pixel sits at x=123 (3 px from inner edge).
    let score = (state.body_len() as u32).saturating_sub(1);
    let score_digits = digits_of(score);
    let score_w = score_digits.len() as u32 * 10 - 1; // 9 px glyph + 1 px gap, minus trailing gap
    let score_x = 124 - score_w;
    draw_big_text(&mut bg, &score_digits, score_x, 2, WHITE);

    // Status text: left-aligned in the score zone, shares row with the score.
    // Tiny font is 6 px tall; vertical-center in the 11-px-tall score line by
    // dropping it 4 px from the score's top.
    let won = !state.dead && state.body_len() >= MAX_LEN;
    if state.dead {
        tiny::draw_text(&mut bg, b"GAME OVER", 4, 4, RED);
    } else if won {
        tiny::draw_text(&mut bg, b"YOU WIN", 4, 4, GREEN);
    }

    bg.push(DrawCommand::rect(
        GAME_X, GAME_Y, GAME_W_PX, GAME_H_PX, DARK_BLUE,
    ));
    // Outer top border (wraps the score zone too).
    bg.push(DrawCommand::rect(0, 0, DISPLAY_PX, 1, WHITE));
    // Score/board separator.
    bg.push(DrawCommand::rect(
        GAME_X - 1,
        GAME_Y - 1,
        GAME_W_PX + 2,
        1,
        WHITE,
    ));
    // Bottom border.
    bg.push(DrawCommand::rect(
        GAME_X - 1,
        GAME_Y + GAME_H_PX,
        GAME_W_PX + 2,
        1,
        WHITE,
    ));
    // Left border (full height).
    bg.push(DrawCommand::rect(0, 0, 1, DISPLAY_PX, WHITE));
    // Right border (full height).
    bg.push(DrawCommand::rect(DISPLAY_PX - 1, 0, 1, DISPLAY_PX, WHITE));

    fb.draw_list(&bg);

    // Walking direction at cells[i]: direction from cells[i] → cells[i+1]
    // (or, equivalently, the "walking-toward-tail" direction at cell i).
    let walking_dirs: Vec<u8> = if cells.len() >= 2 {
        (0..cells.len() - 1)
            .map(|i| direction_between(cells[i], cells[i + 1]).unwrap_or(DIR_UP))
            .collect()
    } else {
        Vec::new()
    };

    // Body cells: index 1..len-1 (excluding head[0] and tail[last]).
    for i in 1..cells.len().saturating_sub(1) {
        let arr = walking_dirs[i - 1];
        let dep = walking_dirs[i];
        draw_body_cell(&mut fb, cells[i], arr, dep);
    }

    // Tail (only if there is a body[0..] beyond the head).
    if cells.len() >= 2 {
        // Tail's body_dir is the direction *from* the tail *toward* the next
        // body cell (i.e., opposite the walking direction).
        let last = cells.len() - 1;
        let body_dir = opposite(walking_dirs[last - 1]);
        draw_tail_cell(&mut fb, cells[last], body_dir);
    }

    draw_head_cell(&mut fb, cells[0], facing, dead);

    if !dead {
        let apple = apple_cell(cells.len(), state.apple_bits);
        draw_apple(&mut fb, apple);
    }

    fb
}

// --- Game logic ---

fn fresh_board(seed: u64) -> State {
    // Spawn centered, facing right with body to the left. Length 2.
    let row = (BOARD_H / 2) as u8;
    let col = (BOARD_W / 2) as u8;
    let head: u8 = row * BOARD_W as u8 + col;
    let body0: u8 = head - 1;
    let cells = vec![head, body0];
    let apple_bits = pick_apple_bits(seed, &cells);
    State {
        apple_bits,
        cells,
        dead: false,
    }
}

struct Snake;

impl Game for Snake {
    const NAME: &'static str = "Snake";
    const FPS: usize = 5;

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

        if state.dead {
            if matches!(buffered, Some(Key::Z) | Some(Key::X)) {
                let next = fresh_board(rng::next(encode(&state)));
                return (encode(&next), render(&next));
            }
            return (encode(&state), render(&state));
        }

        // Won state (max length): freeze, restart on Z/X.
        if state.body_len() >= MAX_LEN {
            if matches!(buffered, Some(Key::Z) | Some(Key::X)) {
                let next = fresh_board(rng::next(encode(&state)));
                return (encode(&next), render(&next));
            }
            return (encode(&state), render(&state));
        }

        let facing = state.facing();
        let candidate = match buffered {
            Some(Key::Up) => Some(DIR_UP),
            Some(Key::Right) => Some(DIR_RIGHT),
            Some(Key::Down) => Some(DIR_DOWN),
            Some(Key::Left) => Some(DIR_LEFT),
            _ => None,
        };
        let new_dir = match candidate {
            Some(d) if d != opposite(facing) => d,
            _ => facing,
        };

        let new_head = match step(state.head(), new_dir) {
            Some(p) => p,
            None => {
                // Tried to walk off the edge — die in place, body stays.
                state.dead = true;
                return (encode(&state), render(&state));
            }
        };

        let current_apple = apple_cell(state.cells.len(), state.apple_bits);
        let ate = new_head == current_apple;

        // Self-collision: check against body. The tail will vacate this tick
        // unless we grew, so exclude the tail cell from the check in that case.
        let check_end = if ate {
            state.cells.len()
        } else {
            state.cells.len() - 1
        };
        if state.cells[..check_end].contains(&new_head) {
            // Walked into body — die in place, body stays.
            state.dead = true;
            return (encode(&state), render(&state));
        }

        // Build new cells: prepend new_head, drop tail unless growing.
        let mut new_cells = Vec::with_capacity(state.cells.len() + 1);
        new_cells.push(new_head);
        let keep = if ate {
            state.cells.len()
        } else {
            state.cells.len() - 1
        };
        new_cells.extend_from_slice(&state.cells[..keep]);

        // Don't exceed MAX_LEN cells of body (head + MAX_LEN total = MAX_LEN+1 cells).
        if new_cells.len() > MAX_LEN + 1 {
            new_cells.truncate(MAX_LEN + 1);
        }

        let new_apple_bits = if ate {
            let seed = (new_head as u64) ^ ((new_cells.len() as u64) << 8);
            pick_apple_bits(seed, &new_cells)
        } else {
            state.apple_bits
        };

        state.apple_bits = new_apple_bits;
        state.cells = new_cells;
        (encode(&state), render(&state))
    }
}

fn main() {
    bitwise_games::run_game::<Snake>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snake(cells: Vec<u8>) -> State {
        State {
            apple_bits: 5,
            cells,
            dead: false,
        }
    }

    fn cell(r: u32, c: u32) -> u8 {
        (r * BOARD_W + c) as u8
    }

    #[test]
    fn round_trip_length_2() {
        // Length-2 snakes (head + one body cell) — each pair must be adjacent.
        let cases: &[(u8, u8)] = &[
            (cell(0, 0), cell(0, 1)),
            (cell(3, 3), cell(3, 2)),
            (cell(4, 3), cell(3, 3)),
            (
                cell(BOARD_H - 1, BOARD_W - 1),
                cell(BOARD_H - 1, BOARD_W - 2),
            ),
            (cell(1, 0), cell(0, 0)),
        ];
        for &(head, body0) in cases {
            let s = snake(vec![head, body0]);
            let d = decode(encode(&s));
            assert!(!d.dead);
            assert_eq!(d.cells, s.cells, "round-trip failed for {:?}", s.cells);
            assert_eq!(d.apple_bits, s.apple_bits);
        }
    }

    #[test]
    fn round_trip_longer_snakes() {
        // A few hand-rolled medium snakes built via `cell(r, c)`.
        let cases: Vec<Vec<u8>> = vec![
            vec![cell(3, 3), cell(3, 2), cell(3, 1), cell(3, 0)],
            vec![cell(3, 3), cell(3, 2), cell(3, 1), cell(4, 1), cell(4, 2)],
            vec![
                cell(0, 0),
                cell(0, 1),
                cell(0, 2),
                cell(1, 2),
                cell(2, 2),
                cell(2, 1),
                cell(2, 0),
                cell(3, 0),
                cell(3, 1),
            ],
        ];
        for cells in &cases {
            let s = snake(cells.clone());
            let d = decode(encode(&s));
            assert!(!d.dead);
            assert_eq!(&d.cells, cells, "round-trip failed for {cells:?}");
        }
    }

    #[test]
    fn dead_round_trip() {
        // A dead snake keeps its full body; `dead` rides on the apple-bits
        // sentinel, which decode reports as DEAD_APPLE_BITS.
        let s = State {
            apple_bits: 3,
            cells: vec![cell(3, 3), cell(3, 2), cell(3, 1)],
            dead: true,
        };
        let d = decode(encode(&s));
        assert!(d.dead);
        assert_eq!(d.cells, s.cells);
        assert_eq!(d.apple_bits, DEAD_APPLE_BITS);
    }

    #[test]
    fn full_board_hamiltonian_round_trips() {
        // A boustrophedon (snaking) path covering all 72 cells — the longest
        // possible body. Confirms the full SAW range round-trips and fits.
        let mut cells = Vec::new();
        for r in 0..BOARD_H {
            if r % 2 == 0 {
                for c in 0..BOARD_W {
                    cells.push(cell(r, c));
                }
            } else {
                for c in (0..BOARD_W).rev() {
                    cells.push(cell(r, c));
                }
            }
        }
        assert_eq!(cells.len(), (BOARD_W * BOARD_H) as usize);
        let s = snake(cells.clone());
        let d = decode(encode(&s));
        assert_eq!(d.cells, cells, "Hamiltonian path did not round-trip");
    }
}
