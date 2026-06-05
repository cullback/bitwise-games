/*

Snake on an 8×8 grid, packed into a u64.

Design goal: maximize both board size and max snake length within the
64-bit state budget. Every encoding choice below is in service of one
or the other — bigger board, longer reachable snake, or freeing bits
that can go to either.

Bit layout (fields packed consecutively, LSB first):
-  6 bits: head position (row*8 + col on the 8×8 grid)
-  2 bits: head direction (0=Up, 1=Right, 2=Down, 3=Left)
-  3 bits: apple entropy (chosen at spawn to dodge the body)
- 53 bits: body tail — varlen base-3 of "turn" digits.
           A special sentinel value in this field means game over.

# Encoding rationale

## Body as variable-length trinary turns

Each cell of the snake's body is adjacent to its neighbours; from any
body cell, the *next* body cell going toward the tail has only 3 valid
positions (Left, Straight, Right relative to the direction the snake
was moving) — backwards would fold the snake into itself, so it's
forbidden. That makes each step a 3-state choice, which is denser than
storing absolute direction (4 states, ~1.585 vs 2 bits per cell).

The body length is variable. We encode it using the var-length base-3
trick from `varlen` — the integer's value implicitly carries both the
length and the L/S/R choices, with no separate length field or sentinel.

## body[0] is implied

The first body cell (the cell immediately behind the head) is always
opposite the head's direction — there's no choice there. So we don't
encode it as a "turn"; it falls out of (head, head_direction). The
turns we *do* encode are the L/S/R choices at body[1], body[2], ...,
body[L-1], i.e. one turn per body cell beyond the implied first one.

This shaves ~log₂(3) ≈ 1.585 bits off the naïve "encode a turn at every
body cell" scheme — phantom turns at body[0] would have been encoding 3
choices on a value that physically has only 1.

## Why direction is still 2 bits

You might think "if body[0] is implied by head + direction, the
direction must somehow be free." It isn't — the direction is what makes
body[0] derivable in the first place. Without it the head has 4
possible interpretations and we lose the savings.

## Minimum snake length = 2

Spawn with zero turns: just head + the implied body[0]. The snake
grows as it eats. body_int = 0 (empty varlen) is a valid live state.

A nice rendering side-effect: at length 2 there are no "body" cells
between the head and the tail, so the rounded head and the tapered
tail end up drawn right next to each other. That makes the initial
spawn look like a complete little snake (head adjoining tail) instead
of needing a body segment to bridge them.

## Body bit budget

We have 53 bits for the body tail. The varlen count for trinary
sequences of length 0..33 fits in 53 bits:

    (3^34 − 1) / 2  =  8.34 × 10^15  ≤  2^53 = 9.0 × 10^15  ✓
    (3^35 − 1) / 2  =  2.5  × 10^16  >  2^53                ✗

So we can have at most 33 turns, which means 34 body cells (body[0]
implied + 33 from turns) and a total snake length of 35.

## Game-over sentinel

The live varlen range is `[0, (3^34 − 1)/2)` ≈ 8.34e15 values, which
fits in 53 bits with ~660e12 unreachable values to spare. We pick one
of those unreachable values as the "dead" sentinel — specifically
`body_int = (3^34 − 1) / 2`, the value just past the largest valid
encoding. On death we store this value; the live body shape is lost
(the snake collapses to length 2 visually), but the encoding stays
clean and max snake length is unaffected.

## Apple: 3 bits of entropy, derived position

The apple's cell needs to:
  - stay put between eats (so it doesn't flicker as the snake moves)
  - regenerate to a fresh cell on each eat

Both invariants require apple-cell to be a pure function of data that's
constant between eats. The only such data we have is `snake_length` and
the 3 stored `apple_bits`. So:

    apple_cell  =  rng::next((snake_length << 3) | apple_bits)  %  64

3 bits chosen over 2 because it's a free upgrade — both 53 and 54 body
bits hold the same L_max = 33, so taking one bit from body and giving
it to apple doesn't cost a snake cell. The extra bit drops the
"apple-spawns-on-body" probability from ~9% to ~0.9% at max length.

## Apple spawn at eat-time

When the snake eats, we pick `apple_bits` from 8 candidates: starting
from a state-derived offset, increment until we find one whose
computed cell isn't a snake cell. If all 8 collide (≈0.9% at max
snake), we accept the collision — the apple sits "inside" the snake's
body, and the player has to wait for the tail to move off it. That's
gameplay-equivalent to a slightly delayed apple, not a hard failure.

## Render

128×128 pixel display. 8×8 grid at 12px per cell, centered horizontally
in the lower portion of the frame. Top 24px is a score area (snake
length, drawn with a 3×5 digit font scaled 3×). Thin border around the
playfield. Head has direction-indicating eyes; tail is drawn smaller
than body cells to taper visually.

*/
use bitwise_games::draw_command::{
    BLACK, Color, DARK_GREY, DARK_PURPLE, DrawCommand, GREEN, RED, WHITE,
};
use bitwise_games::font::{digits_of, draw_text, text_width};
use bitwise_games::frame_buffer::FrameBuffer;
use bitwise_games::rng;
use bitwise_games::varlen::{from_varlen, to_varlen};
use bitwise_games::{Game, Key};

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

fn decode(state: u64) -> (u8, u8, u8, u64) {
    let head = (state & 0x3F) as u8;
    let dir = ((state >> 6) & 0x3) as u8;
    let apple = ((state >> 8) & 0x7) as u8;
    let body = state >> 11;
    (head, dir, apple, body)
}

fn encode(head: u8, dir: u8, apple: u8, body: u64) -> u64 {
    ((head as u64) & 0x3F)
        | (((dir as u64) & 0x3) << 6)
        | (((apple as u64) & 0x7) << 8)
        | (body << 11)
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

fn draw_body_cell(commands: &mut Vec<DrawCommand>, cell: u8) {
    let (x, y) = cell_xy(cell);
    // Fill the whole cell so adjacent body segments connect visually.
    commands.push(DrawCommand::rect(x, y, CELL_PX, CELL_PX, GREEN));
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

fn fresh_state(seed: u64) -> u64 {
    // Spawn at (row 3, col 2) moving Right with body extending left to (3, 1).
    // Length 2 (head + body[0]). Head has 5 cells of room to the right before
    // the wall — ~1s at 5 FPS.
    let head: u8 = 3 * 8 + 2;
    let head_dir = DIR_RIGHT;
    let turns: Vec<u8> = vec![];
    let body_int = from_varlen(&turns, 3); // = 0
    let cells = snake_cells(head, head_dir, &turns);
    let apple_bits = pick_apple_bits(seed, cells.len(), &cells);
    encode(head, head_dir, apple_bits, body_int)
}

fn render(state: u64) -> FrameBuffer {
    let mut fb = FrameBuffer::new();
    let mut commands = Vec::new();

    commands.push(DrawCommand::rect(0, 0, DISPLAY_PX, DISPLAY_PX, BLACK));

    let (head, head_dir, apple_bits, body_int) = decode(state);
    let dead = body_int == DEAD;
    // On death the body shape is lost (the DEAD value isn't a valid varlen
    // encoding), so we render a 2-cell collapsed snake at the last head
    // position. Apple and score are hidden in the death view.
    let turns = if dead {
        Vec::new()
    } else {
        to_varlen(body_int, 3)
    };
    let cells = snake_cells(head, head_dir, &turns);

    if !dead {
        // Score (snake length) — hidden on death
        draw_number(&mut commands, cells.len() as u32, 4, 4);
    }

    // Game-area background + border
    commands.push(DrawCommand::rect(
        GAME_X,
        GAME_Y,
        GAME_SIZE,
        GAME_SIZE,
        DARK_PURPLE,
    ));
    // 1-pixel border outline
    commands.push(DrawCommand::rect(
        GAME_X - 1,
        GAME_Y - 1,
        GAME_SIZE + 2,
        1,
        DARK_GREY,
    ));
    commands.push(DrawCommand::rect(
        GAME_X - 1,
        GAME_Y + GAME_SIZE,
        GAME_SIZE + 2,
        1,
        DARK_GREY,
    ));
    commands.push(DrawCommand::rect(
        GAME_X - 1,
        GAME_Y,
        1,
        GAME_SIZE,
        DARK_GREY,
    ));
    commands.push(DrawCommand::rect(
        GAME_X + GAME_SIZE,
        GAME_Y,
        1,
        GAME_SIZE,
        DARK_GREY,
    ));

    // Apple — hidden on death
    if !dead {
        let apple = apple_cell(cells.len(), apple_bits);
        draw_apple(&mut commands, apple);
    }

    // Body (between head and tail)
    if cells.len() >= 3 {
        for &cell in &cells[1..cells.len() - 1] {
            draw_body_cell(&mut commands, cell);
        }
    }

    // Tail
    if cells.len() >= 2 {
        let dirs = walking_dirs(head_dir, &turns);
        let last_walk = dirs[dirs.len() - 1];
        let body_dir = opposite(last_walk); // from tail toward the body
        draw_tail_cell(&mut commands, *cells.last().unwrap(), body_dir);
    }

    // Head on top
    draw_head_cell(&mut commands, head, head_dir, dead);

    // Terminal-state banners
    let won = !dead && turns.len() >= MAX_TURNS;
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

    fn new(args: Vec<String>) -> (u64, FrameBuffer) {
        let seed = args.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let state = fresh_state(seed);
        (state, render(state))
    }

    fn update(state: u64, _held: &[Key], buffered: Option<Key>) -> (u64, FrameBuffer) {
        let (head, head_dir, apple_bits, body_int) = decode(state);

        // Dead state: Z or X restarts; anything else holds the frozen view.
        if body_int == DEAD {
            if matches!(buffered, Some(Key::Z) | Some(Key::X)) {
                let new_state = fresh_state(rng::next(state));
                return (new_state, render(new_state));
            }
            return (state, render(state));
        }

        let turns = to_varlen(body_int, 3);
        if turns.len() > MAX_TURNS {
            // Defensive: shouldn't happen, but if state is corrupt, freeze it.
            return (state, render(state));
        }

        // Won state (snake reached max length): freeze, restart on Z/X.
        if turns.len() == MAX_TURNS {
            if matches!(buffered, Some(Key::Z) | Some(Key::X)) {
                let new_state = fresh_state(rng::next(state));
                return (new_state, render(new_state));
            }
            return (state, render(state));
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
            Some(d) if d != opposite(head_dir) => d,
            _ => head_dir,
        };

        // New head position. Off the board → game over.
        let new_head = match step(head, new_dir) {
            Some(p) => p,
            None => {
                let dead = encode(head, head_dir, apple_bits, DEAD);
                return (dead, render(dead));
            }
        };

        let current_cells = snake_cells(head, head_dir, &turns);
        let current_apple = apple_cell(current_cells.len(), apple_bits);
        let ate = new_head == current_apple;

        // Self-collision: check against body (and tail, unless it's about to vacate).
        let body_check_len = if ate {
            current_cells.len()
        } else {
            current_cells.len() - 1
        };
        if current_cells[..body_check_len].contains(&new_head) {
            let dead = encode(head, head_dir, apple_bits, DEAD);
            return (dead, render(dead));
        }

        let can_grow = ate && turns.len() < MAX_TURNS;

        // New turn sequence:
        //   - Growing: prepend new_first_turn, keep all old turns (length+1).
        //   - Non-grow, length ≥ 3: shift — prepend new_first_turn, drop last
        //     (tail vacates).
        //   - Non-grow, length 2 (turns empty): no turn to add; the new
        //     body[0] is determined by the new head direction alone.
        let new_first_turn = turn_between(opposite(new_dir), opposite(head_dir));
        let mut new_turns: Vec<u8> = Vec::with_capacity(turns.len() + 1);
        if can_grow {
            new_turns.push(new_first_turn);
            new_turns.extend_from_slice(&turns);
        } else if !turns.is_empty() {
            new_turns.push(new_first_turn);
            new_turns.extend_from_slice(&turns[..turns.len() - 1]);
        }
        let new_body_int = from_varlen(&new_turns, 3);

        let new_apple_bits = if ate {
            let new_cells = snake_cells(new_head, new_dir, &new_turns);
            let seed = new_body_int ^ (new_head as u64) ^ ((new_dir as u64) << 6);
            pick_apple_bits(seed, new_cells.len(), &new_cells)
        } else {
            apple_bits
        };

        let new_state = encode(new_head, new_dir, new_apple_bits, new_body_int);
        (new_state, render(new_state))
    }
}

fn main() {
    bitwise_games::run_game::<SnakeGame>();
}
