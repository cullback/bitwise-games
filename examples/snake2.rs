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

use std::sync::OnceLock;

// --- Board / display ---

const BOARD_CELLS: u32 = 8;
const N_CELLS: usize = 64;
const DISPLAY_PX: u32 = 128;
const CELL_PX: u32 = 12;
const GAME_X: u32 = 16;
const GAME_Y: u32 = 24;
const GAME_SIZE: u32 = BOARD_CELLS * CELL_PX;
const FONT_SCALE: u32 = 3;
const BANNER_SCALE: u32 = 2;

// Maximum body length (cells beyond head). Snake total length = MAX_LEN + 1.
// Capped low because count_extensions is naive — see top of file.
const MAX_LEN: usize = 22;

// --- Directions (absolute) ---

const DIR_UP: u8 = 0;
const DIR_RIGHT: u8 = 1;
const DIR_DOWN: u8 = 2;
const DIR_LEFT: u8 = 3;

fn opposite(dir: u8) -> u8 {
    (dir + 2) & 3
}

fn step(pos: u8, dir: u8) -> Option<u8> {
    let row = (pos / 8) as i32;
    let col = (pos % 8) as i32;
    let (dr, dc) = match dir {
        DIR_UP => (-1, 0),
        DIR_RIGHT => (0, 1),
        DIR_DOWN => (1, 0),
        DIR_LEFT => (0, -1),
        _ => return None,
    };
    let nr = row + dr;
    let nc = col + dc;
    if !(0..8).contains(&nr) || !(0..8).contains(&nc) {
        None
    } else {
        Some((nr * 8 + nc) as u8)
    }
}

fn direction_between(from: u8, to: u8) -> Option<u8> {
    let (fr, fc) = ((from / 8) as i32, (from % 8) as i32);
    let (tr, tc) = ((to / 8) as i32, (to % 8) as i32);
    match (tr - fr, tc - fc) {
        (-1, 0) => Some(DIR_UP),
        (1, 0) => Some(DIR_DOWN),
        (0, 1) => Some(DIR_RIGHT),
        (0, -1) => Some(DIR_LEFT),
        _ => None,
    }
}

// --- Neighbor bitmasks ---

const NEIGHBOR_MASKS: [u64; N_CELLS] = {
    let mut masks = [0u64; N_CELLS];
    let mut i = 0;
    while i < N_CELLS {
        let r = (i / 8) as isize;
        let c = (i % 8) as isize;
        let mut mask = 0u64;
        if r > 0 {
            mask |= 1u64 << ((r as usize - 1) * 8 + c as usize);
        }
        if r < 7 {
            mask |= 1u64 << ((r as usize + 1) * 8 + c as usize);
        }
        if c > 0 {
            mask |= 1u64 << (r as usize * 8 + (c as usize - 1));
        }
        if c < 7 {
            mask |= 1u64 << (r as usize * 8 + (c as usize + 1));
        }
        masks[i] = mask;
        i += 1;
    }
    masks
};

// --- SAW counting (frontier-state DP via the shared module) ---

/// Count SAWs of exactly `remaining` more steps from `pos`, avoiding `visited`.
/// `visited` should include `pos` (the snake convention) — we strip it before
/// passing to the DP, which treats `pos` as the start cell rather than as a
/// forbidden cell.
fn count_extensions(pos: u8, visited: u64, remaining: usize) -> u64 {
    bitwise_games::saw_dp::count_saws(pos, remaining, visited & !(1u64 << pos))
}

// --- Cumulative count table per head ---

/// cum[head][L] = total number of SAWs of body length 1..=L starting from
/// `head`. cum[head][0] = 0. cum[head][MAX_LEN] is the live-encoding range
/// upper bound; rank values ≥ that are the DEAD sentinel.
type CumTable = [[u64; MAX_LEN + 1]; N_CELLS];

fn compute_cum_for_head(head: u8) -> [u64; MAX_LEN + 1] {
    let mut by_len = [0u64; MAX_LEN + 1];
    fn dfs(pos: u8, visited: u64, by_len: &mut [u64], len: usize, max_len: usize) {
        by_len[len] += 1;
        if len == max_len {
            return;
        }
        let mut cand = NEIGHBOR_MASKS[pos as usize] & !visited;
        while cand != 0 {
            let next = cand.trailing_zeros() as u8;
            cand &= cand - 1;
            dfs(next, visited | (1u64 << next), by_len, len + 1, max_len);
        }
    }
    dfs(head, 1u64 << head, &mut by_len, 0, MAX_LEN);
    let mut cum = [0u64; MAX_LEN + 1];
    let mut accum = 0u64;
    for l in 1..=MAX_LEN {
        accum += by_len[l];
        cum[l] = accum;
    }
    cum
}

fn cum_table() -> &'static CumTable {
    static TABLE: OnceLock<CumTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        // Parallel init: each thread handles one head. The center cells take
        // ~100x longer than corners, so 1-thread-per-head with OS scheduling
        // load-balances naturally.
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..N_CELLS)
                .map(|head| s.spawn(move || compute_cum_for_head(head as u8)))
                .collect();
            let mut table: CumTable = [[0; MAX_LEN + 1]; N_CELLS];
            for (head, h) in handles.into_iter().enumerate() {
                table[head] = h.join().unwrap();
            }
            table
        })
    })
}

// --- State ---

const HEAD_BITS: u32 = 6;
const APPLE_BITS: u32 = 3;
const APPLE_MASK: u64 = (1u64 << APPLE_BITS) - 1;
const HEAD_MASK: u64 = (1u64 << HEAD_BITS) - 1;
const RANK_SHIFT: u32 = APPLE_BITS + HEAD_BITS; // 9

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
        if self.dead { 0 } else { self.cells.len() - 1 }
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
    let head = state.head();
    let cum = &cum_table()[head as usize];

    let body_rank = if state.dead {
        cum[MAX_LEN] // DEAD sentinel (just past the live range)
    } else {
        let body_len = state.body_len();
        // Length-prefix offset: everything below this length.
        let mut rank: u64 = cum[body_len - 1];

        // Walk the path; at each step add counts of candidates that come
        // before the actual move in canonical (cell-index) order.
        let mut visited: u64 = 1u64 << head;
        let mut current = head;
        let mut remaining = body_len;
        for &actual in &state.cells[1..] {
            let mut cand = NEIGHBOR_MASKS[current as usize] & !visited;
            while cand != 0 {
                let c = cand.trailing_zeros() as u8;
                cand &= cand - 1;
                if c == actual {
                    break;
                }
                rank += count_extensions(c, visited | (1u64 << c), remaining - 1);
            }
            visited |= 1u64 << actual;
            current = actual;
            remaining -= 1;
        }
        rank
    };

    (state.apple_bits as u64 & APPLE_MASK)
        | ((head as u64 & HEAD_MASK) << APPLE_BITS)
        | (body_rank << RANK_SHIFT)
}

fn decode(state: u64) -> State {
    let apple_bits = (state & APPLE_MASK) as u8;
    let head = ((state >> APPLE_BITS) & HEAD_MASK) as u8;
    let mut rank = state >> RANK_SHIFT;

    let cum = &cum_table()[head as usize];

    if rank >= cum[MAX_LEN] {
        return State {
            apple_bits,
            cells: vec![head],
            dead: true,
        };
    }

    // Find body length L such that cum[L-1] ≤ rank < cum[L].
    let mut length = 1;
    while length <= MAX_LEN && cum[length] <= rank {
        length += 1;
    }
    rank -= cum[length - 1];

    // Walk the rank tree.
    let mut cells = Vec::with_capacity(length + 1);
    cells.push(head);
    let mut visited: u64 = 1u64 << head;
    let mut current = head;
    let mut remaining = length;
    while remaining > 0 {
        let mut cand = NEIGHBOR_MASKS[current as usize] & !visited;
        let mut picked = None;
        while cand != 0 {
            let c = cand.trailing_zeros() as u8;
            cand &= cand - 1;
            let sub = count_extensions(c, visited | (1u64 << c), remaining - 1);
            if rank < sub {
                picked = Some(c);
                break;
            }
            rank -= sub;
        }
        let next = picked.expect("rank tree exhausted — encoding invariant violated");
        cells.push(next);
        visited |= 1u64 << next;
        current = next;
        remaining -= 1;
    }

    State {
        apple_bits,
        cells,
        dead: false,
    }
}

// --- Apple (same scheme as snake.rs) ---

fn apple_cell(snake_length: usize, apple_bits: u8) -> u8 {
    let seed = ((snake_length as u64) << 3) | (apple_bits as u64);
    (rng::next(seed) % 64) as u8
}

fn pick_apple_bits(seed: u64, snake_cells: &[u8]) -> u8 {
    let mask = snake_cells.iter().fold(0u64, |acc, &c| acc | (1u64 << c));
    let start = (rng::next(seed) & APPLE_MASK as u64) as u8;
    for offset in 0..8u8 {
        let bits = (start + offset) & APPLE_MASK as u8;
        let cell = apple_cell(snake_cells.len(), bits);
        if (mask >> cell) & 1 == 0 {
            return bits;
        }
    }
    start
}

// --- Rendering ---

fn cell_xy(cell: u8) -> (u32, u32) {
    let row = (cell / 8) as u32;
    let col = (cell % 8) as u32;
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

const TAIL_NOTCH: [u32; 12] = [0, 1, 2, 3, 4, 5, 5, 4, 3, 2, 1, 0];

fn draw_tail_cell(commands: &mut Vec<DrawCommand>, cell: u8, body_dir: u8) {
    let (x, y) = cell_xy(cell);
    for i in 0..CELL_PX {
        let depth = TAIL_NOTCH[i as usize];
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
    commands.push(DrawCommand::rect(x, y + 2, CELL_PX, CELL_PX - 4, GREEN));
    commands.push(DrawCommand::rect(x + 1, y + 1, CELL_PX - 2, 1, GREEN));
    commands.push(DrawCommand::rect(x + 1, y + 10, CELL_PX - 2, 1, GREEN));
    commands.push(DrawCommand::rect(x + 2, y, CELL_PX - 4, 1, GREEN));
    commands.push(DrawCommand::rect(x + 2, y + 11, CELL_PX - 4, 1, GREEN));

    let (e1, e2) = match dir {
        DIR_UP => ((x + 3, y + 3), (x + 7, y + 3)),
        DIR_RIGHT => ((x + 7, y + 3), (x + 7, y + 7)),
        DIR_DOWN => ((x + 3, y + 7), (x + 7, y + 7)),
        DIR_LEFT => ((x + 3, y + 3), (x + 3, y + 7)),
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
    let banner_x = GAME_X + (GAME_SIZE - banner_w) / 2;
    let banner_y = GAME_Y + (GAME_SIZE - banner_h) / 2;

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

    if !dead {
        draw_number(&mut commands, state.body_len() as u32 - 1, 4, 4);
    }

    commands.push(DrawCommand::rect(
        GAME_X, GAME_Y, GAME_SIZE, GAME_SIZE, DARK_BLUE,
    ));
    commands.push(DrawCommand::rect(
        GAME_X - 1,
        GAME_Y - 1,
        GAME_SIZE + 2,
        1,
        WHITE,
    ));
    commands.push(DrawCommand::rect(
        GAME_X - 1,
        GAME_Y + GAME_SIZE,
        GAME_SIZE + 2,
        1,
        WHITE,
    ));
    commands.push(DrawCommand::rect(
        GAME_X - 1,
        GAME_Y - 1,
        1,
        GAME_SIZE + 2,
        WHITE,
    ));
    commands.push(DrawCommand::rect(
        GAME_X + GAME_SIZE,
        GAME_Y - 1,
        1,
        GAME_SIZE + 2,
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
    // Spawn at (3, 2) facing Right with body[0] at (3, 1). Length 2.
    let head: u8 = 3 * 8 + 2;
    let body0: u8 = 3 * 8 + 1;
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
        // Touch the cum table so the first update tick doesn't pay the init cost.
        let _ = cum_table();
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
                state.dead = true;
                state.cells.truncate(1);
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
            state.dead = true;
            state.cells.truncate(1);
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

    #[test]
    fn round_trip_length_2() {
        // Several length-2 snakes (head + one body cell).
        for &(head, body0) in &[(0u8, 1u8), (27, 26), (35, 27), (63, 62), (8, 0)] {
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
        // A few hand-rolled medium snakes that are valid SAWs.
        let cases: &[&[u8]] = &[
            &[27, 26, 25, 24], // length 4, head moving right (body to left)
            &[3 * 8 + 3, 3 * 8 + 2, 3 * 8 + 1, 4 * 8 + 1, 4 * 8 + 2],
            &[0, 1, 2, 10, 18, 17, 16, 24, 25],
        ];
        for cells in cases {
            let s = snake(cells.to_vec());
            let enc = encode(&s);
            let d = decode(enc);
            assert!(!d.dead);
            assert_eq!(&d.cells[..], *cells, "round-trip failed for {cells:?}");
        }
    }

    #[test]
    fn dead_round_trip() {
        let s = State {
            apple_bits: 3,
            cells: vec![42],
            dead: true,
        };
        let enc = encode(&s);
        let d = decode(enc);
        assert!(d.dead);
        assert_eq!(d.cells, vec![42]);
        assert_eq!(d.apple_bits, 3);
    }

    #[test]
    #[ignore]
    fn timing_open_snake() {
        // Worst case: short snake in open space. First decode step asks for
        // count_extensions at full remaining length from the head — that's
        // the most expensive single query.
        use std::time::Instant;
        let _ = cum_table();

        // A short spiral starting from the center, leaving most of the board open.
        let cells: Vec<u8> = vec![27, 26, 25, 17, 18, 19];
        let s = State {
            apple_bits: 0,
            cells: cells.clone(),
            dead: false,
        };

        let t = Instant::now();
        let enc = encode(&s);
        let et = t.elapsed();
        let t = Instant::now();
        let d = decode(enc);
        let dt = t.elapsed();
        assert_eq!(d.cells, cells);
        println!(
            "open snake body={} encode={:?} decode={:?}",
            cells.len() - 1,
            et,
            dt
        );
    }

    #[test]
    #[ignore]
    fn timing_max_length() {
        // Run with: cargo test --release --example snake2 -- --ignored timing_max_length --nocapture
        use std::time::Instant;

        // Warm up the cum table.
        let _ = cum_table();

        // Build a max-length zigzag snake from (0,0), boustrophedon order.
        let mut cells: Vec<u8> = Vec::new();
        'fill: for row in 0..8u8 {
            if row % 2 == 0 {
                for col in 0..8u8 {
                    cells.push(row * 8 + col);
                    if cells.len() == MAX_LEN + 1 {
                        break 'fill;
                    }
                }
            } else {
                for col in (0..8u8).rev() {
                    cells.push(row * 8 + col);
                    if cells.len() == MAX_LEN + 1 {
                        break 'fill;
                    }
                }
            }
        }
        let s = State {
            apple_bits: 0,
            cells: cells.clone(),
            dead: false,
        };

        let t0 = Instant::now();
        let enc = encode(&s);
        let enc_time = t0.elapsed();

        let t1 = Instant::now();
        let d = decode(enc);
        let dec_time = t1.elapsed();

        assert_eq!(d.cells, cells);
        println!(
            "MAX_LEN={MAX_LEN} body cells={} encode={:?} decode={:?}",
            cells.len() - 1,
            enc_time,
            dec_time
        );
    }

    #[test]
    fn enumerate_then_round_trip() {
        // For one starting head, walk every SAW of length 3 via the cum table
        // and confirm encode → decode → cells round-trips.
        let head: u8 = 3 * 8 + 3;
        let cum = &cum_table()[head as usize];
        let c3 = cum[3] - cum[2];
        for rank in 0..c3 {
            let body_rank = cum[2] + rank;
            let raw = (5u64) | ((head as u64) << APPLE_BITS) | (body_rank << RANK_SHIFT);
            let d = decode(raw);
            assert!(!d.dead);
            assert_eq!(d.cells.len(), 4); // head + 3 body cells
            let re = encode(&d);
            assert_eq!(re, raw, "encode(decode(x)) != x for rank {rank}");
        }
    }
}
