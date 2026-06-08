/*

Minesweeper on a 7×7 grid (49 cells, 8 mines).

# Inputs

- Mouse: hover highlight (no state)
- Z: reveal hovered cell (restart after end-of-game)
- X: toggle flag on hovered cell (restart after end-of-game)

# Maximize

Mines and numbered cells carry different per-cell entropy. Mines only
toggle between HIDDEN and FLAGGED (1 bit); numbered cells need
HIDDEN/REVEALED/FLAGGED (log₂(3) ≈ 1.585 bits). Death is encoded as a
*sentinel* in the numbered-cell value: any packed value ≥ 3^N_NUMBERED
means "dead." That uses the slack between 3^N_NUMBERED and the next
power of two — no explicit dead bit, freeing one numbered cell over
the equivalent dead-bit layout.

Budget with 8 mines and 8 seed bits:

  64 − 8 (seed) − 8 (mine bits) = 48 bits for numbered + sentinel

3^30 + 1 ≈ 2.06e14 < 2^48 ≈ 2.81e14, so 30 numbered cells fit (vs. 29
with an explicit dead bit). Total: 8 mines + 30 numbered = 38
interactive cells. Mine density on 7×7: 16.3%, between beginner
(12–15%) and intermediate (~18%).

Death rendering: the sentinel collapses the numbered snapshot, but the
death screen reveals the entire board anyway — every mine, every
numbered count, every zero cell. Wrong flags simply appear as the
numbers they should have been. Nothing of value is lost from the
collapse.

`N_NUMBERED` is derived from `N_MINES` via `max_numbered`. Each extra
mine costs only 1 bit (vs. 1.585 for a numbered cell), so denser-mine
boards fit more interactive cells — the limit becomes board geometry,
not the encoding.

# Encoding

| Start | Length | Description                                  |
|-------|--------|----------------------------------------------|
|     0 |      8 | board seed (256 unique boards)               |
|     8 |      8 | mine flag bits (1 = FLAGGED, 0 = HIDDEN)     |
|    16 |    ~48 | numbered packed: < 3^N alive, ≥ 3^N is dead  |

Cell layout: `cells[0..N_MINES]` hold mines in row-major grid order
(values 0 = HIDDEN, 2 = FLAGGED — REVEALED is never set on a mine).
`cells[N_MINES..N_INTERACTIVE]` hold numbered cells in row-major grid
order (values 0/1/2 = HIDDEN/REVEALED/FLAGGED).

# Notes

**Which cells are interactive.** Every cell on the board is a mine, a
numbered cell (at least one adjacent mine), or a zero cell (no adjacent
mines). Only mines and numbered cells carry per-cell state — zero cells
reveal as a cascade and never get flagged.

**Zero cells without their own bits.** A zero cell is displayed as
revealed iff every numbered cell on its connected zero-region's border
is revealed in state. That captures classic flood-fill: clicking a
numbered cell N marks just N; clicking a zero in region Z marks every
numbered cell on Z's border in one shot. The render pass walks each
region and asks "is every border cell revealed?" — if so, fill the
region with the revealed colour. The only quirk: revealing every border
cell of a region one-by-one will "auto-reveal" the region on the last
click. Rare and arguably correct.

**Board regeneration per tick.** `update` reconstructs the board from
the seed every tick — the seed alone doesn't pin it down, so we pick
`N_MINES` mines via `rng::next` and reject any placement that doesn't
yield exactly `N_NUMBERED` numbered cells. At 16.3% density the
expected numbered count sits near our target with modest variance, so
the rejection generator converges in a handful of attempts per tick.

**Win = correct flags.** Win is "every mine flagged AND no non-mine
flagged", not "every numbered cell revealed" — players have to actively
assert each mine, no early win from clearing the board.

*/
use bitwise_games::draw_command::{
    BLACK, BLUE, Color, DARK_GREY, DrawCommand, GREEN, LAVENDER, LIGHT_GREY, ORANGE, PINK, RED,
    WHITE, YELLOW,
};
use bitwise_games::font::{draw_text, glyph, text_width};
use bitwise_games::frame_buffer::{self, FrameBuffer};
use bitwise_games::rng;
use bitwise_games::{Game, Key};

const ROWS: usize = 7;
const COLS: usize = 7;
const N_CELLS: usize = ROWS * COLS; // 49
const N_MINES: usize = 8;

const SEED_BITS: u32 = 8;
const SEED_MASK: u64 = (1u64 << SEED_BITS) - 1;

/// Largest K such that (3^K + 1) · 2^(mines + seed_bits) ≤ 2^64. The +1
/// reserves a single sentinel state to mark "dead" — no explicit dead bit.
const fn max_numbered(mines: u32, seed_bits: u32) -> usize {
    let bits_left = 64 - seed_bits - mines;
    let bound: u128 = 1u128 << bits_left;
    let mut k: usize = 0;
    let mut p: u128 = 1;
    while p * 3 < bound {
        p *= 3;
        k += 1;
    }
    k
}

const N_NUMBERED: usize = max_numbered(N_MINES as u32, SEED_BITS);
const N_INTERACTIVE: usize = N_MINES + N_NUMBERED;

const fn pow3(n: usize) -> u64 {
    let mut p: u64 = 1;
    let mut i = 0;
    while i < n {
        p *= 3;
        i += 1;
    }
    p
}

/// Death sentinel — any numbered_part value ≥ this means "dead."
const DEAD_SENTINEL: u64 = pow3(N_NUMBERED);

// State values. Mines use only HIDDEN and FLAGGED (REVEALED is unused); on
// the wire each mine collapses to a single bit (FLAGGED → 1, HIDDEN → 0).
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

// Outer Windows-style chrome: 2-px raised bevel around the whole 128×128
// frame (WHITE top/left, DARK_GREY bottom/right), with a LIGHT_GREY
// background between the bevel and the grid.
const OUTER_BEVEL: u32 = 2;

// 14×14 smiley button at the top centre. Acts as a status indicator —
// smiles while playing, X-eyes on death, sunglasses on win.
const SMILEY_SIZE: u32 = 14;
const SMILEY_X: u32 = (frame_buffer::WIDTH - SMILEY_SIZE) / 2; // 57
const SMILEY_Y: u32 = 3;

// Search bound for valid mine placements. Hit rate ~10%; 1000 attempts is
// vastly more than needed in practice.
const MAX_GEN_ATTEMPTS: u32 = 1000;

// --- State encoding ---

struct State {
    seed: u16,
    dead: bool,
    cells: [u8; N_INTERACTIVE],
}

fn decode(state: u64) -> State {
    let seed = (state & SEED_MASK) as u16;
    let mut v = state >> SEED_BITS;
    let mut cells = [0u8; N_INTERACTIVE];
    for cell in cells.iter_mut().take(N_MINES) {
        *cell = if (v & 1) != 0 { FLAGGED } else { HIDDEN };
        v >>= 1;
    }
    // Sentinel: numbered_part ≥ 3^N_NUMBERED → dead, cells stay HIDDEN.
    let dead = v >= DEAD_SENTINEL;
    if !dead {
        for cell in cells.iter_mut().skip(N_MINES) {
            *cell = (v % 3) as u8;
            v /= 3;
        }
    }
    State { seed, dead, cells }
}

fn encode(state: &State) -> u64 {
    // High end first: pack numbered cells in base-3, or the dead sentinel.
    let mut v = if state.dead {
        DEAD_SENTINEL
    } else {
        let mut packed = 0u64;
        for i in (0..N_NUMBERED).rev() {
            packed = packed * 3 + state.cells[N_MINES + i] as u64;
        }
        packed
    };
    // Then mine flags as one bit each (FLAGGED → 1, HIDDEN → 0).
    for i in (0..N_MINES).rev() {
        v = (v << 1) | (state.cells[i] == FLAGGED) as u64;
    }
    (v << SEED_BITS) | (state.seed as u64 & SEED_MASK)
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

    // Cell index layout: mines occupy 0..N_MINES, numbered occupy
    // N_MINES..N_INTERACTIVE — each kind in row-major grid order. The
    // encoding relies on this split to pack mines as bits and numbered as
    // base-3 digits without consulting the board.
    let mut cell_to_idx = [255u8; N_CELLS];
    let mut n_mines_seen = 0usize;
    let mut n_numbered = 0usize;
    for cell in 0..N_CELLS as u8 {
        if is_mine[cell as usize] {
            cell_to_idx[cell as usize] = n_mines_seen as u8;
            n_mines_seen += 1;
        } else if counts[cell as usize] > 0 {
            if n_numbered >= N_NUMBERED {
                return None;
            }
            cell_to_idx[cell as usize] = (N_MINES + n_numbered) as u8;
            n_numbered += 1;
        }
    }
    if n_numbered != N_NUMBERED {
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
    let mut n_mines_seen = 0usize;
    let mut n_numbered = 0usize;
    for cell in 0..N_CELLS as u8 {
        if is_mine[cell as usize] && n_mines_seen < N_MINES {
            cell_to_idx[cell as usize] = n_mines_seen as u8;
            n_mines_seen += 1;
        } else if !is_mine[cell as usize] && counts[cell as usize] > 0 && n_numbered < N_NUMBERED {
            cell_to_idx[cell as usize] = (N_MINES + n_numbered) as u8;
            n_numbered += 1;
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

fn is_dead(state: &State, _board: &Board) -> bool {
    state.dead
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
        // Mines have no REVEALED state — death is a single global bit.
        state.dead = true;
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
    // Mines only toggle HIDDEN↔FLAGGED; numbered cells additionally have a
    // REVEALED state that flag must not overwrite. Both branches end up the
    // same since REVEALED never appears on a mine.
    let v = &mut state.cells[idx as usize];
    *v = match *v {
        HIDDEN => FLAGGED,
        FLAGGED => HIDDEN,
        _ => *v, // revealed numbered: no-op
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
    // Cross of spikes centred at (col 6, row 6) — shifted 1 px left and
    // 1 px up from the geometric cell centre for visual balance.
    commands.push(DrawCommand::rect(x + 6, y + 2, 1, 9, BLACK));
    commands.push(DrawCommand::rect(x + 2, y + 6, 9, 1, BLACK));
    // 5×5 body centred on the cross.
    commands.push(DrawCommand::rect(x + 4, y + 4, 5, 5, BLACK));
    // Single-pixel highlight glint, biased toward the upper-left.
    commands.push(DrawCommand::rect(x + 5, y + 5, 1, 1, WHITE));
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

/// 2-px raised bevel around the entire 128×128 frame.
fn draw_outer_chrome(commands: &mut Vec<DrawCommand>) {
    let w = frame_buffer::WIDTH;
    let h = frame_buffer::HEIGHT;
    // WHITE top + left highlight.
    commands.push(DrawCommand::rect(0, 0, w, OUTER_BEVEL, WHITE));
    commands.push(DrawCommand::rect(0, 0, OUTER_BEVEL, h, WHITE));
    // DARK_GREY bottom + right shadow.
    commands.push(DrawCommand::rect(
        0,
        h - OUTER_BEVEL,
        w,
        OUTER_BEVEL,
        DARK_GREY,
    ));
    commands.push(DrawCommand::rect(
        w - OUTER_BEVEL,
        0,
        OUTER_BEVEL,
        h,
        DARK_GREY,
    ));
}

/// Sunken inset frame around the mines-remaining counter, with a BLACK
/// background so the ORANGE digits read like a classic 7-segment display.
fn draw_counter_inset(commands: &mut Vec<DrawCommand>) {
    let pad: u32 = 2;
    let x = GRID_X - pad;
    let y = STATUS_Y - pad;
    let w = 3 * 3 * COUNTER_SCALE + 2 * COUNTER_SCALE + 2 * pad; // 2 digits + gap + padding
    let h = 5 * COUNTER_SCALE + 2 * pad; // digit height + padding
    // Sunken bevel: DARK_GREY top/left, WHITE bottom/right.
    commands.push(DrawCommand::rect(x, y, w, 1, DARK_GREY));
    commands.push(DrawCommand::rect(x, y, 1, h, DARK_GREY));
    commands.push(DrawCommand::rect(x, y + h - 1, w, 1, WHITE));
    commands.push(DrawCommand::rect(x + w - 1, y, 1, h, WHITE));
    // BLACK panel interior.
    commands.push(DrawCommand::rect(x + 1, y + 1, w - 2, h - 2, BLACK));
}

/// 14×14 smiley face button at the top centre. Three states drive the
/// expression: live (smile), dead (X eyes + frown), won (sunglasses).
fn draw_smiley(commands: &mut Vec<DrawCommand>, dead: bool, won: bool) {
    let bx = SMILEY_X;
    let by = SMILEY_Y;
    let bs = SMILEY_SIZE;

    // Raised button bevel: WHITE top/left, DARK_GREY bottom/right, LIGHT_GREY body.
    commands.push(DrawCommand::rect(bx, by, bs, bs, DARK_GREY));
    commands.push(DrawCommand::rect(bx, by, bs - 1, bs - 1, WHITE));
    commands.push(DrawCommand::rect(
        bx + 1,
        by + 1,
        bs - 2,
        bs - 2,
        LIGHT_GREY,
    ));

    // Yellow face: 10×10 with corner pixels trimmed back to body for a rounded look.
    let fx = bx + 2;
    let fy = by + 2;
    commands.push(DrawCommand::rect(fx, fy, 10, 10, YELLOW));
    commands.push(DrawCommand::rect(fx, fy, 1, 1, LIGHT_GREY));
    commands.push(DrawCommand::rect(fx + 9, fy, 1, 1, LIGHT_GREY));
    commands.push(DrawCommand::rect(fx, fy + 9, 1, 1, LIGHT_GREY));
    commands.push(DrawCommand::rect(fx + 9, fy + 9, 1, 1, LIGHT_GREY));

    if dead {
        // X eyes — 3×3 cross per eye, separated by a 2-px gap.
        for &(ex, ey) in &[(fx + 1, fy + 2), (fx + 6, fy + 2)] {
            commands.push(DrawCommand::rect(ex, ey, 1, 1, BLACK));
            commands.push(DrawCommand::rect(ex + 2, ey, 1, 1, BLACK));
            commands.push(DrawCommand::rect(ex + 1, ey + 1, 1, 1, BLACK));
            commands.push(DrawCommand::rect(ex, ey + 2, 1, 1, BLACK));
            commands.push(DrawCommand::rect(ex + 2, ey + 2, 1, 1, BLACK));
        }
        // Frown at rows 6–7, leaving a clear empty row 5 between eyes and mouth.
        commands.push(DrawCommand::rect(fx + 2, fy + 6, 6, 1, BLACK));
        commands.push(DrawCommand::rect(fx + 1, fy + 7, 1, 1, BLACK));
        commands.push(DrawCommand::rect(fx + 8, fy + 7, 1, 1, BLACK));
    } else if won {
        // Two 3×2 lenses joined by a 2-px bridge at the top, with angled
        // temple arms extending up + out from each lens's upper-outer corner
        // into the surrounding body chrome.
        commands.push(DrawCommand::rect(fx + 1, fy + 3, 3, 2, BLACK));
        commands.push(DrawCommand::rect(fx + 6, fy + 3, 3, 2, BLACK));
        commands.push(DrawCommand::rect(fx + 4, fy + 3, 2, 1, BLACK));
        // Left leg: 2-pixel diagonal.
        commands.push(DrawCommand::rect(fx, fy + 2, 1, 1, BLACK));
        commands.push(DrawCommand::rect(fx - 1, fy + 1, 1, 1, BLACK));
        // Right leg: mirror.
        commands.push(DrawCommand::rect(fx + 9, fy + 2, 1, 1, BLACK));
        commands.push(DrawCommand::rect(fx + 10, fy + 1, 1, 1, BLACK));
        // Smile at rows 6–7.
        commands.push(DrawCommand::rect(fx + 2, fy + 7, 6, 1, BLACK));
        commands.push(DrawCommand::rect(fx + 1, fy + 6, 1, 1, BLACK));
        commands.push(DrawCommand::rect(fx + 8, fy + 6, 1, 1, BLACK));
    } else {
        // Happy: 2 eye dots at row 3, smile at rows 6–7.
        commands.push(DrawCommand::rect(fx + 3, fy + 3, 1, 1, BLACK));
        commands.push(DrawCommand::rect(fx + 6, fy + 3, 1, 1, BLACK));
        commands.push(DrawCommand::rect(fx + 2, fy + 7, 6, 1, BLACK));
        commands.push(DrawCommand::rect(fx + 1, fy + 6, 1, 1, BLACK));
        commands.push(DrawCommand::rect(fx + 8, fy + 6, 1, 1, BLACK));
    }
}

/// Centred key-binding hint in the bottom margin of the chrome.
fn draw_instructions(commands: &mut Vec<DrawCommand>) {
    let text = b"Z=REVEAL X=FLAG";
    let scale = 1u32;
    let w = text_width(text.len(), scale);
    let x = (frame_buffer::WIDTH - w) / 2;
    // Vertically centred between the grid's bottom edge and the outer bevel.
    let top = GRID_Y + GRID_PX;
    let bottom = frame_buffer::HEIGHT - OUTER_BEVEL;
    let y = top + (bottom - top - 5 * scale) / 2;
    draw_text(commands, text, x, y, scale, DARK_GREY);
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

    // LIGHT_GREY chrome behind everything, then the outer 2-px raised bevel.
    commands.push(DrawCommand::rect(
        0,
        0,
        frame_buffer::WIDTH,
        frame_buffer::HEIGHT,
        LIGHT_GREY,
    ));
    draw_outer_chrome(&mut commands);

    let dead = is_dead(state, board);
    let won = !dead && is_won(state, board);

    // Counter panel on the left of the status strip, smiley face in the middle.
    draw_counter_inset(&mut commands);
    draw_smiley(&mut commands, dead, won);
    draw_instructions(&mut commands);

    // The grid area starts as a uniform BLACK block; every cell paints just
    // its 12×12 interior on top, leaving 1-px BLACK pixels between cells as
    // shared borders and a closing border around the grid's outer edge.
    commands.push(DrawCommand::rect(GRID_X, GRID_Y, GRID_PX, GRID_PX, BLACK));

    // First pass: backgrounds (reveal status determines visual). On death,
    // every cell flips to revealed — the full board is shown as game-over.
    for cell in 0..N_CELLS as u8 {
        let revealed = if dead {
            true
        } else {
            let idx = board.cell_to_idx[cell as usize];
            if idx != 255 {
                state.cells[idx as usize] == REVEALED
            } else {
                let region = board.cell_to_region[cell as usize];
                region != 255 && region_revealed(state, board, region as usize)
            }
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
            if dead || cell_state == FLAGGED {
                // On death every mine shows; otherwise only flagged ones.
                if dead {
                    draw_mine(&mut commands, cell, false);
                } else {
                    draw_flag(&mut commands, cell);
                }
            }
        } else if board.counts[cell as usize] > 0 {
            if dead || cell_state == REVEALED {
                draw_number(&mut commands, cell, board.counts[cell as usize]);
            } else if cell_state == FLAGGED {
                draw_flag(&mut commands, cell);
            }
        }
        // Zero cells: no glyph regardless of state.
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
        dead: false,
        cells: [HIDDEN; N_INTERACTIVE],
    }
}

struct Minesweeper;

impl Game for Minesweeper {
    const NAME: &'static str = "Minesweeper";
    const FPS: usize = 30;

    fn init(args: Vec<String>) -> (u64, FrameBuffer) {
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
    fn alive_state_round_trips() {
        // Alive states preserve seed, mine flags, and every numbered cell.
        for seed in [0u16, 1, 42, 127, 255] {
            let mut s = fresh_state(seed);
            for i in 0..N_MINES {
                s.cells[i] = if i % 2 == 0 { HIDDEN } else { FLAGGED };
            }
            for i in 0..N_NUMBERED {
                s.cells[N_MINES + i] = (i % 3) as u8;
            }
            let bits = encode(&s);
            let d = decode(bits);
            assert_eq!(d.seed, s.seed);
            assert!(!d.dead);
            assert_eq!(d.cells, s.cells);
        }
    }

    #[test]
    fn dead_state_round_trips() {
        // Death sentinel preserves seed, dead flag, and mine flags; numbered
        // cells collapse to HIDDEN (the snapshot at death is intentionally lossy).
        for seed in [0u16, 1, 42, 127, 255] {
            let mut s = fresh_state(seed);
            s.dead = true;
            for i in 0..N_MINES {
                s.cells[i] = if i % 2 == 0 { HIDDEN } else { FLAGGED };
            }
            for i in 0..N_NUMBERED {
                s.cells[N_MINES + i] = (i % 3) as u8;
            }
            let bits = encode(&s);
            let d = decode(bits);
            assert_eq!(d.seed, s.seed);
            assert!(d.dead);
            // Mine flag state survives.
            for i in 0..N_MINES {
                assert_eq!(d.cells[i], s.cells[i]);
            }
            // Numbered cells reset to HIDDEN on death.
            for i in 0..N_NUMBERED {
                assert_eq!(d.cells[N_MINES + i], HIDDEN);
            }
        }
    }

    #[test]
    fn every_seed_finds_a_valid_board() {
        // Hit-rate sanity: every 8-bit seed must converge within
        // MAX_GEN_ATTEMPTS to a board with N_MINES mines and exactly
        // N_NUMBERED numbered cells.
        for seed in 0..256u16 {
            let board = generate_board(seed);
            let n_mines = board.is_mine.iter().filter(|&&m| m).count();
            let n_numbered = (0..N_CELLS as u8)
                .filter(|&c| !board.is_mine[c as usize] && board.counts[c as usize] > 0)
                .count();
            assert_eq!(n_mines, N_MINES, "seed {seed}: wrong mine count");
            assert_eq!(n_numbered, N_NUMBERED, "seed {seed}: wrong numbered count");
        }
    }
}
