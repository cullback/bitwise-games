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
seed. At 14.3% mine density, expected interactive count ≈ 33.4 with
modest variance — the rejection generator converges in a handful of
attempts per tick. Search bound is generous to avoid edge-case lockup.

# Input

- Mouse position drives a hover highlight only — it never enters state.
  Mouse coords come in as framebuffer pixels (0..128).
- Z reveals the hovered cell.
- X toggles a flag on the hovered cell (hidden ↔ flagged).
- After win/lose, Z or X restarts with a fresh seed derived from rng::next.

*/
use bitwise_games::draw_command::{
    BLACK, BLUE, BROWN, Color, DARK_BLUE, DARK_GREEN, DARK_GREY, DARK_PURPLE, DrawCommand, GREEN,
    LAVENDER, LIGHT_GREY, ORANGE, RED, WHITE,
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

// Layout: 16-px top status bar with the mines-remaining counter, then a 7×7
// grid at 16 px/cell = 112×112, centred horizontally. Total: 16 + 112 = 128.
const CELL_PX: u32 = 16;
const GRID_PX: u32 = COLS as u32 * CELL_PX;
const STATUS_H: u32 = 16;
const GRID_X: u32 = (frame_buffer::WIDTH - GRID_PX) / 2; // 8
const GRID_Y: u32 = STATUS_H;
const STATUS_Y: u32 = 3;
const DIGIT_SCALE: u32 = 2;

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
    (0..N_CELLS as u8).all(|cell| {
        if board.is_mine[cell as usize] || board.counts[cell as usize] == 0 {
            return true;
        }
        let idx = board.cell_to_idx[cell as usize];
        idx == 255 || state.cells[idx as usize] == REVEALED
    })
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
    match n {
        1 => BLUE,
        2 => DARK_GREEN,
        3 => RED,
        4 => DARK_BLUE,
        5 => BROWN,
        6 => LAVENDER,
        7 => DARK_GREY,
        _ => DARK_PURPLE,
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
    let (x, y) = cell_xy(cell);
    if revealed {
        // Revealed: light grey fill with a thin darker frame so the grid
        // structure stays visible inside the flood area.
        commands.push(DrawCommand::rect(x, y, CELL_PX, CELL_PX, DARK_GREY));
        commands.push(DrawCommand::rect(
            x + 1,
            y + 1,
            CELL_PX - 1,
            CELL_PX - 1,
            LIGHT_GREY,
        ));
    } else {
        // Hidden: raised tile look — light top/left, dark bottom/right.
        commands.push(DrawCommand::rect(x, y, CELL_PX, CELL_PX, DARK_GREY));
        commands.push(DrawCommand::rect(
            x + 1,
            y + 1,
            CELL_PX - 2,
            CELL_PX - 2,
            BROWN,
        ));
    }
}

fn draw_flag(commands: &mut Vec<DrawCommand>, cell: u8) {
    let (x, y) = cell_xy(cell);
    // Pole + right-pointing triangular pennant + base, sized for a 16-px cell.
    commands.push(DrawCommand::rect(x + 5, y + 3, 1, 11, BLACK));
    commands.push(DrawCommand::rect(x + 6, y + 3, 6, 1, RED));
    commands.push(DrawCommand::rect(x + 6, y + 4, 5, 1, RED));
    commands.push(DrawCommand::rect(x + 6, y + 5, 4, 1, RED));
    commands.push(DrawCommand::rect(x + 6, y + 6, 3, 1, RED));
    commands.push(DrawCommand::rect(x + 6, y + 7, 2, 1, RED));
    commands.push(DrawCommand::rect(x + 6, y + 8, 1, 1, RED));
    commands.push(DrawCommand::rect(x + 3, y + 13, 9, 1, BLACK));
}

fn draw_mine(commands: &mut Vec<DrawCommand>, cell: u8, exploded: bool) {
    let (x, y) = cell_xy(cell);
    if exploded {
        commands.push(DrawCommand::rect(
            x + 1,
            y + 1,
            CELL_PX - 2,
            CELL_PX - 2,
            RED,
        ));
    }
    // Cross of spikes through the centre.
    commands.push(DrawCommand::rect(x + 7, y + 3, 2, 10, BLACK));
    commands.push(DrawCommand::rect(x + 3, y + 7, 10, 2, BLACK));
    // Octagonal body: 8×8 square with corner pixels clipped.
    commands.push(DrawCommand::rect(x + 4, y + 4, 8, 8, BLACK));
    // Highlight glint.
    commands.push(DrawCommand::rect(x + 6, y + 5, 2, 2, WHITE));
}

fn draw_number(commands: &mut Vec<DrawCommand>, cell: u8, count: u8) {
    let (x, y) = cell_xy(cell);
    // 3×5 digit at scale 2 = 6×10, centred in a 16×16 cell.
    draw_digit(
        commands,
        count,
        x + (CELL_PX - 3 * DIGIT_SCALE) / 2,
        y + (CELL_PX - 5 * DIGIT_SCALE) / 2,
        DIGIT_SCALE,
        number_color(count),
    );
}

fn draw_hover(commands: &mut Vec<DrawCommand>, cell: u8) {
    let (x, y) = cell_xy(cell);
    // 1-pixel white frame.
    commands.push(DrawCommand::rect(x, y, CELL_PX, 1, WHITE));
    commands.push(DrawCommand::rect(x, y + CELL_PX - 1, CELL_PX, 1, WHITE));
    commands.push(DrawCommand::rect(x, y, 1, CELL_PX, WHITE));
    commands.push(DrawCommand::rect(x + CELL_PX - 1, y, 1, CELL_PX, WHITE));
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
                commands.push(DrawCommand::rect(x + 3, y + 3, CELL_PX - 6, 1, RED));
                commands.push(DrawCommand::rect(
                    x + 3,
                    y + CELL_PX - 4,
                    CELL_PX - 6,
                    1,
                    RED,
                ));
            }
        }
    }

    // Hover frame — only when game is live and a cell is hovered.
    if !dead && !won {
        if let Some(cell) = hover {
            draw_hover(&mut commands, cell);
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
    // Mines-remaining counter, drawn at scale 2 along the status strip below
    // the grid. Sign bar appears when the player has placed more flags than
    // there are mines.
    let scale = 2u32;
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
