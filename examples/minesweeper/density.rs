// Empirical density experiments for minesweeper boards. For each (size, mine
// count), generates many random boards and reports:
//   - interactive cell count distribution
//   - what fraction fit the 55-bit encoding budget
//   - what fraction are solvable by the basic deductive solver
//   - what fraction both fit AND are solvable (= accepted by the real generator)
//
// Run: cargo run --release --example minesweeper_density

use bitwise_games::rng;

const BUDGET: usize = 55;

// Stripped-down board: just enough fields to compute interactive count and
// run the deductive solver. The real game stores more (zero regions, etc.)
// but those aren't needed for the experiment.
struct Board {
    rows: usize,
    cols: usize,
    is_mine: Vec<bool>,
    counts: Vec<u8>,
}

fn neighbors_8(rows: usize, cols: usize, cell: usize) -> Vec<usize> {
    let r = (cell / cols) as i32;
    let c = (cell % cols) as i32;
    let mut out = Vec::with_capacity(8);
    for dr in -1i32..=1 {
        for dc in -1i32..=1 {
            if dr == 0 && dc == 0 {
                continue;
            }
            let nr = r + dr;
            let nc = c + dc;
            if nr < 0 || nr >= rows as i32 || nc < 0 || nc >= cols as i32 {
                continue;
            }
            out.push((nr as usize) * cols + nc as usize);
        }
    }
    out
}

fn pick_mines(n_cells: usize, n_mines: usize, seed: u64) -> Vec<usize> {
    let mut chosen: Vec<usize> = Vec::with_capacity(n_mines);
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

fn build_board(mines: &[usize], rows: usize, cols: usize) -> Board {
    let n = rows * cols;
    let mut is_mine = vec![false; n];
    for &m in mines {
        is_mine[m] = true;
    }

    let mut counts = vec![0u8; n];
    for cell in 0..n {
        if is_mine[cell] {
            continue;
        }
        for nbr in neighbors_8(rows, cols, cell) {
            if is_mine[nbr] {
                counts[cell] += 1;
            }
        }
    }

    Board {
        rows,
        cols,
        is_mine,
        counts,
    }
}

fn n_interactive(board: &Board) -> usize {
    board
        .is_mine
        .iter()
        .zip(board.counts.iter())
        .filter(|&(&m, &c)| m || c > 0)
        .count()
}

fn cascade_reveal(board: &Board, revealed: &mut [bool], start: usize) {
    let mut stack = vec![start];
    while let Some(cell) = stack.pop() {
        if revealed[cell] || board.is_mine[cell] {
            continue;
        }
        revealed[cell] = true;
        if board.counts[cell] == 0 {
            for nbr in neighbors_8(board.rows, board.cols, cell) {
                if !revealed[nbr] && !board.is_mine[nbr] {
                    stack.push(nbr);
                }
            }
        }
    }
}

/// Number of cells revealed by cascading from `start`. Counts both the zero
/// region's cells and the bordering numbered cells.
fn cascade_size(board: &Board, start: usize) -> usize {
    let n = board.rows * board.cols;
    let mut revealed = vec![false; n];
    cascade_reveal(board, &mut revealed, start);
    revealed.iter().filter(|&&r| r).count()
}

/// Find the first cell of the largest connected zero region. Returns None if
/// the board has no zero cells.
fn largest_region_start(board: &Board) -> Option<usize> {
    let n = board.rows * board.cols;
    let mut visited = vec![false; n];
    let mut best: Option<(usize, usize)> = None; // (start, size)
    for cell in 0..n {
        if visited[cell] || board.is_mine[cell] || board.counts[cell] != 0 {
            continue;
        }
        let mut size = 0;
        let mut stack = vec![cell];
        let start = cell;
        while let Some(c) = stack.pop() {
            if visited[c] {
                continue;
            }
            visited[c] = true;
            size += 1;
            for nbr in neighbors_8(board.rows, board.cols, c) {
                if !visited[nbr] && !board.is_mine[nbr] && board.counts[nbr] == 0 {
                    stack.push(nbr);
                }
            }
        }
        if best.is_none_or(|(_, sz)| size > sz) {
            best = Some((start, size));
        }
    }
    best.map(|(start, _)| start)
}

struct SolveResult {
    iterations: usize,
    /// Cascade size starting from the first zero cell in row-major order
    /// (matches the game's current `cascade_start` behaviour).
    row_major_cascade: usize,
    /// Cascade size starting from any cell in the largest zero region.
    /// Bounds the upside if we switched start-cell strategy.
    largest_region_cascade: usize,
    total_zero_cells: usize,
    n_zero_regions: usize,
    max_count: u8,
}

fn board_is_solvable(board: &Board) -> bool {
    analyze_solve(board).is_some()
}

/// Solve and return diagnostic stats; None if unsolvable by simple deduction.
fn analyze_solve(board: &Board) -> Option<SolveResult> {
    let n = board.rows * board.cols;
    let start = (0..n).find(|&c| !board.is_mine[c] && board.counts[c] == 0)?;

    let mut revealed = vec![false; n];
    let mut flagged = vec![false; n];
    cascade_reveal(board, &mut revealed, start);
    let row_major_cascade = revealed.iter().filter(|&&r| r).count();
    let largest_region_cascade = largest_region_start(board)
        .map(|c| cascade_size(board, c))
        .unwrap_or(0);

    let mut iterations = 0;
    loop {
        let mut progress = false;
        for cell in 0..n {
            if !revealed[cell] || board.is_mine[cell] || board.counts[cell] == 0 {
                continue;
            }
            let count = board.counts[cell] as usize;
            let mut hidden = Vec::new();
            let mut flag_count = 0;
            for nbr in neighbors_8(board.rows, board.cols, cell) {
                if flagged[nbr] {
                    flag_count += 1;
                } else if !revealed[nbr] {
                    hidden.push(nbr);
                }
            }
            if hidden.is_empty() {
                continue;
            }
            if hidden.len() + flag_count == count {
                for &nbr in &hidden {
                    flagged[nbr] = true;
                }
                progress = true;
            } else if flag_count == count {
                for &nbr in &hidden {
                    cascade_reveal(board, &mut revealed, nbr);
                }
                progress = true;
            }
        }
        iterations += 1;
        if !progress {
            break;
        }
    }

    if !(0..n).all(|c| board.is_mine[c] || revealed[c]) {
        return None;
    }

    let total_zero_cells = (0..n)
        .filter(|&c| !board.is_mine[c] && board.counts[c] == 0)
        .count();

    // Count distinct zero regions.
    let mut visited = vec![false; n];
    let mut n_zero_regions = 0;
    for cell in 0..n {
        if visited[cell] || board.is_mine[cell] || board.counts[cell] != 0 {
            continue;
        }
        n_zero_regions += 1;
        let mut stack = vec![cell];
        while let Some(c) = stack.pop() {
            if visited[c] {
                continue;
            }
            visited[c] = true;
            for nbr in neighbors_8(board.rows, board.cols, c) {
                if !visited[nbr] && !board.is_mine[nbr] && board.counts[nbr] == 0 {
                    stack.push(nbr);
                }
            }
        }
    }

    let max_count = board.counts.iter().copied().max().unwrap_or(0);

    Some(SolveResult {
        iterations,
        row_major_cascade,
        largest_region_cascade,
        total_zero_cells,
        n_zero_regions,
        max_count,
    })
}

struct Stats {
    mean_interactive: f64,
    p50: usize,
    p95: usize,
    max: usize,
    fits_budget: f64,
    solvable: f64,
    accepted: f64,
}

fn run_experiment(rows: usize, cols: usize, mines: usize, samples: usize) -> Stats {
    let n_cells = rows * cols;
    let mut interactive = Vec::with_capacity(samples);
    let mut fits_count = 0;
    let mut solvable_count = 0;
    let mut accepted_count = 0;
    let mut rng_state = (rows as u64 * 1000 + cols as u64) * 100 + mines as u64;

    for _ in 0..samples {
        rng_state = rng::next(rng_state);
        let mine_positions = pick_mines(n_cells, mines, rng_state);
        let board = build_board(&mine_positions, rows, cols);
        let ni = n_interactive(&board);
        interactive.push(ni);
        let fits = ni <= BUDGET;
        let solvable = board_is_solvable(&board);
        if fits {
            fits_count += 1;
        }
        if solvable {
            solvable_count += 1;
        }
        if fits && solvable {
            accepted_count += 1;
        }
    }

    interactive.sort();
    let mean_interactive = interactive.iter().sum::<usize>() as f64 / samples as f64;
    let p50 = interactive[samples / 2];
    let p95 = interactive[samples * 95 / 100];
    let max = interactive[samples - 1];

    Stats {
        mean_interactive,
        p50,
        p95,
        max,
        fits_budget: 100.0 * fits_count as f64 / samples as f64,
        solvable: 100.0 * solvable_count as f64 / samples as f64,
        accepted: 100.0 * accepted_count as f64 / samples as f64,
    }
}

fn analyze_hardness(rows: usize, cols: usize, mines: usize, tries: usize) {
    let n_cells = rows * cols;
    let mut rng_state = ((rows as u64) << 24) | ((cols as u64) << 16) | (mines as u64) << 8 | 0xAB;

    let mut accepted: Vec<SolveResult> = Vec::new();
    for _ in 0..tries {
        rng_state = rng::next(rng_state);
        let mine_positions = pick_mines(n_cells, mines, rng_state);
        let board = build_board(&mine_positions, rows, cols);
        if n_interactive(&board) > BUDGET {
            continue;
        }
        if let Some(result) = analyze_solve(&board) {
            accepted.push(result);
        }
    }

    if accepted.is_empty() {
        println!(
            "=== {}x{} + {} mines: NO accepted boards",
            rows, cols, mines
        );
        return;
    }

    let n = accepted.len();
    let baseline_rate = 100.0 * n as f64 / tries as f64;

    println!(
        "=== {}x{} + {} mines (density {:.1}%, baseline accept {:.2}%, sample of {} solvable boards) ===",
        rows,
        cols,
        mines,
        100.0 * mines as f64 / n_cells as f64,
        baseline_rate,
        n
    );

    let mut rm: Vec<usize> = accepted.iter().map(|r| r.row_major_cascade).collect();
    let mut lr: Vec<usize> = accepted.iter().map(|r| r.largest_region_cascade).collect();
    let iters: Vec<usize> = accepted.iter().map(|r| r.iterations).collect();
    let zeros: Vec<usize> = accepted.iter().map(|r| r.total_zero_cells).collect();
    let regions: Vec<usize> = accepted.iter().map(|r| r.n_zero_regions).collect();
    rm.sort();
    lr.sort();
    let mean_rm = rm.iter().sum::<usize>() as f64 / n as f64;
    let mean_lr = lr.iter().sum::<usize>() as f64 / n as f64;
    let mean_iters = iters.iter().sum::<usize>() as f64 / n as f64;
    let mean_zeros = zeros.iter().sum::<usize>() as f64 / n as f64;
    let mean_regions = regions.iter().sum::<usize>() as f64 / n as f64;

    println!(
        "  zero structure: {:.1} zero cells, {:.1} regions, {:.1} cells/region avg",
        mean_zeros,
        mean_regions,
        mean_zeros / mean_regions.max(0.1)
    );
    println!(
        "  solver: {:.1} iterations to clear (higher = more deduction work)",
        mean_iters
    );
    println!();
    println!("  opening cascade sizes (cells revealed on first move):");
    println!(
        "    row-major-first start:    mean {:>5.1}   p10 {:>3}   p50 {:>3}   p90 {:>3}",
        mean_rm,
        rm[n / 10],
        rm[n / 2],
        rm[n * 9 / 10]
    );
    println!(
        "    largest-region start:     mean {:>5.1}   p10 {:>3}   p50 {:>3}   p90 {:>3}",
        mean_lr,
        lr[n / 10],
        lr[n / 2],
        lr[n * 9 / 10]
    );
    let below_10_rm = accepted.iter().filter(|r| r.row_major_cascade < 10).count();
    let below_10_lr = accepted
        .iter()
        .filter(|r| r.largest_region_cascade < 10)
        .count();
    println!(
        "    % below claustrophobia threshold (< 10 cells):  row-major {:>4.1}%   largest {:>4.1}%",
        100.0 * below_10_rm as f64 / n as f64,
        100.0 * below_10_lr as f64 / n as f64,
    );

    let max5 = accepted.iter().filter(|r| r.max_count >= 5).count();
    let max6 = accepted.iter().filter(|r| r.max_count >= 6).count();
    let max7 = accepted.iter().filter(|r| r.max_count >= 7).count();
    let max8 = accepted.iter().filter(|r| r.max_count == 8).count();
    println!(
        "  max numbered value (any cell ≥ k):  ≥5 {:>5.1}%   ≥6 {:>5.1}%   ≥7 {:>5.1}%   =8 {:>5.2}%",
        100.0 * max5 as f64 / n as f64,
        100.0 * max6 as f64 / n as f64,
        100.0 * max7 as f64 / n as f64,
        100.0 * max8 as f64 / n as f64,
    );
    println!();
}

fn main() {
    let samples = 2000;

    println!("=== Section 1: density distribution ===");
    println!(
        "{} samples per config, budget {} interactive cells",
        samples, BUDGET
    );
    println!();
    println!(
        "{:>5} {:>5} {:>6} {:>5} {:>5} {:>4} {:>4} {:>4} {:>9} {:>9} {:>9}",
        "size", "mines", "dens%", "mean", "p50", "p95", "max", "—", "fits%", "solv%", "accept%"
    );
    println!("{}", "-".repeat(86));

    let configs = [
        (7, 7, vec![10, 11, 12]),
        (8, 8, vec![12, 13, 14]),
        (9, 8, vec![12, 13]),
        (9, 9, vec![11, 12, 13]),
    ];

    for (rows, cols, mines_list) in &configs {
        for &mines in mines_list {
            let s = run_experiment(*rows, *cols, mines, samples);
            let density = 100.0 * mines as f64 / ((rows * cols) as f64);
            println!(
                "{:>2}x{:<2} {:>5} {:>5.1} {:>5.1} {:>5} {:>4} {:>4}     {:>8.1} {:>8.1} {:>8.1}",
                rows,
                cols,
                mines,
                density,
                s.mean_interactive,
                s.p50,
                s.p95,
                s.max,
                s.fits_budget,
                s.solvable,
                s.accepted,
            );
        }
        println!();
    }

    println!();
    println!("=== Section 2: hardness filter scenarios ===");
    println!("Goal: find a filter combo that keeps accept rate above ~0.5% (so 1000");
    println!("seed attempts finds a board >99% of the time) but selects harder boards.");
    println!();

    let tries = 30_000;
    for &(rows, cols, mines) in &[(8, 8, 13), (9, 9, 14), (9, 9, 13), (9, 9, 12), (7, 7, 12)] {
        analyze_hardness(rows, cols, mines, tries);
    }
}
