/*

Minesweeper on a 7×7 grid, packed into a u64.

Bit layout (LSB first):
- 10 bits: board seed → 1024 boards
- 54 bits: 34 tri-state cells in base-3, fixed length 34
           (3^34 ≈ 1.67e16 < 2^54 ≈ 1.80e16)

# Why 34 cells

Every cell on the board is one of three things: a mine, a numbered cell
(at least one adjacent mine), or a zero cell (no adjacent mines). Only
mines and numbered cells carry per-cell state worth storing — zero
cells reveal as a cascade and never get flagged in normal play.

We pick a board (mines + numbered = 34 interactive cells) and lex-order
them by row-major position. The state's i-th base-3 digit is the
tri-state of the i-th interactive cell:

  0 = HIDDEN, 1 = REVEALED, 2 = FLAGGED

# Zero cells don't get their own bits

A zero cell is displayed as revealed iff every numbered cell on its
connected zero-region's border is revealed in state. That captures
classic flood-fill behaviour:

  - Clicking a numbered cell N marks just N as revealed.
  - Clicking a zero cell in region Z marks ALL of Z's bordering
    numbered cells as revealed in one shot (the flood).
  - Render walks each region and asks "is every border cell revealed?";
    if so, fill the region's zeros with the revealed background colour.

The only quirk is that revealing every border cell of a region one-by-
one without ever clicking the zero will "auto-reveal" the region on
the last click. Rare and arguably correct — they've earned it.

# Board generation

Every tick, `update` must reconstruct the board from the 10-bit seed.
The seed alone doesn't pin down a board: we pick 7 mines at random
across the 49 cells, build adjacencies, and reject any placement that
doesn't yield exactly 34 interactive cells. The first seed-derived
attempt that satisfies the constraint is the canonical board for that
seed. At 14.3% raw mine density the expected interactive count lands
close to 34 with modest variance — the rejection generator converges
in a handful of attempts per tick.

# Input

- Mouse position drives a hover highlight only — it never enters state.
  Mouse coords come in as framebuffer pixels (0..128).
- Z reveals the hovered cell.
- X toggles a flag on the hovered cell (hidden ↔ flagged).
- After win/lose, Z or X restarts with a fresh seed derived from rng::next.

*/
use bitwise_games::draw_command::{
    BLACK, BLUE, Color, DARK_GREY, DrawCommand, GREEN, LAVENDER, LIGHT_GREY, ORANGE, PINK, RED,
    WHITE, YELLOW,
};
use bitwise_games::font::{draw_text, glyph};
use bitwise_games::frame_buffer::{self, FrameBuffer};
use bitwise_games::rng;
use bitwise_games::{Game, Key};

const ROWS: usize = 7;
const COLS: usize = 7;
const N_CELLS: usize = ROWS * COLS; // 49
const N_MINES: usize = 7;
const N_INTERACTIVE: usize = 34;

const SEED_BITS: u32 = 10;
const SEED_MASK: u64 = (1u64 << SEED_BITS) - 1;

// Tri-state values (base-3 digit per cell).
const HIDDEN: u8 = 0;
const REVEALED: u8 = 1;
const FLAGGED: u8 = 2;

// Layout: 7×7 grid of 13-px cells. Each cell paints a 1-px BLACK top + left
// border and a 12×12 interior at (cell + 1, cell + 1). A 1-px closing
// BLACK border on the grid's right + bottom edges seals the box. Total
// grid: 1 + 7×13 = 92 px. Margin in 128: 18 on every side (symmetric).
//
// Even cell interior (12) pairs with the even scale-2 digit (6×10): padding
// 3 wide, 1 tall — true centring on a single pair of pixels.
const CELL_PX: u32 = 13;
const INNER_PX: u32 = CELL_PX - 1; // 12
const GRID_PX: u32 = COLS as u32 * CELL_PX + 1; // 92
const GRID_X: u32 = (frame_buffer::WIDTH - GRID_PX) / 2; // 18
const GRID_Y: u32 = GRID_X;
const STATUS_Y: u32 = 4;
const DIGIT_SCALE: u32 = 2;
const COUNTER_SCALE: u32 = 2;

// Search bound for valid mine placements. Hit rate ~10%; 1000 attempts is
// vastly more than needed in practice.
const MAX_GEN_ATTEMPTS: u32 = 1000;

// --- State encoding ---

struct State {
    seed: u16,
    cells: [u8; N_INTERACTIVE],
}

fn decode(state: u64) -> State {
    let seed = (state & SEED_MASK) as u16;
    let mut v = state >> SEED_BITS;
    let mut cells = [0u8; N_INTERACTIVE];
    for cell in cells.iter_mut() {
        *cell = (v % 3) as u8;
        v /= 3;
    }
    State { seed, cells }
}

fn encode(state: &State) -> u64 {
    let mut packed = 0u64;
    for i in (0..N_INTERACTIVE).rev() {
        packed = packed * 3 + state.cells[i] as u64;
    }
    (packed << SEED_BITS) | (state.seed as u64 & SEED_MASK)
}

// --- Board (regenerated per tick from seed) ---

struct ZeroRegion {
    /// Indices into `interactive` for the numbered cells on this region's
    /// border. When all are REVEALED in state, the region is shown as
    /// revealed at render time.
    border_idxs: Vec<u8>,
}

struct Board {
    is_mine: [bool; N_CELLS],
    counts: [u8; N_CELLS],
    /// Cell position → digit index in the encoded `cells` array, or 255 for
    /// zero (non-interactive) cells. Implicitly defines lex order: we iterate
    /// cells 0..N_CELLS and assign rising indices to interactive ones.
    cell_to_idx: [u8; N_CELLS],
    zero_regions: Vec<ZeroRegion>,
    /// Per cell, the index into `zero_regions`, or 255 if not a zero cell.
    cell_to_region: [u8; N_CELLS],
}

fn cell_xy(cell: u8) -> (u32, u32) {
    let row = (cell as u32) / COLS as u32;
    let col = (cell as u32) % COLS as u32;
    (GRID_X + col * CELL_PX, GRID_Y + row * CELL_PX)
}

fn neighbors_8(cell: u8) -> impl Iterator<Item = u8> {
    let r = (cell as i32) / COLS as i32;
    let c = (cell as i32) % COLS as i32;
    (-1i32..=1).flat_map(move |dr| {
        (-1i32..=1).filter_map(move |dc| {
            if dr == 0 && dc == 0 {
                return None;
            }
            let nr = r + dr;
            let nc = c + dc;
            if nr < 0 || nr >= ROWS as i32 || nc < 0 || nc >= COLS as i32 {
                return None;
            }
            Some((nr * COLS as i32 + nc) as u8)
        })
    })
}

fn pick_mines(seed: u64) -> [u8; N_MINES] {
    let mut chosen = [255u8; N_MINES];
    let mut n = 0usize;
    let mut rng_state = seed | 1;
    while n < N_MINES {
        rng_state = rng::next(rng_state);
        let cell = (rng_state % N_CELLS as u64) as u8;
        if !chosen[..n].contains(&cell) {
            chosen[n] = cell;
            n += 1;
        }
    }
    chosen
}

fn try_build_board(mines: &[u8; N_MINES]) -> Option<Board> {
    let mut is_mine = [false; N_CELLS];
    for &m in mines {
        is_mine[m as usize] = true;
    }

    let mut counts = [0u8; N_CELLS];
    for cell in 0..N_CELLS as u8 {
        if is_mine[cell as usize] {
            continue;
        }
        for n in neighbors_8(cell) {
            if is_mine[n as usize] {
                counts[cell as usize] += 1;
            }
        }
    }

    // Interactive = mines + numbered, in row-major order.
    let mut cell_to_idx = [255u8; N_CELLS];
    let mut n_interactive = 0usize;
    for cell in 0..N_CELLS as u8 {
        if is_mine[cell as usize] || counts[cell as usize] > 0 {
            if n_interactive >= N_INTERACTIVE {
                return None;
            }
            cell_to_idx[cell as usize] = n_interactive as u8;
            n_interactive += 1;
        }
    }
    if n_interactive != N_INTERACTIVE {
        return None;
    }

    // Connected components of zero cells (8-connected). For each region,
    // collect the unique bordering numbered cells (as interactive indices).
    let mut cell_to_region = [255u8; N_CELLS];
    let mut zero_regions: Vec<ZeroRegion> = Vec::new();
    let mut stack: Vec<u8> = Vec::new();
    let mut border_seen = [false; N_CELLS];

    for start in 0..N_CELLS as u8 {
        if is_mine[start as usize] || counts[start as usize] != 0 {
            continue;
        }
        if cell_to_region[start as usize] != 255 {
            continue;
        }
        let region_idx = zero_regions.len() as u8;
        let mut border_idxs: Vec<u8> = Vec::new();

        // Clear the border-seen scratchpad lazily by tracking which entries
        // we set during this region's flood, then resetting only those.
        let mut touched_border: Vec<u8> = Vec::new();

        stack.push(start);
        while let Some(c) = stack.pop() {
            if cell_to_region[c as usize] != 255 {
                continue;
            }
            cell_to_region[c as usize] = region_idx;
            for n in neighbors_8(c) {
                if is_mine[n as usize] {
                    continue;
                }
                if counts[n as usize] == 0 {
                    if cell_to_region[n as usize] == 255 {
                        stack.push(n);
                    }
                } else if !border_seen[n as usize] {
                    border_seen[n as usize] = true;
                    touched_border.push(n);
                    border_idxs.push(cell_to_idx[n as usize]);
                }
            }
        }

        for n in touched_border {
            border_seen[n as usize] = false;
        }
        zero_regions.push(ZeroRegion { border_idxs });
    }

    Some(Board {
        is_mine,
        counts,
        cell_to_idx,
        zero_regions,
        cell_to_region,
    })
}

fn generate_board(seed: u16) -> Board {
    for k in 0..MAX_GEN_ATTEMPTS {
        // Mix the attempt counter into the seed so each k probes a different
        // placement deterministically.
        let h = rng::next((seed as u64) ^ ((k as u64).wrapping_mul(0x9E37_79B1)));
        let mines = pick_mines(h);
        if let Some(board) = try_build_board(&mines) {
            return board;
        }
    }
    // Pathological seed: fall back to a board with whatever the last attempt
    // produced, even if it's not exactly 34 interactive. We pad/truncate in
    // `try_build_board_lossy`. Should be vanishingly rare with 1000 attempts.
    build_board_lossy(seed)
}

fn build_board_lossy(seed: u16) -> Board {
    let mines = pick_mines(rng::next(seed as u64));
    let mut is_mine = [false; N_CELLS];
    for &m in &mines {
        is_mine[m as usize] = true;
    }
    let mut counts = [0u8; N_CELLS];
    for cell in 0..N_CELLS as u8 {
        if is_mine[cell as usize] {
            continue;
        }
        for n in neighbors_8(cell) {
            if is_mine[n as usize] {
                counts[cell as usize] += 1;
            }
        }
    }
    let mut cell_to_idx = [255u8; N_CELLS];
    let mut n_interactive = 0usize;
    for cell in 0..N_CELLS as u8 {
        if (is_mine[cell as usize] || counts[cell as usize] > 0) && n_interactive < N_INTERACTIVE {
            cell_to_idx[cell as usize] = n_interactive as u8;
            n_interactive += 1;
        }
    }
    Board {
        is_mine,
        counts,
        cell_to_idx,
        zero_regions: Vec::new(),
        cell_to_region: [255u8; N_CELLS],
    }
}

// --- Derived state ---

fn region_revealed(state: &State, board: &Board, region_idx: usize) -> bool {
    board.zero_regions[region_idx]
        .border_idxs
        .iter()
        .all(|&idx| state.cells[idx as usize] == REVEALED)
}

fn is_dead(state: &State, board: &Board) -> bool {
    (0..N_CELLS as u8).any(|cell| {
        if !board.is_mine[cell as usize] {
            return false;
        }
        let idx = board.cell_to_idx[cell as usize];
        idx != 255 && state.cells[idx as usize] == REVEALED
    })
}

fn is_won(state: &State, board: &Board) -> bool {
    // Win = every mine is flagged AND no non-mine cell is flagged.
    // Revealing the rest is encouraged but not required: the assertion is
    // about correctly identifying mines, not clearing the board.
    let mut mines_flagged = 0;
    for cell in 0..N_CELLS as u8 {
        let idx = board.cell_to_idx[cell as usize];
        if idx == 255 {
            continue;
        }
        let flagged = state.cells[idx as usize] == FLAGGED;
        if flagged && !board.is_mine[cell as usize] {
            return false;
        }
        if board.is_mine[cell as usize] && flagged {
            mines_flagged += 1;
        }
    }
    mines_flagged == N_MINES
}

// --- Input ---

fn hovered_cell(mouse: Option<(u8, u8)>) -> Option<u8> {
    let (x, y) = mouse?;
    let x = x as u32;
    let y = y as u32;
    if !(GRID_X..GRID_X + GRID_PX).contains(&x) {
        return None;
    }
    if !(GRID_Y..GRID_Y + GRID_PX).contains(&y) {
        return None;
    }
    let col = (x - GRID_X) / CELL_PX;
    let row = (y - GRID_Y) / CELL_PX;
    Some((row * COLS as u32 + col) as u8)
}

fn apply_reveal(state: &mut State, board: &Board, cell: u8) {
    if board.is_mine[cell as usize] {
        let idx = board.cell_to_idx[cell as usize];
        if idx != 255 {
            state.cells[idx as usize] = REVEALED;
        }
        return;
    }
    if board.counts[cell as usize] > 0 {
        let idx = board.cell_to_idx[cell as usize];
        if idx != 255 && state.cells[idx as usize] == HIDDEN {
            state.cells[idx as usize] = REVEALED;
        }
        return;
    }
    // Zero cell: flood — mark every numbered cell on the region's border
    // as revealed (skip flagged ones; the player asserted those are mines).
    let region = board.cell_to_region[cell as usize];
    if region == 255 {
        return;
    }
    for &border_idx in &board.zero_regions[region as usize].border_idxs {
        if state.cells[border_idx as usize] == HIDDEN {
            state.cells[border_idx as usize] = REVEALED;
        }
    }
}

fn apply_flag(state: &mut State, board: &Board, cell: u8) {
    let idx = board.cell_to_idx[cell as usize];
    if idx == 255 {
        return;
    }
    let v = &mut state.cells[idx as usize];
    *v = match *v {
        HIDDEN => FLAGGED,
        FLAGGED => HIDDEN,
        _ => *v, // revealed: no-op
    };
}

// --- Rendering ---

fn number_color(n: u8) -> Color {
    // Picked for readability against the DARK_GREY revealed background.
    match n {
        1 => BLUE,
        2 => GREEN,
        3 => RED,
        4 => LAVENDER,
        5 => ORANGE,
        6 => PINK,
        7 => WHITE,
        _ => LIGHT_GREY,
    }
}

fn draw_digit(
    commands: &mut Vec<DrawCommand>,
    digit: u8,
    x: u32,
    y: u32,
    scale: u32,
    color: Color,
) {
    let pattern = glyph(b'0' + digit);
    for (row, bits) in pattern.iter().enumerate() {
        for col in 0..3u32 {
            if (bits >> (2 - col)) & 1 == 1 {
                commands.push(DrawCommand::rect(
                    x + col * scale,
                    y + row as u32 * scale,
                    scale,
                    scale,
                    color,
                ));
            }
        }
    }
}

fn draw_cell_background(commands: &mut Vec<DrawCommand>, cell: u8, revealed: bool) {
    // The grid area is pre-filled BLACK; each cell only paints its 12×12
    // interior, leaving the surrounding BLACK pixels as the cell's top +
    // left border (and the grid's closing right + bottom borders for the
    // last row/column).
    let (x, y) = cell_xy(cell);
    let ix = x + 1;
    let iy = y + 1;
    if revealed {
        commands.push(DrawCommand::rect(ix, iy, INNER_PX, INNER_PX, DARK_GREY));
    } else {
        // Raised LIGHT_GREY tile with a 1-px WHITE top/left highlight and a
        // 1-px DARK_GREY bottom/right shadow — classic minesweeper bevel.
        commands.push(DrawCommand::rect(ix, iy, INNER_PX, INNER_PX, LIGHT_GREY));
        commands.push(DrawCommand::rect(ix, iy, INNER_PX, 1, WHITE));
        commands.push(DrawCommand::rect(ix, iy, 1, INNER_PX, WHITE));
        commands.push(DrawCommand::rect(
            ix,
            iy + INNER_PX - 1,
            INNER_PX,
            1,
            DARK_GREY,
        ));
        commands.push(DrawCommand::rect(
            ix + INNER_PX - 1,
            iy,
            1,
            INNER_PX,
            DARK_GREY,
        ));
    }
}

fn draw_flag(commands: &mut Vec<DrawCommand>, cell: u8) {
    let (x, y) = cell_xy(cell);
    // Pole at col 6, pennant right of pole, base bar at the bottom.
    // Shifted 1 px up and 1 px left from the previous placement for visual
    // balance against the bevel.
    commands.push(DrawCommand::rect(x + 6, y + 2, 1, 8, BLACK));
    commands.push(DrawCommand::rect(x + 7, y + 2, 4, 1, RED));
    commands.push(DrawCommand::rect(x + 7, y + 3, 3, 1, RED));
    commands.push(DrawCommand::rect(x + 7, y + 4, 2, 1, RED));
    commands.push(DrawCommand::rect(x + 7, y + 5, 1, 1, RED));
    commands.push(DrawCommand::rect(x + 3, y + 10, 7, 1, BLACK));
}

fn draw_mine(commands: &mut Vec<DrawCommand>, cell: u8, exploded: bool) {
    let (x, y) = cell_xy(cell);
    if exploded {
        commands.push(DrawCommand::rect(x + 1, y + 1, INNER_PX, INNER_PX, RED));
    }
    // Cross of spikes centred at (col 8, row 6) — shifted 1 px right and
    // 1 px up from the geometric cell centre for visual balance.
    commands.push(DrawCommand::rect(x + 8, y + 2, 1, 9, BLACK));
    commands.push(DrawCommand::rect(x + 4, y + 6, 9, 1, BLACK));
    // 5×5 body centred on the cross.
    commands.push(DrawCommand::rect(x + 6, y + 4, 5, 5, BLACK));
    // Single-pixel highlight glint, biased toward the upper-left.
    commands.push(DrawCommand::rect(x + 7, y + 5, 1, 1, WHITE));
}

fn draw_number(commands: &mut Vec<DrawCommand>, cell: u8, count: u8) {
    let (x, y) = cell_xy(cell);
    // 6×10 scale-2 digit centred in the 12×12 interior: padding 3 wide, 1 tall.
    draw_digit(
        commands,
        count,
        x + 1 + (INNER_PX - 3 * DIGIT_SCALE) / 2,
        y + 1 + (INNER_PX - 5 * DIGIT_SCALE) / 2,
        DIGIT_SCALE,
        number_color(count),
    );
}

fn draw_hover(commands: &mut Vec<DrawCommand>, cell: u8) {
    let (x, y) = cell_xy(cell);
    // YELLOW ring around the cell's full 12×12 interior — sits 1 px past the
    // BLACK top+left border and flush with the right+bottom interior edges
    // (which are flanked by the neighbouring cell's BLACK border).
    let left = x + 1;
    let top = y + 1;
    let right = x + CELL_PX - 1;
    let bottom = y + CELL_PX - 1;
    let width = right - left + 1;
    let height = bottom - top + 1;
    commands.push(DrawCommand::rect(left, top, width, 1, YELLOW));
    commands.push(DrawCommand::rect(left, bottom, width, 1, YELLOW));
    commands.push(DrawCommand::rect(left, top, 1, height, YELLOW));
    commands.push(DrawCommand::rect(right, top, 1, height, YELLOW));
}

fn draw_banner(commands: &mut Vec<DrawCommand>, text: &[u8], color: Color) {
    let scale = 2u32;
    let glyph_w = 3 * scale;
    let gap = scale;
    let total_w = text.len() as u32 * glyph_w + (text.len() as u32 - 1) * gap;
    let bw = total_w + 8;
    let bh = 5 * scale + 8;
    let bx = GRID_X + (GRID_PX - bw) / 2;
    let by = GRID_Y + (GRID_PX - bh) / 2;
    commands.push(DrawCommand::rect(bx, by, bw, bh, BLACK));
    commands.push(DrawCommand::rect(bx, by, bw, 1, color));
    commands.push(DrawCommand::rect(bx, by + bh - 1, bw, 1, color));
    commands.push(DrawCommand::rect(bx, by, 1, bh, color));
    commands.push(DrawCommand::rect(bx + bw - 1, by, 1, bh, color));
    draw_text(commands, text, bx + 4, by + 4, scale, color);
}

fn render(state: &State, board: &Board, hover: Option<u8>) -> FrameBuffer {
    let mut fb = FrameBuffer::new();
    let mut commands = Vec::new();

    commands.push(DrawCommand::rect(
        0,
        0,
        frame_buffer::WIDTH,
        frame_buffer::HEIGHT,
        BLACK,
    ));

    let dead = is_dead(state, board);
    let won = !dead && is_won(state, board);

    // The grid area starts as a uniform BLACK block; every cell paints just
    // its 12×12 interior on top, leaving 1-px BLACK pixels between cells as
    // shared borders and a closing border around the grid's outer edge.
    commands.push(DrawCommand::rect(GRID_X, GRID_Y, GRID_PX, GRID_PX, BLACK));

    // First pass: backgrounds (reveal status determines visual).
    for cell in 0..N_CELLS as u8 {
        let idx = board.cell_to_idx[cell as usize];
        let revealed = if idx != 255 {
            // Mine or numbered: revealed status lives directly in the digit.
            state.cells[idx as usize] == REVEALED
        } else {
            // Zero cell: revealed iff its region's border is fully revealed.
            let region = board.cell_to_region[cell as usize];
            region != 255 && region_revealed(state, board, region as usize)
        };
        draw_cell_background(&mut commands, cell, revealed);
    }

    // Hover ring is drawn *before* glyphs so the flag pole, digit, or mine
    // body lands on top of it — the yellow shows in the gaps the glyph
    // leaves rather than overpainting the glyph.
    if !dead && !won {
        if let Some(cell) = hover {
            draw_hover(&mut commands, cell);
        }
    }

    // Second pass: glyphs (numbers, mines, flags). Drawn after all backgrounds
    // so adjacent cell frames don't paint over them.
    for cell in 0..N_CELLS as u8 {
        let idx = board.cell_to_idx[cell as usize];
        let cell_state = if idx == 255 {
            HIDDEN
        } else {
            state.cells[idx as usize]
        };

        if board.is_mine[cell as usize] {
            if dead {
                // All mines revealed on death; the clicked one shows red.
                let exploded = cell_state == REVEALED;
                draw_mine(&mut commands, cell, exploded);
            } else if cell_state == FLAGGED {
                draw_flag(&mut commands, cell);
            }
            // Hidden mines stay hidden during play and on win.
        } else if board.counts[cell as usize] > 0 {
            if cell_state == REVEALED {
                draw_number(&mut commands, cell, board.counts[cell as usize]);
            } else if cell_state == FLAGGED {
                draw_flag(&mut commands, cell);
            }
        }
        // Zero cells: no glyph regardless of state.
    }

    // Wrong-flag indicator on death: flags on non-mine cells get a red X.
    if dead {
        for cell in 0..N_CELLS as u8 {
            if board.is_mine[cell as usize] {
                continue;
            }
            let idx = board.cell_to_idx[cell as usize];
            if idx != 255 && state.cells[idx as usize] == FLAGGED {
                let (x, y) = cell_xy(cell);
                commands.push(DrawCommand::rect(x + 2, y + 2, CELL_PX - 4, 1, RED));
                commands.push(DrawCommand::rect(
                    x + 2,
                    y + CELL_PX - 3,
                    CELL_PX - 4,
                    1,
                    RED,
                ));
            }
        }
    }

    // Status strip: mines remaining = N_MINES − flags placed on mines we
    // *believe* are mines (every flagged interactive cell). Classic
    // minesweeper counter behaviour.
    let flags = state.cells.iter().filter(|&&v| v == FLAGGED).count() as i32;
    let remaining = N_MINES as i32 - flags;
    draw_counter(&mut commands, remaining);

    if dead {
        draw_banner(&mut commands, b"GAME OVER", RED);
    } else if won {
        draw_banner(&mut commands, b"YOU WIN", GREEN);
    }

    fb.draw_list(&commands);
    fb
}

fn draw_counter(commands: &mut Vec<DrawCommand>, remaining: i32) {
    // Mines-remaining counter at the top status bar, scale 2 (6×10 per digit).
    // Negative values get a 1-unit-tall horizontal bar before the digits.
    let scale = COUNTER_SCALE;
    let y = STATUS_Y;
    let mut x = GRID_X;
    if remaining < 0 {
        commands.push(DrawCommand::rect(
            x,
            y + 2 * scale,
            2 * scale,
            scale,
            ORANGE,
        ));
        x += 3 * scale;
    }
    let n = remaining.unsigned_abs();
    let tens = (n / 10) as u8;
    let ones = (n % 10) as u8;
    let glyph_w = 3 * scale;
    let gap = scale;
    if tens > 0 {
        draw_digit(commands, tens, x, y, scale, ORANGE);
        x += glyph_w + gap;
    }
    draw_digit(commands, ones, x, y, scale, ORANGE);
}

// --- Game impl ---

fn fresh_state(seed: u16) -> State {
    State {
        seed: seed & SEED_MASK as u16,
        cells: [HIDDEN; N_INTERACTIVE],
    }
}

struct Minesweeper;

impl Game for Minesweeper {
    const NAME: &'static str = "Minesweeper";
    const FPS: usize = 30;

    fn new(args: Vec<String>) -> (u64, FrameBuffer) {
        let raw = args.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let seed = (raw & SEED_MASK) as u16;
        let state = fresh_state(seed);
        let board = generate_board(state.seed);
        (encode(&state), render(&state, &board, None))
    }

    fn update(
        state: u64,
        _held: &[Key],
        buffered: Option<Key>,
        mouse: Option<(u8, u8)>,
    ) -> (u64, FrameBuffer) {
        let mut s = decode(state);
        let board = generate_board(s.seed);
        let dead = is_dead(&s, &board);
        let won = !dead && is_won(&s, &board);
        let hover = hovered_cell(mouse);

        if dead || won {
            if matches!(buffered, Some(Key::Z) | Some(Key::X)) {
                let next_seed = (rng::next(state) & SEED_MASK) as u16;
                let ns = fresh_state(next_seed);
                let nb = generate_board(ns.seed);
                return (encode(&ns), render(&ns, &nb, hover));
            }
            return (encode(&s), render(&s, &board, None));
        }

        if let Some(cell) = hover {
            match buffered {
                Some(Key::Z) => apply_reveal(&mut s, &board, cell),
                Some(Key::X) => apply_flag(&mut s, &board, cell),
                _ => {}
            }
        }

        let fb = render(&s, &board, hover);
        (encode(&s), fb)
    }
}

fn main() {
    bitwise_games::run_game::<Minesweeper>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips() {
        for seed in [0u16, 1, 42, 511, 1023] {
            let mut s = fresh_state(seed);
            // Fill every cell with a non-trivial mix of states.
            for (i, c) in s.cells.iter_mut().enumerate() {
                *c = (i % 3) as u8;
            }
            let bits = encode(&s);
            let d = decode(bits);
            assert_eq!(d.seed, s.seed);
            assert_eq!(d.cells, s.cells);
        }
    }

    #[test]
    fn every_seed_finds_a_valid_board() {
        // Hit-rate sanity: for every 10-bit seed, generation should converge
        // within MAX_GEN_ATTEMPTS and yield exactly 34 interactive cells.
        for seed in 0..1024u16 {
            let board = generate_board(seed);
            let interactive = board.cell_to_idx.iter().filter(|&&i| i != 255).count();
            assert_eq!(
                interactive, N_INTERACTIVE,
                "seed {seed} produced {interactive} interactive cells"
            );
        }
    }
}
