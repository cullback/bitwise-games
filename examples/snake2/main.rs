/*

Snake on an 8×8 grid (variant 2: SAW-rank encoding, proof of concept).

# Idea

Encode the snake's body as a *rank* in the space of self-avoiding walks (SAWs)
on the grid. Each (head_cell, body_length, body_shape) triple gets exactly one
integer in [0, cum[head][MAX_LEN]); decoding navigates the count tree of valid
SAW extensions step by step.

Direction is implicit — head facing = direction(body[0] → head). No bits spent
on a direction field.

# Encoding

| Field      | Bits | Description                                       |
|------------|------|---------------------------------------------------|
| apple_bits | 3    | apple entropy (same scheme as snake.rs)           |
| head cell  | 6    | row*8 + col                                       |
| body rank  | 55   | rank in [0, cum[head][MAX_LEN]) for alive states; |
|            |      | ≥ cum[head][MAX_LEN] for the DEAD sentinel        |

Layout (low → high): apple_bits | head | body rank.

# Status

Proof of concept. `count_extensions` is backtracking with three optimizations:
inlined base cases (remaining = 0..3), an early "unvisited cells fewer than
remaining" prune, and a reachability BFS prune that triggers once the grid
is dense enough to make BFS pay off. `cum_table` initialization is
parallelized via `thread::scope` (one thread per starting cell — natural
load balancing since corners finish quickly and the OS schedules the heavy
center cells across cores).

With those, MAX_LEN = 22 fits comfortably in the 5 FPS frame budget (decode
≈ 65 ms worst-case on a tight zigzag). Cum-table init ≈ 3.5 s on first call.

To push MAX_LEN past ~22 within budget would require a transfer-matrix
based count function (frontier-state matchings on a column scan-line). The
encoding scheme itself supports MAX_LEN well past 50 — it's the count
function that's the bottleneck, not the bit math.

Max snake length here: 23 (head + 22 body cells). Max score: 21. Below
the current snake.rs ceiling of 33 — the SAW-rank scheme has more
theoretical headroom but needs faster counts to claim it.

*/
use bitwise_games::draw_command::{BLACK, Color, DARK_BLUE, DrawCommand, GREEN, RED, WHITE};
use bitwise_games::font::{digits_of, draw_text, text_width};
use bitwise_games::frame_buffer::FrameBuffer;
use bitwise_games::rng;
use bitwise_games::{Game, Key};

mod saw_dp;
mod saw_rank;
mod saw_tables;
mod zdd;

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
const DISPLAY_PX: u32 = 128;
const CELL_PX: u32 = 14;
const SCORE_H: u32 = 14;
const GAME_W_PX: u32 = BOARD_W * CELL_PX;
const GAME_H_PX: u32 = BOARD_H * CELL_PX;
const GAME_X: u32 = 1;
const GAME_Y: u32 = SCORE_H + 1;
const FONT_SCALE: u32 = 2;
const BANNER_SCALE: u32 = 2;

// Maximum body length (cells beyond head). Snake total length = MAX_LEN + 1.
// We use the polynomial `saw_dp` counter for in-path conditional counts and
// const-baked cumulative tables for the length/start peel, so MAX_LEN can be
// the full Hamiltonian (W*H − 1) without exponential build or runtime cost.
const MAX_LEN: usize = saw_tables::MAX_LENGTH;

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

// --- SAW rank/unrank ---
//
// Ordering is (length, start_cell, lex-of-next-choices) so length and head
// are recoverable from the rank — no separate fields needed.
//
//   - Cumulative tables (`saw_tables::CUM_LENGTHS` and `CUM_PER_START`) are
//     precomputed and baked in as `const`, so startup is instant.
//   - In-path conditional counts during encode/decode use the polynomial
//     frontier DP (`saw_dp::count_saws_for::<W, H>`) — sub-millisecond per
//     query at any length, including the full Hamiltonian.

const N_CELLS_USIZE: usize = (BOARD_W * BOARD_H) as usize;

fn count_extensions(start: u8, forbidden: u128, remaining: usize) -> u64 {
    if remaining == 0 {
        return 1;
    }
    saw_dp::count_saws_for::<{ BOARD_W as usize }, { BOARD_H as usize }>(
        start, remaining, forbidden,
    )
}

fn neighbors_of(cell: u8) -> impl Iterator<Item = u8> {
    let w = BOARD_W as i32;
    let h = BOARD_H as i32;
    let r = (cell as i32) / w;
    let c = (cell as i32) % w;
    let mut out = [None, None, None, None];
    if r > 0 {
        out[0] = Some((cell as i32 - w) as u8);
    }
    if c > 0 {
        out[1] = Some(cell - 1);
    }
    if c + 1 < w {
        out[2] = Some(cell + 1);
    }
    if r + 1 < h {
        out[3] = Some((cell as i32 + w) as u8);
    }
    out.into_iter().flatten()
}

fn rank_of_path(path: &[u8]) -> u64 {
    let length = path.len() - 1;
    let start = path[0] as usize;
    let mut rank = saw_tables::CUM_LENGTHS[length];
    for s in 0..start {
        rank += saw_tables::CUM_PER_START[s][length];
    }
    let mut visited: u128 = 1u128 << start;
    let mut current = path[0];
    for step in 1..=length {
        let actual = path[step];
        for next in neighbors_of(current) {
            if (visited >> next) & 1 != 0 {
                continue;
            }
            if next == actual {
                break;
            }
            rank += count_extensions(next, visited, length - step);
        }
        visited |= 1u128 << actual;
        current = actual;
    }
    rank
}

fn path_at_rank(mut rank: u64) -> Vec<u8> {
    // Peel length.
    let mut length = 0;
    while length < MAX_LEN && rank >= saw_tables::CUM_LENGTHS[length + 1] {
        length += 1;
    }
    rank -= saw_tables::CUM_LENGTHS[length];

    // Peel start.
    let mut start = 0;
    while start < N_CELLS_USIZE && rank >= saw_tables::CUM_PER_START[start][length] {
        rank -= saw_tables::CUM_PER_START[start][length];
        start += 1;
    }

    // Walk the path.
    let mut path = Vec::with_capacity(length + 1);
    path.push(start as u8);
    let mut visited: u128 = 1u128 << start;
    let mut current = start as u8;
    let mut remaining = length;
    while remaining > 0 {
        let mut picked = None;
        for next in neighbors_of(current) {
            if (visited >> next) & 1 != 0 {
                continue;
            }
            let sub_count = count_extensions(next, visited, remaining - 1);
            if rank < sub_count {
                picked = Some(next);
                break;
            }
            rank -= sub_count;
        }
        let next = picked.expect("decode exhausted");
        path.push(next);
        visited |= 1u128 << next;
        current = next;
        remaining -= 1;
    }
    path
}

// --- State ---
//
// Packed `u64`:
//   bits 0..APPLE_BITS — apple_bits
//   bits APPLE_BITS..  — rank
//
// `apple_bits == DEAD_APPLE_BITS` (7) signals a dead snake; the rank field
// then holds the head cell index (where to draw the corpse). Live states
// use apple_bits ∈ 0..7 (seven candidates for the apple-cell hash).

const APPLE_BITS: u32 = 3;
const APPLE_MASK: u64 = (1u64 << APPLE_BITS) - 1;
const DEAD_APPLE_BITS: u8 = 7;
const APPLE_CANDIDATES: u8 = 7;

struct State {
    apple_bits: u8,
    /// Snake cells in head→tail order. cells[0] is the head.
    /// Alive: len ≥ 2. Dead: len = 1 (just the head; body collapsed away).
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
    // Dead and alive states use the same `rank_of_path` encoding; the only
    // difference is that the apple-bits slot holds the DEAD sentinel for a
    // dead snake. This way the corpse body stays on screen.
    let rank = rank_of_path(&state.cells);
    let apple = if state.dead {
        DEAD_APPLE_BITS
    } else {
        state.apple_bits
    };
    (rank << APPLE_BITS) | (apple as u64 & APPLE_MASK)
}

fn decode(state: u64) -> State {
    let apple_bits = (state & APPLE_MASK) as u8;
    let rank = state >> APPLE_BITS;
    let cells = path_at_rank(rank);
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
    // Score in the 14 px top zone. FONT_SCALE=2 glyphs are 6×10, so y=2
    // centers them vertically (2 px above, 2 px below).
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

    fn new(args: Vec<String>) -> (u64, FrameBuffer) {
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
            let enc = encode(&s);
            let d = decode(enc);
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
            let enc = encode(&s);
            let d = decode(enc);
            assert!(!d.dead);
            assert_eq!(&d.cells, cells, "round-trip failed for {cells:?}");
        }
    }

    #[test]
    fn dead_round_trip() {
        // The dead state ignores the input `apple_bits` — the encoding always
        // sets it to `DEAD_APPLE_BITS` as the sentinel that marks "dead".
        let s = State {
            apple_bits: 3,
            cells: vec![42],
            dead: true,
        };
        let enc = encode(&s);
        let d = decode(enc);
        assert!(d.dead);
        assert_eq!(d.cells, vec![42]);
        assert_eq!(d.apple_bits, DEAD_APPLE_BITS);
    }
}
