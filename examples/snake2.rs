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
use bitwise_games::draw_command::{BLACK, Color, DARK_BLUE, DrawCommand, GREEN, RED, WHITE};
use bitwise_games::font::{digits_of, draw_text, text_width};
use bitwise_games::frame_buffer::FrameBuffer;
use bitwise_games::rng;
use bitwise_games::{Game, Key};
use rancor::saw::Saw;
use std::sync::OnceLock;

// --- Board / display ---
//
// 9 wide × 8 tall board, 14 px cells, 1 px border all around. The "missing
// 9th row" at the top (14 px) is the score zone. Math:
//   - Score:        y =   0..13  (14 px tall, FONT_SCALE=2 fits a 10×14 digit)
//   - Top border:   y =  14      (1 px)
//   - Board:        y =  15..126 (8 rows × 14 = 112 px)
//   - Bottom border:y = 127      (1 px)
//   - Left border:  x =   0      (1 px)
//   - Board:        x =   1..126 (9 cols × 14 = 126 px)
//   - Right border: x = 127      (1 px)
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
const FONT_SCALE: u32 = 2;
const BANNER_SCALE: u32 = 2;

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

// --- Apple (same scheme as snake.rs) ---

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

fn draw_number(commands: &mut Vec<DrawCommand>, n: u32, x: u32, y: u32) {
    draw_text(commands, &digits_of(n), x, y, FONT_SCALE, WHITE);
}

#[derive(Copy, Clone)]
enum Corner {
    TL,
    TR,
    BR,
    BL,
}

fn outer_corner(arrival: u8, departure: u8) -> Corner {
    match (arrival, departure) {
        (DIR_DOWN, DIR_RIGHT) | (DIR_LEFT, DIR_UP) => Corner::BL,
        (DIR_DOWN, DIR_LEFT) | (DIR_RIGHT, DIR_UP) => Corner::BR,
        (DIR_UP, DIR_RIGHT) | (DIR_LEFT, DIR_DOWN) => Corner::TL,
        (DIR_UP, DIR_LEFT) | (DIR_RIGHT, DIR_DOWN) => Corner::TR,
        _ => Corner::TL,
    }
}

fn draw_body_cell(commands: &mut Vec<DrawCommand>, cell: u8, rounded: Option<Corner>) {
    let (x, y) = cell_xy(cell);
    commands.push(DrawCommand::rect(x, y, CELL_PX, CELL_PX, GREEN));
    if let Some(corner) = rounded {
        let (cx, cy, dx, dy) = match corner {
            Corner::TL => (x, y, 1i32, 1i32),
            Corner::TR => (x + CELL_PX - 1, y, -1, 1),
            Corner::BR => (x + CELL_PX - 1, y + CELL_PX - 1, -1, -1),
            Corner::BL => (x, y + CELL_PX - 1, 1, -1),
        };
        commands.push(DrawCommand::rect(cx, cy, 1, 1, DARK_BLUE));
        commands.push(DrawCommand::rect(
            (cx as i32 + dx) as u32,
            cy,
            1,
            1,
            DARK_BLUE,
        ));
        commands.push(DrawCommand::rect(
            cx,
            (cy as i32 + dy) as u32,
            1,
            1,
            DARK_BLUE,
        ));
    }
}

/// Triangular notch depth at row/column `i` of a tail cell. The notch is
/// `i` deep at the edges and `CELL_PX/2 - 1` at the middle.
fn tail_notch_depth(i: u32) -> u32 {
    i.min(CELL_PX - 1 - i)
}

fn draw_tail_cell(commands: &mut Vec<DrawCommand>, cell: u8, body_dir: u8) {
    let (x, y) = cell_xy(cell);
    for i in 0..CELL_PX {
        let depth = tail_notch_depth(i);
        let span = CELL_PX - depth;
        match body_dir {
            DIR_RIGHT => commands.push(DrawCommand::rect(x + depth, y + i, span, 1, GREEN)),
            DIR_LEFT => commands.push(DrawCommand::rect(x, y + i, span, 1, GREEN)),
            DIR_DOWN => commands.push(DrawCommand::rect(x + i, y + depth, 1, span, GREEN)),
            DIR_UP => commands.push(DrawCommand::rect(x + i, y, 1, span, GREEN)),
            _ => {}
        }
    }
}

fn draw_head_cell(commands: &mut Vec<DrawCommand>, cell: u8, dir: u8, dead: bool) {
    let (x, y) = cell_xy(cell);
    // Body strip + rounded corners. All offsets are CELL_PX-relative so the
    // shape scales cleanly with cell size.
    commands.push(DrawCommand::rect(x, y + 2, CELL_PX, CELL_PX - 4, GREEN));
    commands.push(DrawCommand::rect(x + 1, y + 1, CELL_PX - 2, 1, GREEN));
    commands.push(DrawCommand::rect(
        x + 1,
        y + CELL_PX - 2,
        CELL_PX - 2,
        1,
        GREEN,
    ));
    commands.push(DrawCommand::rect(x + 2, y, CELL_PX - 4, 1, GREEN));
    commands.push(DrawCommand::rect(
        x + 2,
        y + CELL_PX - 1,
        CELL_PX - 4,
        1,
        GREEN,
    ));

    // Eyes: 2×2 black squares. "Near" coordinate = 3 from edge, "far" = CELL_PX - 5.
    let near = 3u32;
    let far = CELL_PX - 5;
    let (e1, e2) = match dir {
        DIR_UP => ((x + near, y + near), (x + far, y + near)),
        DIR_RIGHT => ((x + far, y + near), (x + far, y + far)),
        DIR_DOWN => ((x + near, y + far), (x + far, y + far)),
        DIR_LEFT => ((x + near, y + near), (x + near, y + far)),
        _ => return,
    };
    if dead {
        draw_x_eye(commands, e1.0, e1.1);
        draw_x_eye(commands, e2.0, e2.1);
    } else {
        commands.push(DrawCommand::rect(e1.0, e1.1, 2, 2, BLACK));
        commands.push(DrawCommand::rect(e2.0, e2.1, 2, 2, BLACK));
    }
}

fn draw_x_eye(commands: &mut Vec<DrawCommand>, x: u32, y: u32) {
    commands.push(DrawCommand::rect(x, y, 1, 1, BLACK));
    commands.push(DrawCommand::rect(x + 2, y, 1, 1, BLACK));
    commands.push(DrawCommand::rect(x + 1, y + 1, 1, 1, BLACK));
    commands.push(DrawCommand::rect(x, y + 2, 1, 1, BLACK));
    commands.push(DrawCommand::rect(x + 2, y + 2, 1, 1, BLACK));
}

fn draw_apple(commands: &mut Vec<DrawCommand>, cell: u8) {
    let (x, y) = cell_xy(cell);
    commands.push(DrawCommand::rect(
        x + 3,
        y + 3,
        CELL_PX - 6,
        CELL_PX - 6,
        RED,
    ));
}

fn draw_banner(commands: &mut Vec<DrawCommand>, line1: &[u8], line2: &[u8], color: Color) {
    let line1_w = text_width(line1.len(), BANNER_SCALE);
    let line2_w = text_width(line2.len(), BANNER_SCALE);
    let banner_w = line1_w.max(line2_w) + 8;
    let banner_h = 36;
    let banner_x = GAME_X + (GAME_W_PX - banner_w) / 2;
    let banner_y = GAME_Y + (GAME_H_PX - banner_h) / 2;

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
    commands.push(DrawCommand::rect(0, 0, DISPLAY_PX, DISPLAY_PX, BLACK));

    let dead = state.dead;
    let cells = &state.cells;
    let facing = state.facing();

    // Score = apples eaten = body_len − 1 (the starting body length is 1).
    // Always drawn so the final score stays visible on the death screen.
    let score = (state.body_len() as u32).saturating_sub(1);
    draw_number(&mut commands, score, 4, 2);

    commands.push(DrawCommand::rect(
        GAME_X, GAME_Y, GAME_W_PX, GAME_H_PX, DARK_BLUE,
    ));
    // Top border.
    commands.push(DrawCommand::rect(
        GAME_X - 1,
        GAME_Y - 1,
        GAME_W_PX + 2,
        1,
        WHITE,
    ));
    // Bottom border.
    commands.push(DrawCommand::rect(
        GAME_X - 1,
        GAME_Y + GAME_H_PX,
        GAME_W_PX + 2,
        1,
        WHITE,
    ));
    // Left border.
    commands.push(DrawCommand::rect(
        GAME_X - 1,
        GAME_Y - 1,
        1,
        GAME_H_PX + 2,
        WHITE,
    ));
    // Right border.
    commands.push(DrawCommand::rect(
        GAME_X + GAME_W_PX,
        GAME_Y - 1,
        1,
        GAME_H_PX + 2,
        WHITE,
    ));

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
        let rounded = if arr != dep {
            Some(outer_corner(arr, dep))
        } else {
            None
        };
        draw_body_cell(&mut commands, cells[i], rounded);
    }

    // Tail (only if there is a body[0..] beyond the head).
    if cells.len() >= 2 {
        // Tail's body_dir is the direction *from* the tail *toward* the next
        // body cell (i.e., opposite the walking direction).
        let last = cells.len() - 1;
        let body_dir = opposite(walking_dirs[last - 1]);
        draw_tail_cell(&mut commands, cells[last], body_dir);
    }

    draw_head_cell(&mut commands, cells[0], facing, dead);

    if !dead {
        let apple = apple_cell(cells.len(), state.apple_bits);
        draw_apple(&mut commands, apple);
    }

    let won = !dead && state.body_len() >= MAX_LEN;
    if dead {
        draw_banner(&mut commands, b"GAME OVER", b"PRESS Z", RED);
    } else if won {
        draw_banner(&mut commands, b"YOU WIN", b"PRESS Z", GREEN);
    }

    fb.draw_list(&commands);
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

struct Snake2;

impl Game for Snake2 {
    const NAME: &'static str = "Snake2";
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
    bitwise_games::run_game::<Snake2>();
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
