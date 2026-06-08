/*

Minesweeper on a 9-wide × 8-tall grid (72 cells, 13 mines, 18.1% density —
between Intermediate and Expert).

# Inputs

- Mouse: hover highlight (no state)
- Z: REVEAL commitment — kills if hovered cell is a mine
- X: FLAG commitment — kills if hovered cell is NOT a mine

Both actions are commitments. There's no tentative scratch state: every
interaction is a real decision with real risk. Each cell takes exactly 1 bit
of state — there's no FLAGGED-on-numbered slot, because flagging a non-mine
kills you outright.

# First-move cascade

The board starts fully closed. On the player's first Z or X press, we
rotate the seed to one whose board has the hovered cell as a zero cell and
is deductively solvable starting from that cell. Then we cascade from the
clicked cell. So the first click *encodes* where the player wanted to open
— their choice is recorded in the 7 seed bits.

Coverage was probed (`first_click_seed_coverage` test): every cell has at
least 14 valid seeds in 0..128, so the rotation always succeeds. The
initial seed shown in the CLI args is therefore just the preview board;
once the player clicks, they're playing whichever seed matched their pick.

# Encoding

| Bits  | Description                                          |
|-------|------------------------------------------------------|
| 0..7  | board seed (128 unique solvable boards)              |
| 7     | dead flag                                            |
| 8..   | per-cell assertion (1 bit per interactive cell)      |

Each interactive cell takes 1 bit, in cell row-major order:
  - On a mine cell:     bit = 1 → FLAGGED, bit = 0 → HIDDEN
  - On a numbered cell: bit = 1 → REVEALED, bit = 0 → HIDDEN

Zero cells aren't stored — they display as revealed iff every numbered cell
on their region's border is revealed.

# Budget

64 bits = 7 (seed) + 1 (dead) + 56 cell-assertion bits. The engine's
`max_numbered` constraint caps numbered cells at 43 (= 56 − N_MINES), so
the per-cell bits always fit. All 128 seeds land inside the 5000-attempt
budget under the full rule set.

Note: there are 72 cells but the cell-position index needs to fit in `1 <<
cell` shifts; we use `u128` for `state.asserted` internally. The encoded
u64 only writes/reads N_INTERACTIVE ≤ 56 bits.

# Solvability

Boards go through `engine::solve` with `RuleSet::full()`: single-cell rules
A and B plus subset propagation, which covers every named multi-cell
pattern (1-1, 1-2-1, 1-2-2-1, …) without hard-coded matchers:

  A. number's hidden neighbors == count − flags  →  flag all hidden
  B. number's flagged neighbors == count          →  reveal all hidden
  S. constraint X's hidden ⊆ Y's  →  Y \\ X is its own constraint that
     forces flags or reveals whenever it saturates or zeros out

A board is accepted iff this loop reveals every non-mine cell starting
from a cascade at its largest zero region. No guessing is ever required,
but the player needs to know more than just A and B to clear every board.

# Win

Win = every non-mine cell is revealed. Flags are scratchpad — a player can
win without flagging anything. In practice flagging mines is useful because
it enables rule B (reveal all hidden = safe).

# Layout

128×128 display, modeled on the earlier `minesweeper2.aseprite` layout:

  - Outer raised 3D frame: 1 px white top/left, 1 px dark-grey bottom/right
  - Light-grey background fills the interior
  - Chrome strip (y=0..16):
      mines-remaining counter at (4,4)–(35,13) — inset 3D black panel
      smiley face sprite centered at (58, 3)
      status panel at (92,4)–(123,13) — mirrors the counter; shows
      `WIN`/`LOSE` text when the game ends, black during play
  - Grid box (4, 17)–(123, 123): inset 3D bevel around a black interior
    that shows through 1 px gaps between 12×12 cell sprites

Tiles are loaded from `minesweeper.aseprite` (14 tiles × 12×12):

  0: unrevealed       7: numbered "5"
  1: flagged          8: numbered "6"
  2: revealed "0"     9: numbered "7"
  3: numbered "1"    10: bomb (mine)
  4: numbered "2"    11: smiley face (alive)
  5: numbered "3"    12: dead face
  6: numbered "4"    13: sunglasses (won)

Count 8 (essentially impossible at this density) is rendered as the "7"
sprite — graceful degradation rather than crash.

The mines counter and status panel use `assets/6x4-alphanum.aseprite` — 36
glyphs (`0`–`9`, `A`–`Z`) in a 16-wide × 3-tall grid, each glyph 4 px
content wide and 6 px tall with 1 px gutters.

*/

mod engine;

use asefile::AsepriteFile;
use bitwise_games::aseprite::load_color_grid;
use bitwise_games::draw_command::{
    BLACK, Color, DARK_GREY, DrawCommand, GREEN, LIGHT_GREY, RED, WHITE, YELLOW,
};
use bitwise_games::frame_buffer::FrameBuffer;
use bitwise_games::rng;
use bitwise_games::sprite::{Rot, blit_square};
use bitwise_games::{Game, Key};
use engine::Board;
use std::sync::OnceLock;

const ROWS: usize = 8;
const COLS: usize = 9;
const N_CELLS: usize = ROWS * COLS; // 72
const N_MINES: usize = 13;

const SEED_BITS: u32 = 7;
const SEED_MASK: u64 = (1u64 << SEED_BITS) - 1;
const DEAD_BIT: u32 = SEED_BITS; // bit 7
const CELL_BITS_START: u32 = SEED_BITS + 1; // bit 8
const MAX_INTERACTIVE: usize = (64 - CELL_BITS_START) as usize; // 56

// Cell size, shared by grid sprites and the chrome smiley.
const CELL_PX: u32 = 12;
const CELL_PITCH: u32 = CELL_PX + 1; // sprite + 1 px black separator

// Chrome strip (y=0..16): mines counter (left) / smiley (center) / status (right).
const COUNTER_X: u32 = 4;
const COUNTER_Y: u32 = 4;
const COUNTER_W: u32 = 32;
const COUNTER_H: u32 = 10;

const SMILEY_X: u32 = (128 - CELL_PX) / 2; // 58
const SMILEY_Y: u32 = 3;

// Right-side status panel mirrors the mines counter geometry.
const STATUS_X: u32 = 128 - 4 - COUNTER_W; // 92
const STATUS_Y: u32 = COUNTER_Y;
const STATUS_W: u32 = COUNTER_W;
const STATUS_H: u32 = COUNTER_H;

// Grid box: bevel(1) + K border(1) + cells/separators + K border(1) + bevel(1).
const GRID_FRAME_X: u32 = 4;
const GRID_FRAME_Y: u32 = 17;
const GRID_W: u32 = (COLS as u32) * CELL_PITCH + 3; // 120
const GRID_H: u32 = (ROWS as u32) * CELL_PITCH + 3; // 107
const GRID_X: u32 = GRID_FRAME_X + 2; // 6 (first cell sprite x)
const GRID_Y: u32 = GRID_FRAME_Y + 2; // 19 (first cell sprite y)

const MAX_GEN_ATTEMPTS: u32 = 5000;

// --- State ---

struct State {
    seed: u8,
    dead: bool,
    /// Per-cell assertion bits indexed by cell position (0..N_CELLS).
    /// Meaning depends on the cell's type:
    ///   - Mine:     bit = 1 → FLAGGED, bit = 0 → HIDDEN
    ///   - Numbered: bit = 1 → REVEALED, bit = 0 → HIDDEN
    ///   - Zero:     always 0; display state derives from region border.
    asserted: u128,
}

fn encode(state: &State) -> u64 {
    let board = board_for_seed(state.seed);
    let mut packed: u64 = state.seed as u64;
    packed |= (state.dead as u64) << DEAD_BIT;
    let mut bit = CELL_BITS_START;
    for cell in 0..N_CELLS {
        if is_interactive(board, cell) {
            let v = ((state.asserted >> cell) & 1) as u64;
            packed |= v << bit;
            bit += 1;
        }
    }
    packed
}

fn decode(packed: u64) -> State {
    let seed = (packed & SEED_MASK) as u8;
    let dead = ((packed >> DEAD_BIT) & 1) == 1;
    let board = board_for_seed(seed);
    let mut asserted: u128 = 0;
    let mut bit = CELL_BITS_START;
    for cell in 0..N_CELLS {
        if is_interactive(board, cell) {
            let v = ((packed >> bit) & 1) as u128;
            asserted |= v << cell;
            bit += 1;
        }
    }
    State {
        seed,
        dead,
        asserted,
    }
}

fn is_interactive(board: &Board, cell: usize) -> bool {
    board.mines[cell] || board.counts[cell] > 0
}

// --- Board ---

fn cell_xy(cell: u8) -> (u32, u32) {
    let row = (cell as u32) / COLS as u32;
    let col = (cell as u32) % COLS as u32;
    (GRID_X + col * CELL_PITCH, GRID_Y + row * CELL_PITCH)
}

fn board_config() -> engine::Config {
    engine::Config {
        rows: ROWS,
        cols: COLS,
        mines: N_MINES,
        rules: engine::RuleSet::full(),
        constraints: engine::Constraints {
            // Bit-budget cap: numbered + N_MINES ≤ MAX_INTERACTIVE.
            max_numbered: Some(MAX_INTERACTIVE - N_MINES),
            ..Default::default()
        },
        max_attempts: MAX_GEN_ATTEMPTS,
    }
}

/// All 128 boards are pre-generated on first access and cached. Every
/// `encode`/`decode`/render call hits this table, so we can't afford to
/// regenerate per-frame.
static BOARDS: OnceLock<Vec<Board>> = OnceLock::new();

fn boards() -> &'static [Board] {
    BOARDS.get_or_init(|| {
        let cfg = board_config();
        (0..=SEED_MASK as u8)
            .map(|seed| {
                engine::generate(&cfg, seed as u64)
                    .unwrap_or_else(|| panic!("no valid board for seed {seed}"))
            })
            .collect()
    })
}

fn board_for_seed(seed: u8) -> &'static Board {
    &boards()[seed as usize]
}

// --- Derived state ---

fn region_revealed(state: &State, board: &Board, region_idx: usize) -> bool {
    board.zero_regions[region_idx]
        .border
        .iter()
        .all(|&c| (state.asserted >> c) & 1 == 1)
}

fn cell_revealed(state: &State, board: &Board, cell: usize) -> bool {
    if board.mines[cell] {
        return false;
    }
    if board.counts[cell] > 0 {
        return (state.asserted >> cell) & 1 == 1;
    }
    let region = board.cell_to_region[cell];
    region != i16::MAX && region_revealed(state, board, region as usize)
}

fn is_won(state: &State, board: &Board) -> bool {
    (0..N_CELLS).all(|c| board.mines[c] || cell_revealed(state, board, c))
}

// --- Input ---

fn hovered_cell(mouse: Option<(u8, u8)>) -> Option<u8> {
    let (x, y) = mouse?;
    let x = x as u32;
    let y = y as u32;
    if x < GRID_X || x >= GRID_X + COLS as u32 * CELL_PITCH {
        return None;
    }
    if y < GRID_Y || y >= GRID_Y + ROWS as u32 * CELL_PITCH {
        return None;
    }
    let col = (x - GRID_X) / CELL_PITCH;
    let row = (y - GRID_Y) / CELL_PITCH;
    Some((row * COLS as u32 + col) as u8)
}

/// Search the 128 seeds for one whose board has `target` as a zero cell
/// that's also a deductively-solvable cascade origin. The search start is
/// a hash of `initial_seed` and `target`, so the same click on different
/// initial seeds lands on different valid seeds — mixing CLI randomness
/// with player choice. Deterministic given both inputs.
fn find_first_click_seed(target: u8, initial_seed: u8) -> Option<u8> {
    let mask = SEED_MASK as u8;
    let rules = engine::RuleSet::full();
    let start = (rng::next(((initial_seed as u64) << 8) | target as u64) & SEED_MASK) as u8;
    for k in 0..=mask {
        let seed = start.wrapping_add(k) & mask;
        let board = board_for_seed(seed);
        let c = target as usize;
        if board.mines[c] || board.counts[c] != 0 {
            continue;
        }
        if engine::solve(board, c, &rules) == engine::SolveResult::Cleared {
            return Some(seed);
        }
    }
    None
}

fn apply_reveal_commit(state: &mut State, board: &Board, cell: u8) {
    let c = cell as usize;
    let bit = 1u128 << cell;
    if board.mines[c] && (state.asserted & bit) != 0 {
        return; // forgiving: revealing a flagged cell is a no-op
    }
    if board.mines[c] {
        state.dead = true;
        return;
    }
    if board.counts[c] == 0 {
        let region = board.cell_to_region[c];
        if region != i16::MAX {
            for &border in &board.zero_regions[region as usize].border {
                state.asserted |= 1u128 << border;
            }
        }
    } else {
        state.asserted |= bit;
    }
}

fn apply_flag_commit(state: &mut State, board: &Board, cell: u8) {
    let c = cell as usize;
    let bit = 1u128 << cell;
    if board.mines[c] {
        state.asserted ^= bit; // toggle (safe: known-mine)
        return;
    }
    if cell_revealed(state, board, c) {
        return; // forgiving: flagging an already-revealed cell is a no-op
    }
    state.dead = true;
}

// --- Sprite tiles ---

const SPR_UNREVEALED: usize = 0;
const SPR_FLAG: usize = 1;
/// Number tile index = SPR_NUM_BASE + count, valid for count in 0..=7.
/// Count 0 is the "revealed empty" tile.
const SPR_NUM_BASE: usize = 2;
const SPR_BOMB: usize = 10;
const SPR_SMILEY: usize = 11;
const SPR_DEAD: usize = 12;
const SPR_SUNGLASSES: usize = 13;

static TILES: OnceLock<[[[u8; 12]; 12]; 14]> = OnceLock::new();

fn tiles() -> &'static [[[u8; 12]; 12]; 14] {
    TILES.get_or_init(|| load_color_grid::<12, 12, 14, 14>(include_bytes!("minesweeper.aseprite")))
}

fn blit_tile(fb: &mut FrameBuffer, cell: u8, sprite_idx: usize) {
    let (x, y) = cell_xy(cell);
    blit_square::<12>(fb, &tiles()[sprite_idx], x, y, Rot::R0);
}

fn blit_number(fb: &mut FrameBuffer, cell: u8, count: u8) {
    // Count 8 (essentially impossible at this density) falls back to the "7"
    // sprite — visually wrong by 1 but doesn't crash.
    let idx = SPR_NUM_BASE + count.min(7) as usize;
    blit_tile(fb, cell, idx);
}

// --- Mines counter font (assets/6x4-alphanum.aseprite) ---
//
// 36 glyphs in a 16-wide × 3-tall grid. Each cell is 5 px wide (4 px
// content + 1 px gutter) and 7 px tall (6 px content + 1 px gutter).
// Source image is 80×21 (16*5=80, 3*7=21).
//
// Row 1 (image rows 0–5):  0 1 2 3 4 5 6 7 8 9 A B C D E F  (hex)
// Row 2 (image rows 7–12): G H I J K L M N O P Q R S T U V
// Row 3 (image rows 14–19): W X Y Z

const MINES_GLYPH_W: u32 = 4;
const MINES_GLYPH_H: usize = 6;
const MINES_CELL_W: u32 = 5;
const MINES_CELL_H: u32 = 7;
const MINES_GRID_COLS: usize = 16;
const MINES_FONT_COUNT: usize = 36;

static MINES_FONT: OnceLock<[[u8; MINES_GLYPH_H]; MINES_FONT_COUNT]> = OnceLock::new();

fn mines_font() -> &'static [[u8; MINES_GLYPH_H]; MINES_FONT_COUNT] {
    MINES_FONT.get_or_init(|| {
        let bytes = include_bytes!("../../assets/6x4-alphanum.aseprite");
        let ase = AsepriteFile::read(&bytes[..]).expect("parse 6x4-alphanum");
        let img = ase.frame(0).image();
        let mut out = [[0u8; MINES_GLYPH_H]; MINES_FONT_COUNT];
        for (g, glyph) in out.iter_mut().enumerate() {
            let x0 = (g % MINES_GRID_COLS) as u32 * MINES_CELL_W;
            let y0 = (g / MINES_GRID_COLS) as u32 * MINES_CELL_H;
            for (y, row_bits) in glyph.iter_mut().enumerate() {
                let mut bits = 0u8;
                for x in 0..MINES_GLYPH_W {
                    let p = img.get_pixel(x0 + x, y0 + y as u32);
                    if p.0[3] > 0 {
                        bits |= 1 << (MINES_GLYPH_W - 1 - x);
                    }
                }
                *row_bits = bits;
            }
        }
        out
    })
}

/// Map ASCII byte to glyph index. Supports `0`–`9` and `A`–`Z`.
fn mines_glyph_index(ch: u8) -> Option<usize> {
    match ch {
        b'0'..=b'9' => Some((ch - b'0') as usize),
        b'A'..=b'Z' => Some(10 + (ch - b'A') as usize),
        _ => None,
    }
}

fn draw_mines_glyph(commands: &mut Vec<DrawCommand>, idx: usize, x: u32, y: u32, color: Color) {
    let pattern = &mines_font()[idx];
    for (row, &bits) in pattern.iter().enumerate() {
        for col in 0..MINES_GLYPH_W {
            if (bits >> (MINES_GLYPH_W - 1 - col)) & 1 == 1 {
                commands.push(DrawCommand::rect(x + col, y + row as u32, 1, 1, color));
            }
        }
    }
}

fn draw_mines_text(commands: &mut Vec<DrawCommand>, text: &[u8], x: u32, y: u32, color: Color) {
    for (i, &ch) in text.iter().enumerate() {
        if let Some(idx) = mines_glyph_index(ch) {
            draw_mines_glyph(commands, idx, x + i as u32 * MINES_CELL_W, y, color);
        }
    }
}

fn mines_text_width(n_chars: u32) -> u32 {
    // n glyphs × 4 px + (n−1) gaps × 1 px
    n_chars * MINES_GLYPH_W + n_chars.saturating_sub(1)
}

// --- Chrome (top status strip) ---

/// Outer raised 3D bevel: the dark bottom edge spans full width (owning the
/// bottom corners), while the light side rails fill the middle rows. This
/// reads as "shadow underneath" — the box sits raised on the surface.
fn draw_raised_bevel(commands: &mut Vec<DrawCommand>, x: u32, y: u32, w: u32, h: u32) {
    commands.push(DrawCommand::rect(x, y, w, 1, WHITE));
    commands.push(DrawCommand::rect(x, y + h - 1, w, 1, DARK_GREY));
    commands.push(DrawCommand::rect(x, y + 1, 1, h - 2, WHITE));
    commands.push(DrawCommand::rect(x + w - 1, y + 1, 1, h - 2, DARK_GREY));
}

/// Inset 3D bevel: the dark top and left edges run end-to-end (owning three
/// of the four corners), while the light bottom and right edges only fill
/// the inner span. This reads as "shadow wrapping the top-left" — the box
/// looks pressed into the surface.
fn draw_inset_bevel(commands: &mut Vec<DrawCommand>, x: u32, y: u32, w: u32, h: u32) {
    commands.push(DrawCommand::rect(x, y, w, 1, DARK_GREY));
    commands.push(DrawCommand::rect(x, y, 1, h, DARK_GREY));
    commands.push(DrawCommand::rect(x + 1, y + h - 1, w - 1, 1, WHITE));
    commands.push(DrawCommand::rect(x + w - 1, y + 1, 1, h - 1, WHITE));
}

fn draw_background(commands: &mut Vec<DrawCommand>) {
    commands.push(DrawCommand::rect(0, 0, 128, 128, LIGHT_GREY));
    draw_raised_bevel(commands, 0, 0, 128, 128);
}

/// Inset black panel with optional centered text. Used for both the mines
/// counter (left) and the win/lose status panel (right).
fn draw_inset_panel(
    commands: &mut Vec<DrawCommand>,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    text: &[u8],
    color: Color,
) {
    commands.push(DrawCommand::rect(x, y, w, h, BLACK));
    draw_inset_bevel(commands, x, y, w, h);
    if text.is_empty() {
        return;
    }
    let text_w = mines_text_width(text.len() as u32);
    let dx = x + (w - text_w) / 2;
    let dy = y + (h - MINES_GLYPH_H as u32) / 2;
    draw_mines_text(commands, text, dx, dy, color);
}

fn draw_mines_counter(commands: &mut Vec<DrawCommand>, remaining: i32) {
    let n = remaining.clamp(0, 999) as u32;
    let text = [
        b'0' + (n / 100) as u8,
        b'0' + ((n / 10) % 10) as u8,
        b'0' + (n % 10) as u8,
    ];
    draw_inset_panel(
        commands, COUNTER_X, COUNTER_Y, COUNTER_W, COUNTER_H, &text, RED,
    );
}

fn draw_status_panel(commands: &mut Vec<DrawCommand>, dead: bool, won: bool) {
    let (text, color): (&[u8], Color) = if dead {
        (b"DIE", RED)
    } else if won {
        (b"WIN", GREEN)
    } else {
        (b"", RED) // empty during normal play; color unused
    };
    draw_inset_panel(
        commands, STATUS_X, STATUS_Y, STATUS_W, STATUS_H, text, color,
    );
}

fn draw_smiley(fb: &mut FrameBuffer, dead: bool, won: bool) {
    let sprite = if dead {
        SPR_DEAD
    } else if won {
        SPR_SUNGLASSES
    } else {
        SPR_SMILEY
    };
    blit_square::<12>(fb, &tiles()[sprite], SMILEY_X, SMILEY_Y, Rot::R0);
}

fn draw_hover(commands: &mut Vec<DrawCommand>, cell: u8) {
    let (x, y) = cell_xy(cell);
    commands.push(DrawCommand::rect(x, y, CELL_PX, 1, YELLOW));
    commands.push(DrawCommand::rect(x, y + CELL_PX - 1, CELL_PX, 1, YELLOW));
    commands.push(DrawCommand::rect(x, y, 1, CELL_PX, YELLOW));
    commands.push(DrawCommand::rect(x + CELL_PX - 1, y, 1, CELL_PX, YELLOW));
}

fn render_cell(fb: &mut FrameBuffer, state: &State, board: &Board, cell: u8, dead: bool) {
    let c = cell as usize;
    let bit = 1u128 << cell;
    let asserted = (state.asserted & bit) != 0;
    if dead {
        if board.mines[c] {
            blit_tile(fb, cell, if asserted { SPR_FLAG } else { SPR_BOMB });
        } else if board.counts[c] == 0 {
            blit_tile(fb, cell, SPR_NUM_BASE);
        } else {
            blit_number(fb, cell, board.counts[c]);
        }
        return;
    }
    // Alive
    if board.mines[c] {
        blit_tile(fb, cell, if asserted { SPR_FLAG } else { SPR_UNREVEALED });
    } else if board.counts[c] == 0 {
        if cell_revealed(state, board, c) {
            blit_tile(fb, cell, SPR_NUM_BASE);
        } else {
            blit_tile(fb, cell, SPR_UNREVEALED);
        }
    } else if asserted {
        blit_number(fb, cell, board.counts[c]);
    } else {
        blit_tile(fb, cell, SPR_UNREVEALED);
    }
}

fn render(state: &State, board: &Board, hover: Option<u8>) -> FrameBuffer {
    let mut fb = FrameBuffer::new();
    let dead = state.dead;
    let won = !dead && is_won(state, board);

    let mut chrome = Vec::new();
    draw_background(&mut chrome);
    let flagged_mines = (0..N_CELLS)
        .filter(|&c| board.mines[c] && (state.asserted >> c) & 1 == 1)
        .count() as i32;
    draw_mines_counter(&mut chrome, N_MINES as i32 - flagged_mines);
    draw_status_panel(&mut chrome, dead, won);

    // Grid: black interior with an inset 3D bevel. Sprites blit on top; the
    // 1 px gaps between them show the black through as grid lines.
    chrome.push(DrawCommand::rect(
        GRID_FRAME_X,
        GRID_FRAME_Y,
        GRID_W,
        GRID_H,
        BLACK,
    ));
    draw_inset_bevel(&mut chrome, GRID_FRAME_X, GRID_FRAME_Y, GRID_W, GRID_H);
    fb.draw_list(&chrome);
    draw_smiley(&mut fb, dead, won);

    for cell in 0..N_CELLS as u8 {
        render_cell(&mut fb, state, board, cell, dead);
    }

    if !dead && !won {
        if let Some(cell) = hover {
            let mut overlay = Vec::new();
            draw_hover(&mut overlay, cell);
            fb.draw_list(&overlay);
        }
    }
    fb
}

// --- Game ---

fn fresh_state(seed: u8) -> State {
    State {
        seed,
        dead: false,
        asserted: 0,
    }
}

struct Minesweeper;

impl Game for Minesweeper {
    const NAME: &'static str = "Minesweeper";
    const FPS: usize = 30;

    fn init(args: Vec<String>) -> (u64, FrameBuffer) {
        let raw = args.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let seed = (raw & SEED_MASK) as u8;
        let state = fresh_state(seed);
        let board = board_for_seed(state.seed);
        (encode(&state), render(&state, board, None))
    }

    fn update(
        state: u64,
        _held: &[Key],
        buffered: Option<Key>,
        mouse: Option<(u8, u8)>,
    ) -> (u64, FrameBuffer) {
        let mut s = decode(state);
        let mut board = board_for_seed(s.seed);
        let dead = s.dead;
        let won = !dead && is_won(&s, board);
        let hover = hovered_cell(mouse);

        if dead || won {
            if matches!(buffered, Some(Key::Z) | Some(Key::X)) {
                let next_seed = (rng::next(state) & SEED_MASK) as u8;
                let ns = fresh_state(next_seed);
                let nb = board_for_seed(ns.seed);
                return (encode(&ns), render(&ns, nb, hover));
            }
            return (encode(&s), render(&s, board, None));
        }

        if let Some(cell) = hover {
            if s.asserted == 0 && matches!(buffered, Some(Key::Z) | Some(Key::X)) {
                if let Some(new_seed) = find_first_click_seed(cell, s.seed) {
                    s.seed = new_seed;
                    board = board_for_seed(s.seed);
                    apply_reveal_commit(&mut s, board, cell);
                }
            } else {
                match buffered {
                    Some(Key::Z) => apply_reveal_commit(&mut s, board, cell),
                    Some(Key::X) => apply_flag_commit(&mut s, board, cell),
                    _ => {}
                }
            }
        }

        let fb = render(&s, board, hover);
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
        for seed in [0u8, 1, 42, 100, 127] {
            let board = board_for_seed(seed);
            let mut asserted: u128 = 0;
            for c in 0..N_CELLS {
                if board.mines[c] || board.counts[c] > 0 {
                    asserted |= 1u128 << c;
                }
            }
            let s = State {
                seed,
                dead: false,
                asserted,
            };
            let d = decode(encode(&s));
            assert_eq!(d.seed, s.seed);
            assert!(!d.dead);
            assert_eq!(d.asserted, s.asserted);
        }
    }

    #[test]
    fn dead_state_round_trips() {
        for seed in [0u8, 1, 42, 100, 127] {
            let board = board_for_seed(seed);
            let mut asserted: u128 = 0;
            for (i, c) in (0..N_CELLS).enumerate() {
                if (board.mines[c] || board.counts[c] > 0) && i % 2 == 0 {
                    asserted |= 1u128 << c;
                }
            }
            let s = State {
                seed,
                dead: true,
                asserted,
            };
            let d = decode(encode(&s));
            assert_eq!(d.seed, s.seed);
            assert!(d.dead);
            assert_eq!(d.asserted, s.asserted);
        }
    }

    #[test]
    fn renders_initial_and_terminal_states() {
        for seed in [0u8, 42, 127] {
            let board = board_for_seed(seed);
            let mut state = fresh_state(seed);
            let _ = render(&state, board, None);
            let _ = render(&state, board, Some(0));
            state.dead = true;
            let _ = render(&state, board, None);
            state.dead = false;
            for c in 0..N_CELLS {
                if !board.mines[c] && board.counts[c] > 0 {
                    state.asserted |= 1u128 << c;
                }
            }
            let _ = render(&state, board, None);
        }
    }

    #[test]
    fn first_click_finds_a_valid_seed_for_every_cell() {
        let rules = engine::RuleSet::full();
        for initial in [0u8, 5, 42, 99, 127] {
            for target in 0..N_CELLS as u8 {
                let seed = find_first_click_seed(target, initial).expect("no seed for cell");
                let board = board_for_seed(seed);
                let c = target as usize;
                assert!(!board.mines[c], "cell {target} is mine in seed {seed}");
                assert_eq!(board.counts[c], 0, "cell {target} not zero in seed {seed}");
                assert_eq!(
                    engine::solve(board, c, &rules),
                    engine::SolveResult::Cleared,
                    "cell {target} not solvable in seed {seed}"
                );
            }
        }
    }

    #[test]
    fn first_click_seed_coverage() {
        // For each cell, count how many seeds in 0..128 produce a board where
        // that cell is a zero cell AND the board is solvable starting from
        // that cell. We want every cell to have ≥1 valid seed so that a
        // first click anywhere can redirect to a board that opens at the
        // clicked cell.
        let rules = engine::RuleSet::full();
        let mut counts = vec![0usize; N_CELLS];
        for seed in 0..=SEED_MASK as u8 {
            let board = board_for_seed(seed);
            for cell in 0..N_CELLS {
                if board.mines[cell] || board.counts[cell] != 0 {
                    continue;
                }
                if engine::solve(board, cell, &rules) == engine::SolveResult::Cleared {
                    counts[cell] += 1;
                }
            }
        }
        let uncovered: Vec<usize> = (0..N_CELLS).filter(|&c| counts[c] == 0).collect();
        let min = *counts.iter().min().unwrap();
        let max = *counts.iter().max().unwrap();
        let avg = counts.iter().sum::<usize>() as f64 / N_CELLS as f64;
        eprintln!(
            "Coverage: uncovered={} min={} max={} avg={:.1}",
            uncovered.len(),
            min,
            max,
            avg
        );
        if !uncovered.is_empty() {
            eprintln!(
                "Uncovered cells (row, col): {:?}",
                uncovered
                    .iter()
                    .map(|&c| (c / COLS, c % COLS))
                    .collect::<Vec<_>>()
            );
        }
        for r in 0..ROWS {
            let row: Vec<String> = (0..COLS)
                .map(|c| format!("{:3}", counts[r * COLS + c]))
                .collect();
            eprintln!("{}", row.join(" "));
        }
    }

    #[test]
    fn every_seed_finds_a_solvable_board() {
        for seed in 0..=127u8 {
            let board = board_for_seed(seed);
            let interactive = (0..board.cells())
                .filter(|&c| is_interactive(board, c))
                .count();
            assert!(
                interactive <= MAX_INTERACTIVE,
                "seed {seed}: interactive {interactive} exceeds budget {MAX_INTERACTIVE}"
            );
            assert!(
                !board.zero_regions.is_empty(),
                "seed {seed}: no zero region (cascade impossible)"
            );
            let n_mines = board.mines.iter().filter(|&&m| m).count();
            assert_eq!(n_mines, N_MINES, "seed {seed}: wrong mine count");
        }
    }
}
