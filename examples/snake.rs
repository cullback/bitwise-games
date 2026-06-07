/*

Snake on an 8×8 grid.

# Inputs

- Arrow keys: turn the snake
- Z or X: restart after game-over or win

# Maximize

Max snake length on an 8×8 board, bounded by the 64-bit budget. Head
position (6) + head direction (2) + apple entropy (3) = 11 bits spoken
for; the remaining 53 bits hold the body tail. The body is the
expensive field — each cell beyond head + an implied first body cell
is a 3-state turn (Left / Straight / Right relative to the walking
direction), encoded as a variable-length base-3 integer via `varlen`.
Trinary varlen of length 0..33 fits in 53 bits ((3^34 − 1) / 2 ≈
8.34e15 < 2^53 ≈ 9.0e15), so the cap is head + body[0] + 33 turns =
length 35.

# Encoding

| Start | Length | Description                                            |
|-------|--------|--------------------------------------------------------|
|     0 |      6 | head cell (row*8 + col on the 8×8 grid)                |
|     6 |      2 | head direction (0=Up, 1=Right, 2=Down, 3=Left)         |
|     8 |      3 | apple entropy (chosen at spawn to dodge the body)      |
|    11 |     53 | body tail — varlen base-3 of L/S/R turns, or DEAD      |

# Notes

**Body as L/S/R turns.** From any body cell, the *next* cell going
toward the tail has only 3 valid positions (Left, Straight, Right
relative to the walking direction) — backwards would fold the snake
into itself. Trinary digits at ≈1.585 bits/cell beat absolute direction
(2 bits/cell, with one state per step wasted).

**body[0] is implied by (head, head_direction).** The first body cell
is always opposite the head's direction, so we don't encode it as a
turn. The turns we *do* encode are L/S/R choices at body[1], …,
body[L-1]. This shaves one trinary digit (≈1.585 bits) compared to
encoding a phantom choice at body[0] that physically has only 1 option.
The direction stays at 2 bits because it's what makes body[0] derivable
— drop it and the head has 4 interpretations.

**Minimum length 2.** Spawn with zero turns: head + the implied body[0].
`body_int = 0` is a valid live state. At length 2 the rounded head and
the tapered tail draw right next to each other — looks like a complete
little snake without needing a body segment.

**Dead sentinel.** Live varlen range [0, (3^34 − 1)/2) ≈ 8.34e15 fills
53 bits with ~660e12 unreachable values to spare. We pick
`body_int = (3^34 − 1)/2` (just past the largest valid encoding) as the
"dead" sentinel. On death the live body shape is lost (rendered as a
collapsed length-2), but the encoding stays clean and max length is
unaffected.

**Apple position.** The apple cell needs to (1) stay put between eats
so it doesn't flicker, and (2) regenerate on each eat. Both require it
to be a pure function of data that's constant between eats — namely
`snake_length` and the 3 stored `apple_bits`:

    apple_cell = rng::next((snake_length << 3) | apple_bits) % 64

3 bits over 2 is a free upgrade — neither costs a snake cell since 53
and 54 body bits both hold L_max = 33. The extra bit drops the
"apple spawns on body" probability from ~9% to ~0.9% at max length. On
eat we scan 8 candidate `apple_bits` values and take the first whose
cell isn't a snake cell; if all 8 collide (~0.9% at max), we accept the
collision and the apple sits inside the body until the tail clears.

**Render.** 8×8 grid at 12 px/cell, centred in the lower portion of
the frame with a 24-px score strip above. Head has direction-indicating
eyes; tail is tapered; turn cells round the outer corner of the bend.

*/
use bitwise_games::draw_command::{
    BLACK, Color, DARK_BLUE, DrawCommand, GREEN, LIGHT_GREY, RED, WHITE,
};
use bitwise_games::font::{digits_of, draw_text, text_width};
use bitwise_games::frame_buffer::FrameBuffer;
use bitwise_games::rng;
use bitwise_games::{Game, Key};
use rancor::varlen;

const BOARD_CELLS: u32 = 8;
const DISPLAY_PX: u32 = 128;
const CELL_PX: u32 = 12;
const GAME_X: u32 = 16;
const GAME_Y: u32 = 24;
const GAME_SIZE: u32 = BOARD_CELLS * CELL_PX;

const FONT_SCALE: u32 = 3;
const BANNER_SCALE: u32 = 2;

// Directions
const DIR_UP: u8 = 0;
const DIR_RIGHT: u8 = 1;
const DIR_DOWN: u8 = 2;
const DIR_LEFT: u8 = 3;

// Turns (relative rotation of the "walking direction" as we trace
// the snake from head toward tail).
const T_CCW: u8 = 0;
const T_STRAIGHT: u8 = 1;
const T_CW: u8 = 2;

const MAX_TURNS: usize = 33;

// Game-over sentinel: a body_int value that's just past the largest valid
// varlen encoding for our length cap, so it can't collide with a real snake.
// On death the live body shape is discarded; rendering falls back to a 2-cell
// "collapsed" snake at the last head position.
//   (3^34 − 1) / 2 = 8338590849833284
const DEAD: u64 = (3u64.pow(34) - 1) / 2;

// --- State encoding ---

struct State {
    head: u8,
    head_dir: u8,
    apple_bits: u8,
    /// Body tail as the varlen base-3 turn sequence. `body == DEAD` is the
    /// game-over sentinel; otherwise `varlen::unrank(body, 3)` reconstructs turns.
    body: u64,
    /// Decoded body. Empty when `body == DEAD`. Cached so the varlen decode
    /// runs once per tick instead of twice (update + render both need it).
    /// Mutators must keep this consistent with `body`; use `set_turns`.
    turns: Vec<u8>,
    /// Snake length in cells (head + body). Equals `turns.len() + 2` for
    /// live states; equals 2 in the dead/collapsed render state (since
    /// `turns` is empty then). Pure cache — `encode` ignores it.
    length: usize,
}

impl State {
    /// Replace the turn sequence and refresh the derived `body` + `length`.
    fn set_turns(&mut self, turns: Vec<u8>) {
        self.body = varlen::rank(&turns, 3);
        self.length = turns.len() + 2;
        self.turns = turns;
    }

    /// Transition to the game-over sentinel. The live body shape is discarded;
    /// rendering falls back to a 2-cell collapsed snake at the current head.
    fn die(&mut self) {
        self.body = DEAD;
        self.turns.clear();
        self.length = 2;
    }
}

fn decode(state: u64) -> State {
    let body = state >> 11;
    // DEAD isn't a valid varlen — leave `turns` empty; length stays at the
    // 2-cell collapsed-render value via `turns.len() + 2`.
    let turns = if body == DEAD {
        Vec::new()
    } else {
        varlen::unrank(body, 3)
    };
    let length = turns.len() + 2;
    State {
        head: (state & 0x3F) as u8,
        head_dir: ((state >> 6) & 0x3) as u8,
        apple_bits: ((state >> 8) & 0x7) as u8,
        body,
        turns,
        length,
    }
}

fn encode(state: &State) -> u64 {
    ((state.head as u64) & 0x3F)
        | (((state.head_dir as u64) & 0x3) << 6)
        | (((state.apple_bits as u64) & 0x7) << 8)
        | (state.body << 11)
}

// --- Direction utilities ---

fn opposite(dir: u8) -> u8 {
    (dir + 2) & 3
}

fn apply_turn(dir: u8, turn: u8) -> u8 {
    match turn {
        T_CCW => (dir + 3) & 3,
        T_STRAIGHT => dir,
        T_CW => (dir + 1) & 3,
        _ => dir,
    }
}

fn turn_between(from_dir: u8, to_dir: u8) -> u8 {
    let diff = (to_dir + 4 - from_dir) & 3;
    match diff {
        0 => T_STRAIGHT,
        1 => T_CW,
        3 => T_CCW,
        _ => T_STRAIGHT, // 180° flip — shouldn't happen
    }
}

fn step(pos: u8, dir: u8) -> Option<u8> {
    let row = (pos / BOARD_CELLS as u8) as i32;
    let col = (pos % BOARD_CELLS as u8) as i32;
    let (dr, dc) = match dir {
        DIR_UP => (-1, 0),
        DIR_RIGHT => (0, 1),
        DIR_DOWN => (1, 0),
        DIR_LEFT => (0, -1),
        _ => return None,
    };
    let nr = row + dr;
    let nc = col + dc;
    if nr < 0 || nr >= BOARD_CELLS as i32 || nc < 0 || nc >= BOARD_CELLS as i32 {
        None
    } else {
        Some((nr * BOARD_CELLS as i32 + nc) as u8)
    }
}

// --- Snake decoding ---

// Cells in head→tail order.
fn snake_cells(head: u8, head_dir: u8, turns: &[u8]) -> Vec<u8> {
    let mut cells = Vec::with_capacity(turns.len() + 2);
    cells.push(head);
    let mut walking = opposite(head_dir);
    let Some(first_body) = step(head, walking) else {
        return cells;
    };
    cells.push(first_body);
    let mut current = first_body;
    for &turn in turns {
        walking = apply_turn(walking, turn);
        match step(current, walking) {
            Some(next) => {
                cells.push(next);
                current = next;
            }
            None => break,
        }
    }
    cells
}

// Walking direction at each step (length = cells.len() − 1).
fn walking_dirs(head_dir: u8, turns: &[u8]) -> Vec<u8> {
    let mut dirs = Vec::with_capacity(turns.len() + 1);
    let mut walking = opposite(head_dir);
    dirs.push(walking);
    for &turn in turns {
        walking = apply_turn(walking, turn);
        dirs.push(walking);
    }
    dirs
}

// --- Apple ---

fn apple_cell(snake_length: usize, apple_bits: u8) -> u8 {
    let seed = ((snake_length as u64) << 3) | (apple_bits as u64);
    (rng::next(seed) % 64) as u8
}

fn pick_apple_bits(seed: u64, snake_length: usize, snake_cells: &[u8]) -> u8 {
    let mask = snake_cells.iter().fold(0u64, |acc, &c| acc | (1u64 << c));
    let start = (rng::next(seed) & 0x7) as u8;
    for offset in 0..8u8 {
        let bits = (start + offset) & 0x7;
        let cell = apple_cell(snake_length, bits);
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

/// At a turn cell, the outer corner of the bend (the sharp 90° apex on the
/// convex side). Given the walking direction arriving at the cell and the
/// walking direction leaving it, return which of its 4 corners that is.
fn outer_corner(arrival: u8, departure: u8) -> Corner {
    match (arrival, departure) {
        (DIR_DOWN, DIR_RIGHT) | (DIR_LEFT, DIR_UP) => Corner::BL,
        (DIR_DOWN, DIR_LEFT) | (DIR_RIGHT, DIR_UP) => Corner::BR,
        (DIR_UP, DIR_RIGHT) | (DIR_LEFT, DIR_DOWN) => Corner::TL,
        (DIR_UP, DIR_LEFT) | (DIR_RIGHT, DIR_DOWN) => Corner::TR,
        _ => unreachable!("outer_corner called on a straight (non-turn) cell"),
    }
}

fn draw_body_cell(commands: &mut Vec<DrawCommand>, cell: u8, rounded: Option<Corner>) {
    let (x, y) = cell_xy(cell);
    // Fill the whole cell so adjacent body segments connect visually.
    commands.push(DrawCommand::rect(x, y, CELL_PX, CELL_PX, GREEN));

    // At a turn, snip 3 pixels from the outer corner so the bend's convex
    // perimeter has a 2-pixel diagonal instead of a sharp 90° step — matches
    // the head's corner rounding. Paint with the playfield colour so the snip
    // blends with the background.
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

// Symmetric triangular notch profile for the tail: 0 at edges, 5 at center.
const TAIL_NOTCH: [u32; 12] = [0, 1, 2, 3, 4, 5, 5, 4, 3, 2, 1, 0];

fn draw_tail_cell(commands: &mut Vec<DrawCommand>, cell: u8, body_dir: u8) {
    // body_dir points from tail toward the body. The tail's "tip" is the
    // opposite side — that's where the triangular notch is carved out.
    let (x, y) = cell_xy(cell);
    for i in 0..CELL_PX {
        let depth = TAIL_NOTCH[i as usize];
        let span = CELL_PX - depth;
        match body_dir {
            DIR_RIGHT => {
                // Body to the right → tip on left → carve left side
                commands.push(DrawCommand::rect(x + depth, y + i, span, 1, GREEN));
            }
            DIR_LEFT => {
                // Tip on right
                commands.push(DrawCommand::rect(x, y + i, span, 1, GREEN));
            }
            DIR_DOWN => {
                // Tip on top — iterate columns instead
                commands.push(DrawCommand::rect(x + i, y + depth, 1, span, GREEN));
            }
            DIR_UP => {
                // Tip on bottom
                commands.push(DrawCommand::rect(x + i, y, 1, span, GREEN));
            }
            _ => {}
        }
    }
}

fn draw_head_cell(commands: &mut Vec<DrawCommand>, cell: u8, dir: u8, dead: bool) {
    let (x, y) = cell_xy(cell);
    // Rounded square: middle 8 rows are full width; rows 1/10 lose 1 corner
    // pixel each side; rows 0/11 lose 2 corner pixels each side.
    commands.push(DrawCommand::rect(x, y + 2, CELL_PX, CELL_PX - 4, GREEN));
    commands.push(DrawCommand::rect(x + 1, y + 1, CELL_PX - 2, 1, GREEN));
    commands.push(DrawCommand::rect(x + 1, y + 10, CELL_PX - 2, 1, GREEN));
    commands.push(DrawCommand::rect(x + 2, y, CELL_PX - 4, 1, GREEN));
    commands.push(DrawCommand::rect(x + 2, y + 11, CELL_PX - 4, 1, GREEN));

    // Eyes (or X eyes on death) positioned according to facing direction.
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
    // 3x3 X pattern.
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

fn fresh_board(seed: u64) -> State {
    // Spawn at (row 3, col 2) moving Right with body extending left to (3, 1).
    // Length 2 (head + body[0]). Head has 5 cells of room to the right before
    // the wall — ~1s at 5 FPS.
    let head: u8 = 3 * 8 + 2;
    let head_dir = DIR_RIGHT;
    let turns: Vec<u8> = vec![];
    let body = varlen::rank(&turns, 3); // = 0
    let cells = snake_cells(head, head_dir, &turns);
    let apple_bits = pick_apple_bits(seed, cells.len(), &cells);
    let length = turns.len() + 2;
    State {
        head,
        head_dir,
        apple_bits,
        body,
        turns,
        length,
    }
}

fn render(state: &State) -> FrameBuffer {
    let mut fb = FrameBuffer::new();
    let mut commands = Vec::new();

    commands.push(DrawCommand::rect(0, 0, DISPLAY_PX, DISPLAY_PX, BLACK));

    let dead = state.body == DEAD;
    let cells = snake_cells(state.head, state.head_dir, &state.turns);
    // dirs[i] is the walking direction at cells[i]: i=0 is head→body[0], and
    // dirs[i] for i≥1 is the direction body[i-1]→body[i]. Used to detect
    // turn cells and orient the tail.
    let dirs = walking_dirs(state.head_dir, &state.turns);

    if !dead {
        // Score = apples eaten. Snake spawns at length 2 (head + body[0]);
        // each apple adds one cell, so apples = length − 2.
        draw_number(&mut commands, (state.length - 2) as u32, 4, 4);
    }

    // Game-area background + border
    commands.push(DrawCommand::rect(
        GAME_X, GAME_Y, GAME_SIZE, GAME_SIZE, DARK_BLUE,
    ));
    // 1-pixel border outline
    commands.push(DrawCommand::rect(
        GAME_X - 1,
        GAME_Y - 1,
        GAME_SIZE + 2,
        1,
        LIGHT_GREY,
    ));
    commands.push(DrawCommand::rect(
        GAME_X - 1,
        GAME_Y + GAME_SIZE,
        GAME_SIZE + 2,
        1,
        LIGHT_GREY,
    ));
    commands.push(DrawCommand::rect(
        GAME_X - 1,
        GAME_Y,
        1,
        GAME_SIZE,
        LIGHT_GREY,
    ));
    commands.push(DrawCommand::rect(
        GAME_X + GAME_SIZE,
        GAME_Y,
        1,
        GAME_SIZE,
        LIGHT_GREY,
    ));

    // Body (between head and tail). At turn cells, round the outer corner of
    // the bend so the silhouette reads as a smooth curve, not a 90° step.
    if state.length >= 3 {
        for i in 1..cells.len() - 1 {
            let arrival = dirs[i - 1];
            let departure = dirs[i];
            let rounded = (arrival != departure).then(|| outer_corner(arrival, departure));
            draw_body_cell(&mut commands, cells[i], rounded);
        }
    }

    // Tail
    if state.length >= 2 {
        let last_walk = dirs[dirs.len() - 1];
        let body_dir = opposite(last_walk); // from tail toward the body
        draw_tail_cell(&mut commands, *cells.last().unwrap(), body_dir);
    }

    // Head on top of the body.
    draw_head_cell(&mut commands, state.head, state.head_dir, dead);

    // Apple drawn last so it stays visible even when it spawns under the
    // snake's body (≈0.9% at max length — see `pick_apple_bits`). Hidden on
    // death along with the score.
    if !dead {
        let apple = apple_cell(state.length, state.apple_bits);
        draw_apple(&mut commands, apple);
    }

    // Terminal-state banners
    let won = !dead && state.turns.len() >= MAX_TURNS;
    if dead {
        draw_banner(&mut commands, b"GAME OVER", b"PRESS Z", RED);
    } else if won {
        draw_banner(&mut commands, b"YOU WIN", b"PRESS Z", GREEN);
    }

    fb.draw_list(&commands);
    fb
}

// --- Game impl ---

struct SnakeGame;

impl Game for SnakeGame {
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

        // Dead state: Z or X restarts; anything else holds the frozen view.
        if state.body == DEAD {
            if matches!(buffered, Some(Key::Z) | Some(Key::X)) {
                let new_state = fresh_board(rng::next(encode(&state)));
                return (encode(&new_state), render(&new_state));
            }
            return (encode(&state), render(&state));
        }

        if state.turns.len() > MAX_TURNS {
            // Defensive: shouldn't happen, but if state is corrupt, freeze it.
            return (encode(&state), render(&state));
        }

        // Won state (snake reached max length): freeze, restart on Z/X.
        if state.turns.len() == MAX_TURNS {
            if matches!(buffered, Some(Key::Z) | Some(Key::X)) {
                let new_state = fresh_board(rng::next(encode(&state)));
                return (encode(&new_state), render(&new_state));
            }
            return (encode(&state), render(&state));
        }

        // Direction from buffered arrow; can't reverse 180°.
        let candidate = match buffered {
            Some(Key::Up) => Some(DIR_UP),
            Some(Key::Right) => Some(DIR_RIGHT),
            Some(Key::Down) => Some(DIR_DOWN),
            Some(Key::Left) => Some(DIR_LEFT),
            _ => None,
        };
        let new_dir = match candidate {
            Some(d) if d != opposite(state.head_dir) => d,
            _ => state.head_dir,
        };

        // New head position. Off the board → game over.
        let new_head = match step(state.head, new_dir) {
            Some(p) => p,
            None => {
                state.die();
                return (encode(&state), render(&state));
            }
        };

        let current_cells = snake_cells(state.head, state.head_dir, &state.turns);
        let current_apple = apple_cell(state.length, state.apple_bits);
        let ate = new_head == current_apple;

        // Self-collision: check against body (and tail, unless it's about to vacate).
        let body_check_len = if ate { state.length } else { state.length - 1 };
        if current_cells[..body_check_len].contains(&new_head) {
            state.die();
            return (encode(&state), render(&state));
        }

        let can_grow = ate && state.turns.len() < MAX_TURNS;

        // New turn sequence:
        //   - Growing: prepend new_first_turn, keep all old turns (length+1).
        //   - Non-grow, length ≥ 3: shift — prepend new_first_turn, drop last
        //     (tail vacates).
        //   - Non-grow, length 2 (turns empty): no turn to add; the new
        //     body[0] is determined by the new head direction alone.
        let new_first_turn = turn_between(opposite(new_dir), opposite(state.head_dir));
        let mut new_turns: Vec<u8> = Vec::with_capacity(state.turns.len() + 1);
        if can_grow {
            new_turns.push(new_first_turn);
            new_turns.extend_from_slice(&state.turns);
        } else if !state.turns.is_empty() {
            new_turns.push(new_first_turn);
            new_turns.extend_from_slice(&state.turns[..state.turns.len() - 1]);
        }

        let new_apple_bits = if ate {
            let new_cells = snake_cells(new_head, new_dir, &new_turns);
            let new_body = varlen::rank(&new_turns, 3);
            let seed = new_body ^ (new_head as u64) ^ ((new_dir as u64) << 6);
            pick_apple_bits(seed, new_cells.len(), &new_cells)
        } else {
            state.apple_bits
        };

        state.head = new_head;
        state.head_dir = new_dir;
        state.apple_bits = new_apple_bits;
        state.set_turns(new_turns);
        (encode(&state), render(&state))
    }
}

fn main() {
    bitwise_games::run_game::<SnakeGame>();
}
