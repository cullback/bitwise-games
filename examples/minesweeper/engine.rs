//! Standalone minesweeper board generator + deductive solver.
//!
//! Lets you dial in a board: size, mine count, which solver rules the
//! generated board must be clearable under, and constraints on what counts
//! as an "interesting" result (cascade size, 3BV, numbered-cell density).
//!
//! # Pattern references
//!
//! All the named multi-cell patterns from competitive minesweeper (1-1,
//! 1-2, 1-2-1, 1-2-2-1, "triangle", etc.) reduce to *subset reasoning*: if
//! one revealed number's hidden-neighbor set is a subset of another's, the
//! difference is its own constraint and often forces a flag or a reveal.
//! `RuleSet::subset_propagation` covers them all without hard-coded
//! pattern matching.
//!
//! - <https://minesweeper-pro.com/advanced-patterns/>
//! - <https://minesweepergame.com/strategy/patterns.php>
//!
//! # Limits
//!
//! Hidden-cell sets ride in `u128` bitmasks, so total cells must fit in
//! 128. That covers everything up to 10×12 / 11×11 / 12×10 (≤120 cells).
//! Larger boards would need a wider bitmask type.

#![allow(dead_code)]

use bitwise_games::rng;

// --- Types ---

#[derive(Clone, Debug)]
pub struct Board {
    pub rows: usize,
    pub cols: usize,
    pub mines: Vec<bool>,
    pub counts: Vec<u8>,
    pub zero_regions: Vec<ZeroRegion>,
    /// `i16::MAX` means the cell is not part of any zero region.
    pub cell_to_region: Vec<i16>,
}

impl Board {
    pub fn cells(&self) -> usize {
        self.rows * self.cols
    }

    pub fn xy(&self, cell: usize) -> (usize, usize) {
        (cell % self.cols, cell / self.cols)
    }

    pub fn index(&self, x: usize, y: usize) -> usize {
        y * self.cols + x
    }

    pub fn neighbors(&self, cell: usize) -> impl Iterator<Item = usize> + '_ {
        let (x, y) = self.xy(cell);
        let cols = self.cols as i32;
        let rows = self.rows as i32;
        (-1i32..=1).flat_map(move |dy| {
            (-1i32..=1).filter_map(move |dx| {
                if dx == 0 && dy == 0 {
                    return None;
                }
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || nx >= cols || ny < 0 || ny >= rows {
                    return None;
                }
                Some(ny as usize * cols as usize + nx as usize)
            })
        })
    }

    pub fn numbered_count(&self) -> usize {
        (0..self.cells())
            .filter(|&c| !self.mines[c] && self.counts[c] > 0)
            .count()
    }

    pub fn largest_region(&self) -> Option<&ZeroRegion> {
        self.zero_regions.iter().max_by_key(|r| r.size)
    }
}

#[derive(Clone, Debug)]
pub struct ZeroRegion {
    /// Zero cells in this connected component (row-major order).
    pub cells: Vec<u16>,
    /// Numbered cells adjacent to this region's zero cells. Cascading from
    /// any cell in the region reveals exactly these borders.
    pub border: Vec<u16>,
    pub size: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuleSet {
    /// Rule A: a revealed number whose `count - flags == hidden_count` →
    /// flag all hidden neighbors.
    pub count_matches_hidden: bool,
    /// Rule B: a revealed number whose `flags == count` → reveal all
    /// hidden neighbors.
    pub flags_satisfy_count: bool,
    /// Subset rule: for two revealed numbers whose hidden-neighbor sets A
    /// and B satisfy A ⊆ B, the derived constraint on B \\ A forces flags
    /// or reveals whenever its mine count saturates or zeroes out.
    pub subset_propagation: bool,
}

impl RuleSet {
    /// Just the single-cell rules — what beginners learn first.
    pub const fn basic() -> Self {
        Self {
            count_matches_hidden: true,
            flags_satisfy_count: true,
            subset_propagation: false,
        }
    }

    /// Everything implemented so far. Equivalent to mastery of every named
    /// pattern in the references at the top of this module.
    pub const fn full() -> Self {
        Self {
            count_matches_hidden: true,
            flags_satisfy_count: true,
            subset_propagation: true,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Constraints {
    /// Largest zero region must be at least this big (avoids
    /// claustrophobic openings).
    pub min_cascade_size: Option<usize>,
    pub max_cascade_size: Option<usize>,
    pub min_three_bv: Option<u32>,
    pub max_three_bv: Option<u32>,
    /// Cap on numbered-cell count (useful for bit-budget encodings).
    pub max_numbered: Option<usize>,
    /// If set, require solvability starting from this specific cell.
    /// Otherwise the largest zero region's first cell is used.
    pub start_cell: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub rows: usize,
    pub cols: usize,
    pub mines: usize,
    pub rules: RuleSet,
    pub constraints: Constraints,
    pub max_attempts: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolveResult {
    Cleared,
    /// Solver ran out of progress with at least one hidden non-mine cell
    /// still unrevealed. Board needs guessing or unsupported deduction.
    Stuck,
}

// --- Generation ---

/// Try up to `cfg.max_attempts` random mine placements; return the first
/// that satisfies the configured constraints AND is solvable under the
/// configured rule set. Deterministic for a given `seed`.
pub fn generate(cfg: &Config, seed: u64) -> Option<Board> {
    assert!(
        cfg.rows * cfg.cols <= 128,
        "engine supports up to 128 cells (got {}×{})",
        cfg.rows,
        cfg.cols
    );
    assert!(cfg.mines < cfg.rows * cfg.cols, "more mines than cells");

    for k in 0..cfg.max_attempts {
        let h = rng::next(seed ^ (k as u64).wrapping_mul(0x9E37_79B1));
        let mines = pick_mines(cfg.rows, cfg.cols, cfg.mines, h);
        let board = build_board(cfg.rows, cfg.cols, &mines);
        if !passes_constraints(&board, cfg) {
            continue;
        }
        let start = match cfg.constraints.start_cell {
            Some(c) => c,
            None => match board.largest_region() {
                Some(r) => r.cells[0] as usize,
                None => continue,
            },
        };
        if solve(&board, start, &cfg.rules) != SolveResult::Cleared {
            continue;
        }
        return Some(board);
    }
    None
}

pub fn pick_mines(rows: usize, cols: usize, n_mines: usize, seed: u64) -> Vec<usize> {
    let n_cells = rows * cols;
    let mut chosen = Vec::with_capacity(n_mines);
    let mut rng_state = seed | 1;
    while chosen.len() < n_mines {
        rng_state = rng::next(rng_state);
        let cell = (rng_state % n_cells as u64) as usize;
        if !chosen.contains(&cell) {
            chosen.push(cell);
        }
    }
    chosen
}

pub fn build_board(rows: usize, cols: usize, mine_cells: &[usize]) -> Board {
    let n = rows * cols;
    let mut mines = vec![false; n];
    for &m in mine_cells {
        mines[m] = true;
    }

    let mut board = Board {
        rows,
        cols,
        mines,
        counts: vec![0u8; n],
        zero_regions: Vec::new(),
        cell_to_region: vec![i16::MAX; n],
    };

    for cell in 0..n {
        if board.mines[cell] {
            continue;
        }
        let count = board.neighbors(cell).filter(|&n| board.mines[n]).count() as u8;
        board.counts[cell] = count;
    }

    // Flood fill the zero regions and record borders.
    let mut stack = Vec::new();
    for start in 0..n {
        if board.mines[start] || board.counts[start] != 0 {
            continue;
        }
        if board.cell_to_region[start] != i16::MAX {
            continue;
        }
        let region_idx = board.zero_regions.len() as i16;
        let mut region_cells = Vec::new();
        let mut border_seen = vec![false; n];
        let mut border = Vec::new();

        stack.push(start);
        while let Some(c) = stack.pop() {
            if board.cell_to_region[c] != i16::MAX {
                continue;
            }
            board.cell_to_region[c] = region_idx;
            region_cells.push(c as u16);
            for nb in board.neighbors(c) {
                if board.mines[nb] {
                    continue;
                }
                if board.counts[nb] == 0 {
                    if board.cell_to_region[nb] == i16::MAX {
                        stack.push(nb);
                    }
                } else if !border_seen[nb] {
                    border_seen[nb] = true;
                    border.push(nb as u16);
                }
            }
        }
        let size = region_cells.len();
        board.zero_regions.push(ZeroRegion {
            cells: region_cells,
            border,
            size,
        });
    }

    board
}

fn passes_constraints(board: &Board, cfg: &Config) -> bool {
    let cs = &cfg.constraints;
    let largest = board.zero_regions.iter().map(|r| r.size).max().unwrap_or(0);
    if matches!(cs.min_cascade_size, Some(min) if largest < min) {
        return false;
    }
    if matches!(cs.max_cascade_size, Some(max) if largest > max) {
        return false;
    }
    if matches!(cs.max_numbered, Some(max) if board.numbered_count() > max) {
        return false;
    }
    if cs.min_three_bv.is_some() || cs.max_three_bv.is_some() {
        let bv = three_bv(board);
        if matches!(cs.min_three_bv, Some(min) if bv < min) {
            return false;
        }
        if matches!(cs.max_three_bv, Some(max) if bv > max) {
            return false;
        }
    }
    true
}

// --- 3BV ---

/// Bechtel's Board Benchmark Value: minimum left-clicks to clear the
/// board. Each zero region = 1 click (cascade opens the whole region plus
/// its numbered border); each numbered cell *not* on any region border = 1
/// click (must be opened individually).
pub fn three_bv(board: &Board) -> u32 {
    let mut bv = board.zero_regions.len() as u32;
    let mut is_border = vec![false; board.cells()];
    for region in &board.zero_regions {
        for &c in &region.border {
            is_border[c as usize] = true;
        }
    }
    for (c, &border) in is_border.iter().enumerate() {
        if !board.mines[c] && board.counts[c] > 0 && !border {
            bv += 1;
        }
    }
    bv
}

// --- Solver ---

/// Simulate playing the board starting from `start` using only the
/// deductions enabled in `rules`. Returns `Cleared` iff every non-mine
/// cell ends up revealed.
pub fn solve(board: &Board, start: usize, rules: &RuleSet) -> SolveResult {
    let n = board.cells();
    let mut revealed = vec![false; n];
    let mut flagged = vec![false; n];
    cascade_reveal(board, &mut revealed, start);

    loop {
        let mut progress = false;
        if rules.count_matches_hidden || rules.flags_satisfy_count {
            progress |= apply_single_cell_rules(board, &mut revealed, &mut flagged, rules);
        }
        if rules.subset_propagation {
            progress |= apply_subset_rule(board, &mut revealed, &mut flagged);
        }
        if !progress {
            break;
        }
    }

    if (0..n).all(|c| board.mines[c] || revealed[c]) {
        SolveResult::Cleared
    } else {
        SolveResult::Stuck
    }
}

fn cascade_reveal(board: &Board, revealed: &mut [bool], start: usize) {
    let mut stack = vec![start];
    while let Some(cell) = stack.pop() {
        if revealed[cell] || board.mines[cell] {
            continue;
        }
        revealed[cell] = true;
        if board.counts[cell] == 0 {
            for nb in board.neighbors(cell) {
                if !revealed[nb] && !board.mines[nb] {
                    stack.push(nb);
                }
            }
        }
    }
}

fn apply_single_cell_rules(
    board: &Board,
    revealed: &mut [bool],
    flagged: &mut [bool],
    rules: &RuleSet,
) -> bool {
    let mut progress = false;
    for cell in 0..board.cells() {
        if !revealed[cell] || board.mines[cell] || board.counts[cell] == 0 {
            continue;
        }
        let count = board.counts[cell] as usize;
        let mut hidden = [0usize; 8];
        let mut hidden_n = 0;
        let mut flag_count = 0;
        for nb in board.neighbors(cell) {
            if flagged[nb] {
                flag_count += 1;
            } else if !revealed[nb] {
                hidden[hidden_n] = nb;
                hidden_n += 1;
            }
        }
        if hidden_n == 0 {
            continue;
        }
        if rules.count_matches_hidden && hidden_n + flag_count == count {
            for &c in &hidden[..hidden_n] {
                if !flagged[c] {
                    flagged[c] = true;
                    progress = true;
                }
            }
        } else if rules.flags_satisfy_count && flag_count == count {
            for &c in &hidden[..hidden_n] {
                if !revealed[c] {
                    cascade_reveal(board, revealed, c);
                    progress = true;
                }
            }
        }
    }
    progress
}

struct Constraint {
    hidden: u128,
    mines: u32,
}

fn apply_subset_rule(board: &Board, revealed: &mut [bool], flagged: &mut [bool]) -> bool {
    // Each revealed numbered cell with hidden neighbors becomes a
    // constraint "exactly `mines` of these hidden cells are mines".
    let mut cs: Vec<Constraint> = Vec::new();
    for cell in 0..board.cells() {
        if !revealed[cell] || board.mines[cell] || board.counts[cell] == 0 {
            continue;
        }
        let mut hidden = 0u128;
        let mut flag_count = 0u32;
        for nb in board.neighbors(cell) {
            if flagged[nb] {
                flag_count += 1;
            } else if !revealed[nb] {
                hidden |= 1u128 << nb;
            }
        }
        if hidden == 0 {
            continue;
        }
        cs.push(Constraint {
            hidden,
            mines: board.counts[cell] as u32 - flag_count,
        });
    }

    let mut progress = false;
    for i in 0..cs.len() {
        for j in 0..cs.len() {
            if i == j {
                continue;
            }
            // Is constraint i a subset of constraint j?
            if cs[i].hidden & cs[j].hidden != cs[i].hidden {
                continue;
            }
            let diff = cs[j].hidden & !cs[i].hidden;
            if diff == 0 {
                continue;
            }
            let diff_mines = cs[j].mines - cs[i].mines;
            let diff_count = diff.count_ones();
            if diff_mines == 0 {
                // Every cell in `diff` is safe.
                let mut bits = diff;
                while bits != 0 {
                    let c = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    if !revealed[c] && !flagged[c] {
                        cascade_reveal(board, revealed, c);
                        progress = true;
                    }
                }
            } else if diff_mines == diff_count {
                // Every cell in `diff` is a mine.
                let mut bits = diff;
                while bits != 0 {
                    let c = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    if !flagged[c] {
                        flagged[c] = true;
                        progress = true;
                    }
                }
            }
        }
    }
    progress
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_board_computes_counts() {
        // 3×3 with mine at center (cell 4). All 8 neighbors see exactly 1 mine.
        let board = build_board(3, 3, &[4]);
        for c in 0..9 {
            if c == 4 {
                continue;
            }
            assert_eq!(board.counts[c], 1, "cell {c}");
        }
    }

    #[test]
    fn empty_board_is_one_big_zero_region() {
        let board = build_board(4, 4, &[]);
        assert_eq!(board.zero_regions.len(), 1);
        assert_eq!(board.zero_regions[0].size, 16);
        assert!(board.zero_regions[0].border.is_empty());
        assert_eq!(three_bv(&board), 1);
    }

    #[test]
    fn three_bv_corner_mine() {
        // 3×3 with mine at corner: cells (1,0), (0,1), (1,1) are 1's;
        // cells (2,0), (0,2), (1,2), (2,1), (2,2) form one zero region.
        // 3BV = 1 region + 0 isolated numbered = 1.
        let board = build_board(3, 3, &[0]);
        assert_eq!(three_bv(&board), 1);
    }

    #[test]
    fn three_bv_split_board() {
        // 5×1 with mine at cell 2 splits the row into two isolated zero
        // regions plus two numbered borders.
        // [0 1 m 1 0]: 2 zero regions, both 1's are on borders.
        let board = build_board(1, 5, &[2]);
        assert_eq!(board.zero_regions.len(), 2);
        assert_eq!(three_bv(&board), 2);
    }

    #[test]
    fn basic_rules_clear_a_simple_board() {
        // 3×3, corner mine. Cascade from far corner reveals everything
        // except the mine; rule A flags the mine.
        let board = build_board(3, 3, &[0]);
        assert_eq!(solve(&board, 8, &RuleSet::basic()), SolveResult::Cleared);
    }

    #[test]
    fn subset_rule_clears_otherwise_stuck_boards() {
        // Sweep a few hundred random 8×9 boards. Count how many basic rules
        // fail to clear but the full rule set does. Should be > 0 if subset
        // propagation is doing real work.
        let mut helped = 0;
        for seed in 0..500u64 {
            let h = rng::next(seed);
            let mines = pick_mines(8, 9, 13, h);
            let board = build_board(8, 9, &mines);
            let Some(region) = board.largest_region() else {
                continue;
            };
            let start = region.cells[0] as usize;
            let basic = solve(&board, start, &RuleSet::basic());
            let full = solve(&board, start, &RuleSet::full());
            if basic == SolveResult::Stuck && full == SolveResult::Cleared {
                helped += 1;
            }
        }
        assert!(
            helped > 0,
            "subset rule should clear at least one otherwise-stuck board"
        );
        eprintln!("subset rule cleared {helped} extra boards out of 500");
    }

    #[test]
    fn generate_respects_constraints() {
        let cfg = Config {
            rows: 8,
            cols: 9,
            mines: 13,
            rules: RuleSet::full(),
            constraints: Constraints {
                min_cascade_size: Some(10),
                max_numbered: Some(50),
                ..Default::default()
            },
            max_attempts: 1000,
        };
        let board = generate(&cfg, 42).expect("should find a board");
        assert!(board.largest_region().unwrap().size >= 10);
        assert!(board.numbered_count() <= 50);
    }

    #[test]
    fn generate_full_finds_more_boards_than_basic() {
        // Same seed budget; the larger acceptance set under `full` should
        // succeed at least as often as `basic` (usually noticeably more).
        let make = |rules| Config {
            rows: 8,
            cols: 9,
            mines: 13,
            rules,
            constraints: Constraints::default(),
            max_attempts: 200,
        };
        let basic_cfg = make(RuleSet::basic());
        let full_cfg = make(RuleSet::full());
        let mut basic_hits = 0;
        let mut full_hits = 0;
        for seed in 0..50u64 {
            if generate(&basic_cfg, seed).is_some() {
                basic_hits += 1;
            }
            if generate(&full_cfg, seed).is_some() {
                full_hits += 1;
            }
        }
        eprintln!("basic: {basic_hits}/50, full: {full_hits}/50");
        assert!(full_hits >= basic_hits);
    }
}
