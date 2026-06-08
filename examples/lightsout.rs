/*

Lights Out on a 5×5 grid.

# Inputs

- Mouse: hover highlight (no state)
- Z: toggle hovered cell + its 4 orthogonal neighbours; counts as a move
- X: regenerate a fresh puzzle (also serves as the post-win restart)

# Maximize

Board variety + move-count headroom within 64 bits. The 5×5 grid only
needs 25 bits, but every state is reachable from solved on a 5×5 (no
parity constraint), so all 2^25 grids are valid puzzles. The remaining
budget holds a move counter so the player can chase a low solve count.

# Encoding

| Start | Length | Description                                            |
|-------|--------|--------------------------------------------------------|
|     0 |     25 | grid state, 1 bit per cell (row-major; 1 = lit)        |
|    25 |      8 | move counter (saturates at 255)                        |
|    33 |     31 | unused                                                 |

# Notes

**Solvability.** Every 2^25 grid on a 5×5 Lights Out board is reachable
from all-off, so we can generate puzzles by sampling a handful of random
clicks from the solved state. (Larger boards aren't this nice — a 6×6
has a non-trivial kernel and only ~1 in 16 random grids is solvable.)

**Puzzle generation.** `new` takes a seed from args and applies a fixed
number of random clicks to all-off via `rng::next`. Click order doesn't
matter — XOR is commutative — so the puzzle's difficulty floor equals
the number of distinct cells in the click set; duplicate clicks cancel.

**Win condition.** Win = all 25 grid bits are 0. Banner appears with
the final move count; Z or X starts a fresh puzzle with a seed derived
from `rng::next(state)`.

**Move counter saturates at 255.** 8 bits is plenty — optimal solves on
a 5×5 are ≤15 moves and even a tangled human attempt rarely exceeds 50.

**Solution recipe (3 steps, works on every solvable 5×5).**

1. *Chase down*: for each row top-to-bottom, click every cell directly
   below a lit cell in the row above. Row 5 ends up with all the
   remaining lights.

2. *Top-row fix*: look at row 5, cols 1–3 (ignore 4–5). For each lit
   cell, click these row-1 cells:
     - col 1 lit → click row 1, cols 1, 2
     - col 2 lit → click row 1, cols 1, 2, 3
     - col 3 lit → click row 1, cols 2, 3

3. *Chase down again*. Board clears.

*/
use bitwise_games::draw_command::{
    BLACK, Color, DARK_BLUE, DARK_GREY, DrawCommand, GREEN, LIGHT_GREY, WHITE, YELLOW,
};
use bitwise_games::font::{GLYPH_H, digits_of, draw_text, text_width};
use bitwise_games::frame_buffer::{self, FrameBuffer};
use bitwise_games::rng;
use bitwise_games::{Game, Key};

const ROWS: usize = 5;
const COLS: usize = 5;
const N_CELLS: usize = ROWS * COLS; // 25

// Bit layout.
const GRID_BITS: u32 = N_CELLS as u32; // 25
const COUNTER_BITS: u32 = 8;
const COUNTER_SHIFT: u32 = GRID_BITS;
const COUNTER_MAX: u32 = (1u32 << COUNTER_BITS) - 1;
const GRID_MASK: u64 = (1u64 << GRID_BITS) - 1;
const COUNTER_MASK: u64 = ((1u64 << COUNTER_BITS) - 1) << COUNTER_SHIFT;

// Layout: 14-px header (move counter), then a 112×112 board centred at
// (8, 14) with a 2-px DARK_GREY pad framing the outer cells, 2-px gaps
// between them, and 20×20 interiors.
const CELL_PX: u32 = 20;
const GAP: u32 = 2;
const PAD: u32 = GAP;
const BOARD_W: u32 = 2 * PAD + COLS as u32 * CELL_PX + (COLS as u32 - 1) * GAP;
const BOARD_OFFSET_X: u32 = (frame_buffer::WIDTH - BOARD_W) / 2;
const HEADER_H: u32 = 14;
const BOARD_OFFSET_Y: u32 = HEADER_H;

const COUNTER_SCALE: u32 = 2;
const BANNER_SCALE: u32 = 2;

// Initial puzzle difficulty: how many random clicks from all-off.
const INITIAL_CLICKS: usize = 10;

// --- State helpers ---

fn grid_of(state: u64) -> u64 {
    state & GRID_MASK
}

fn counter_of(state: u64) -> u32 {
    ((state & COUNTER_MASK) >> COUNTER_SHIFT) as u32
}

fn pack(grid: u64, counter: u32) -> u64 {
    let c = (counter.min(COUNTER_MAX) as u64) << COUNTER_SHIFT;
    (grid & GRID_MASK) | c
}

fn cell_index(r: usize, c: usize) -> u32 {
    (r * COLS + c) as u32
}

fn cell_lit(grid: u64, r: usize, c: usize) -> bool {
    (grid >> cell_index(r, c)) & 1 == 1
}

fn toggle(grid: u64, r: i32, c: i32) -> u64 {
    if r < 0 || c < 0 || r >= ROWS as i32 || c >= COLS as i32 {
        return grid;
    }
    grid ^ (1u64 << cell_index(r as usize, c as usize))
}

/// Apply a click: toggle the cell and its 4 orthogonal neighbours.
fn apply_click(grid: u64, r: i32, c: i32) -> u64 {
    let mut g = toggle(grid, r, c);
    g = toggle(g, r - 1, c);
    g = toggle(g, r + 1, c);
    g = toggle(g, r, c - 1);
    g = toggle(g, r, c + 1);
    g
}

fn is_won(grid: u64) -> bool {
    grid == 0
}

fn fresh_grid(seed: u64) -> u64 {
    let mut grid = 0u64;
    let mut rng_state = seed | 1; // splitmix collapses 0 → 0
    for _ in 0..INITIAL_CLICKS {
        rng_state = rng::next(rng_state);
        let cell = (rng_state % N_CELLS as u64) as i32;
        let r = cell / COLS as i32;
        let c = cell % COLS as i32;
        grid = apply_click(grid, r, c);
    }
    grid
}

// --- Input ---

fn hovered_cell(mouse: Option<(u8, u8)>) -> Option<(usize, usize)> {
    let (mx, my) = mouse?;
    let mx = mx as u32;
    let my = my as u32;
    let x0 = BOARD_OFFSET_X + PAD;
    let y0 = BOARD_OFFSET_Y + PAD;
    if mx < x0 || my < y0 {
        return None;
    }
    let dx = mx - x0;
    let dy = my - y0;
    let col = dx / (CELL_PX + GAP);
    let row = dy / (CELL_PX + GAP);
    if col >= COLS as u32 || row >= ROWS as u32 {
        return None;
    }
    // Reject the gap between cells.
    let cell_x = col * (CELL_PX + GAP);
    let cell_y = row * (CELL_PX + GAP);
    if dx - cell_x >= CELL_PX || dy - cell_y >= CELL_PX {
        return None;
    }
    Some((row as usize, col as usize))
}

// --- Rendering ---

fn cell_xy(r: usize, c: usize) -> (u32, u32) {
    (
        BOARD_OFFSET_X + PAD + c as u32 * (CELL_PX + GAP),
        BOARD_OFFSET_Y + PAD + r as u32 * (CELL_PX + GAP),
    )
}

fn cell_color(lit: bool) -> Color {
    if lit { YELLOW } else { DARK_BLUE }
}

fn draw_cell(commands: &mut Vec<DrawCommand>, r: usize, c: usize, lit: bool) {
    let (x, y) = cell_xy(r, c);
    commands.push(DrawCommand::rect(x, y, CELL_PX, CELL_PX, cell_color(lit)));
}

fn draw_hover(commands: &mut Vec<DrawCommand>, r: usize, c: usize) {
    let (x, y) = cell_xy(r, c);
    // 1-px WHITE frame around the cell — visible on both lit and unlit tiles.
    commands.push(DrawCommand::rect(x, y, CELL_PX, 1, WHITE));
    commands.push(DrawCommand::rect(x, y + CELL_PX - 1, CELL_PX, 1, WHITE));
    commands.push(DrawCommand::rect(x, y, 1, CELL_PX, WHITE));
    commands.push(DrawCommand::rect(x + CELL_PX - 1, y, 1, CELL_PX, WHITE));
}

fn draw_counter(commands: &mut Vec<DrawCommand>, count: u32) {
    let digits = digits_of(count);
    let w = text_width(digits.len(), COUNTER_SCALE);
    let h = GLYPH_H * COUNTER_SCALE;
    let x = (frame_buffer::WIDTH - w) / 2;
    let y = (HEADER_H - h) / 2;
    draw_text(commands, &digits, x, y, COUNTER_SCALE, DARK_GREY);
}

fn draw_banner(commands: &mut Vec<DrawCommand>, lines: &[&[u8]], color: Color) {
    let glyph_w = 3 * BANNER_SCALE;
    let gap = BANNER_SCALE;
    let widths: Vec<u32> = lines
        .iter()
        .map(|l| l.len() as u32 * glyph_w + (l.len() as u32 - 1) * gap)
        .collect();
    let max_w = *widths.iter().max().unwrap_or(&0);
    let bw = max_w + 8;
    let line_h = GLYPH_H * BANNER_SCALE;
    let bh = lines.len() as u32 * line_h + (lines.len() as u32 - 1) * BANNER_SCALE + 8;
    let bx = BOARD_OFFSET_X + (BOARD_W - bw) / 2;
    let by = BOARD_OFFSET_Y + (BOARD_W - bh) / 2;

    commands.push(DrawCommand::rect(bx, by, bw, bh, BLACK));
    commands.push(DrawCommand::rect(bx, by, bw, 1, color));
    commands.push(DrawCommand::rect(bx, by + bh - 1, bw, 1, color));
    commands.push(DrawCommand::rect(bx, by, 1, bh, color));
    commands.push(DrawCommand::rect(bx + bw - 1, by, 1, bh, color));

    for (i, line) in lines.iter().enumerate() {
        let line_w = widths[i];
        let lx = bx + (bw - line_w) / 2;
        let ly = by + 4 + i as u32 * (line_h + BANNER_SCALE);
        let col = if i == 0 { color } else { WHITE };
        draw_text(commands, line, lx, ly, BANNER_SCALE, col);
    }
}

fn render(state: u64, hover: Option<(usize, usize)>) -> FrameBuffer {
    let mut fb = FrameBuffer::new();
    let mut commands = Vec::new();

    let grid = grid_of(state);
    let count = counter_of(state);
    let won = is_won(grid);

    // Light cream chrome behind everything.
    commands.push(DrawCommand::rect(
        0,
        0,
        frame_buffer::WIDTH,
        frame_buffer::HEIGHT,
        LIGHT_GREY,
    ));

    // Board base: DARK_GREY block — the PAD around cells and the GAP between
    // them show through as a continuous grid border.
    commands.push(DrawCommand::rect(
        BOARD_OFFSET_X,
        BOARD_OFFSET_Y,
        BOARD_W,
        BOARD_W,
        DARK_GREY,
    ));

    for r in 0..ROWS {
        for c in 0..COLS {
            draw_cell(&mut commands, r, c, cell_lit(grid, r, c));
        }
    }

    draw_counter(&mut commands, count);

    if !won {
        if let Some((r, c)) = hover {
            draw_hover(&mut commands, r, c);
        }
    } else {
        draw_banner(&mut commands, &[b"YOU WIN", b"PRESS X"], GREEN);
    }

    fb.draw_list(&commands);
    fb
}

// --- Game impl ---

struct LightsOut;

impl Game for LightsOut {
    const NAME: &'static str = "Lights Out";
    const FPS: usize = 30;

    fn init(args: Vec<String>) -> (u64, FrameBuffer) {
        let seed = args.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let grid = fresh_grid(seed);
        let state = pack(grid, 0);
        (state, render(state, None))
    }

    fn update(
        state: u64,
        _held: &[Key],
        buffered: Option<Key>,
        mouse: Option<(u8, u8)>,
    ) -> (u64, FrameBuffer) {
        let grid = grid_of(state);
        let count = counter_of(state);
        let hover = hovered_cell(mouse);

        if is_won(grid) {
            // Frozen win state: any of Z/X regenerates from rng::next(state).
            if matches!(buffered, Some(Key::Z) | Some(Key::X)) {
                let next_seed = rng::next(state);
                let new_grid = fresh_grid(next_seed);
                let new_state = pack(new_grid, 0);
                return (new_state, render(new_state, hover));
            }
            return (state, render(state, hover));
        }

        // X restarts the puzzle outright (also doubles as "give up").
        if buffered == Some(Key::X) {
            let next_seed = rng::next(state);
            let new_grid = fresh_grid(next_seed);
            let new_state = pack(new_grid, 0);
            return (new_state, render(new_state, hover));
        }

        // Z toggles the hovered cell + its 4 neighbours.
        if buffered == Some(Key::Z) {
            if let Some((r, c)) = hover {
                let new_grid = apply_click(grid, r as i32, c as i32);
                let new_count = (count + 1).min(COUNTER_MAX);
                let new_state = pack(new_grid, new_count);
                return (new_state, render(new_state, hover));
            }
        }

        (state, render(state, hover))
    }
}

fn main() {
    bitwise_games::run_game::<LightsOut>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_round_trips() {
        for &(g, c) in &[
            (0u64, 0u32),
            (0x1, 1),
            (0x1FFFFFF, COUNTER_MAX),
            (0xAAAAAA, 42),
        ] {
            let s = pack(g, c);
            assert_eq!(grid_of(s), g);
            assert_eq!(counter_of(s), c);
        }
    }

    #[test]
    fn click_toggles_cross() {
        // From all-off, clicking centre cell (2, 2) lights the centre + 4
        // orthogonal neighbours = exactly 5 cells.
        let g = apply_click(0, 2, 2);
        assert_eq!(g.count_ones(), 5);
        // Centre + (1,2), (3,2), (2,1), (2,3) — all bits in {12, 7, 17, 11, 13}.
        let expected = (1u64 << 12) | (1u64 << 7) | (1u64 << 17) | (1u64 << 11) | (1u64 << 13);
        assert_eq!(g, expected);
    }

    #[test]
    fn click_self_inverse() {
        let g0 = 0xDEADBEu64 & GRID_MASK;
        let g1 = apply_click(g0, 1, 3);
        let g2 = apply_click(g1, 1, 3);
        assert_eq!(g0, g2);
    }

    #[test]
    fn corner_click_toggles_three() {
        // Top-left corner: itself + (0,1) + (1,0). 3 cells.
        let g = apply_click(0, 0, 0);
        assert_eq!(g.count_ones(), 3);
    }
}
