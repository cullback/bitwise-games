// Build SAW count tables for snake2 via frontier-state DP.
//
// See `docs/snake2-frontier-dp.md` for the design. Status: Phase 2 (skeleton).
// Transitions are not yet implemented — the binary compiles and lays out the
// frontier-state types but the DP currently returns 0 counts.
//
// Validation strategy: each phase adds tests against the naive
// `count_extensions` from snake2 on a small grid (initially 4×4).

use std::env;
use std::fmt;

// --- Grid (parameterized so we can validate on small boards before 8×8) ---

const GRID_W: usize = 8;
const GRID_H: usize = 8;
const N_CELLS: usize = GRID_W * GRID_H;

/// Row-major cell index.
fn cell(r: usize, c: usize) -> u8 {
    (r * GRID_W + c) as u8
}

// --- Neighbor bitmasks (same layout as snake2.rs) ---

const NEIGHBOR_MASKS: [u64; N_CELLS] = {
    let mut masks = [0u64; N_CELLS];
    let mut i = 0;
    while i < N_CELLS {
        let r = (i / GRID_W) as isize;
        let c = (i % GRID_W) as isize;
        let mut mask = 0u64;
        if r > 0 {
            mask |= 1u64 << ((r as usize - 1) * GRID_W + c as usize);
        }
        if r < (GRID_H as isize) - 1 {
            mask |= 1u64 << ((r as usize + 1) * GRID_W + c as usize);
        }
        if c > 0 {
            mask |= 1u64 << (r as usize * GRID_W + (c as usize - 1));
        }
        if c < (GRID_W as isize) - 1 {
            mask |= 1u64 << (r as usize * GRID_W + (c as usize + 1));
        }
        masks[i] = mask;
        i += 1;
    }
    masks
};

// --- Oracle: naive backtracking SAW count ---
//
// The frontier DP we're building (below) will be cross-checked against this
// for small lengths. This naive impl is the same as snake2.rs's
// `count_extensions` modulo the prune optimizations (kept simple for clarity
// as a test oracle).

fn naive_count_extensions(pos: u8, visited: u64, remaining: usize) -> u64 {
    if remaining == 0 {
        return 1;
    }
    let mut total = 0u64;
    let mut cand = NEIGHBOR_MASKS[pos as usize] & !visited;
    while cand != 0 {
        let next = cand.trailing_zeros() as u8;
        cand &= cand - 1;
        total += naive_count_extensions(next, visited | (1u64 << next), remaining - 1);
    }
    total
}

/// Naive count of SAWs of length L starting from `start_cell` (no obstacles).
fn naive_count(start_cell: u8, length: usize) -> u64 {
    naive_count_extensions(start_cell, 1u64 << start_cell, length)
}

// --- OEIS A001411: SAWs on infinite Z² from origin, by step count ---
//
// https://oeis.org/A001411
//
// Used to validate our naive count for cells far enough from the boundary
// that the 8×8 grid is effectively unbounded for that length.
const OEIS_A001411: &[u64] = &[
    1, 4, 12, 36, 100, 284, 780, 2172, 5916, 16268, 44100, 120292,
];

// --- Frontier state ---
//
// The frontier sits between processed cells (above and to the left in
// row-major order) and unprocessed cells. After processing cell k, exactly
// GRID_W "slots" are active on the frontier — one per column — each
// representing the edge of the partial path emerging from the processed
// region at that column.

/// Per-column tag in the frontier state.
///
/// `Free` means an open path endpoint that is not (yet) matched to another:
/// the start cell, or the moving end of the partial path. There may be at
/// most two `Free` slots in a valid state (start + current end).
///
/// `Arc(id)` means a paired open endpoint. Two slots carrying the same id
/// connect to one another via path edges through the processed region.
/// Matchings are non-crossing.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
#[allow(dead_code)] // Free + Arc constructed once Phase 3 transitions land.
enum Slot {
    Empty,
    Free,
    Arc(u8),
}

impl fmt::Debug for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Slot::Empty => write!(f, "."),
            Slot::Free => write!(f, "F"),
            Slot::Arc(id) => write!(f, "a{id}"),
        }
    }
}

/// Frontier state across all `GRID_W` columns at the current row.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Frontier {
    slots: [Slot; GRID_W],
}

impl Frontier {
    fn empty() -> Self {
        Frontier {
            slots: [Slot::Empty; GRID_W],
        }
    }

    /// Total number of free + arc endpoints. Useful as a sanity check.
    fn open_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| !matches!(s, Slot::Empty))
            .count()
    }
}

impl fmt::Debug for Frontier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[")?;
        for s in self.slots.iter() {
            write!(f, "{s:?}")?;
        }
        write!(f, "]")
    }
}

// --- DP ---
//
// f[state][length] = number of partial paths producing the frontier state
// after processing the current cell, having used exactly `length` path edges.
//
// Transitions for processing cell at (r, c):
//   Decision A: cell is not on path. Slot at column c stays Empty.
//   Decision B: cell is on path. Connects to ≤ 2 of its 4 neighbors via path
//               edges. Update arc structure accordingly.
//
// All transitions filter to keep the state valid: at most one free endpoint
// after the start cell is consumed, non-crossing arcs, etc.

#[derive(Default)]
struct Dp {
    // (Phase 2: stubbed.)
    _placeholder: (),
}

impl Dp {
    fn new() -> Self {
        Dp::default()
    }

    /// Count length-`target_length` SAWs from `start_cell` on the empty grid.
    /// Returns 0 until the transition logic is filled in (Phase 3).
    #[allow(unused_variables)]
    fn count(&self, start_cell: u8, target_length: usize) -> u64 {
        // TODO(Phase 3): run the DP and return the count.
        0
    }

    /// Same as `count`, but with `visited` cells forbidden (used during decode).
    #[allow(unused_variables, dead_code)]
    fn count_avoiding(&self, current: u8, visited: u64, remaining: usize) -> u64 {
        // TODO(Phase 3): run the DP treating cells in `visited` as forbidden.
        0
    }
}

// --- CLI ---

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("smoke");

    match mode {
        "smoke" => {
            let dp = Dp::new();
            for &(r, c, l) in &[(0, 0, 1usize), (3, 3, 5), (3, 3, 10)] {
                let count = dp.count(cell(r, c), l);
                println!("count_saws(start=({r},{c}), len={l}) = {count}  [stub returns 0]");
            }
            println!();
            let f = Frontier::empty();
            println!("empty frontier: {f:?} (open_count={})", f.open_count());
        }
        "verify" => {
            verify_naive_oracle();
        }
        "validate" => {
            validate_dp_against_naive();
        }
        "emit" => {
            // TODO(Phase 4): run full DP for all heads, all lengths, write
            // tables to `assets/saw_tables.bin`.
            eprintln!("emit: not yet implemented (Phase 4)");
            std::process::exit(1);
        }
        _ => {
            eprintln!("Usage: build_saw_tables [smoke|verify|validate|emit]");
            std::process::exit(2);
        }
    }
}

// --- Verification: naive count against OEIS + hand-computed fixtures ---

/// Cross-check the naive count against OEIS A001411 (where the boundary
/// doesn't reach yet) and against hand-computed corner values. This sanity-
/// checks the *oracle* itself before we start trusting it for DP validation.
fn verify_naive_oracle() {
    let mut failures = 0;

    // Center-cell SAWs match A001411 while the walk hasn't reached the edge.
    // From (3,3) on 8×8, minimum distance to boundary is 3, so L ≤ 3 is safe.
    for l in 0..=3 {
        let got = naive_count(cell(3, 3), l);
        let want = OEIS_A001411[l];
        let ok = got == want;
        println!(
            "[{}] center=(3,3) L={l}: got={got} oeis_a001411={want}",
            mark(ok)
        );
        if !ok {
            failures += 1;
        }
    }
    // From (4,4), same situation (distance 3 to nearest boundary).
    for l in 0..=3 {
        let got = naive_count(cell(4, 4), l);
        let want = OEIS_A001411[l];
        let ok = got == want;
        println!(
            "[{}] center=(4,4) L={l}: got={got} oeis_a001411={want}",
            mark(ok)
        );
        if !ok {
            failures += 1;
        }
    }

    // Hand-computed corner-cell SAW counts on 8×8 from (0,0):
    //   L=0: just the cell           → 1
    //   L=1: (0,1) or (1,0)          → 2
    //   L=2: each L=1 extends to 2   → 4
    //   L=3: enumerated below        → 10
    let corner_fixtures: &[(usize, u64)] = &[(0, 1), (1, 2), (2, 4), (3, 10)];
    for &(l, want) in corner_fixtures {
        let got = naive_count(cell(0, 0), l);
        let ok = got == want;
        println!(
            "[{}] corner=(0,0) L={l}: got={got} expected={want}",
            mark(ok)
        );
        if !ok {
            failures += 1;
        }
    }

    // Edge midpoint (0,3): distance to top = 0, others ≥ 3. L=1 manually = 3
    // (down/left/right; up off-board), L=2 = 8 (verified below).
    //   L=0 → 1
    //   L=1 → 3 (neighbors (0,2), (0,4), (1,3))
    //   L=2 → for each L=1 extension, count second-step options:
    //         from (0,2): (0,1), (0,3-visited? no, head is (0,3), (0,2) is body[0]).
    //                     Wait, (0,3) is start, body[0]=(0,2). Next from (0,2):
    //                     (0,1) ok, (1,2) ok, (0,3) visited. → 2
    //         from (0,4): (0,5) ok, (1,4) ok, (0,3) visited. → 2
    //         from (1,3): (1,2) ok, (1,4) ok, (2,3) ok, (0,3) visited. → 3
    //         Total: 2+2+3 = 7
    let edge_fixtures: &[(usize, u64)] = &[(0, 1), (1, 3), (2, 7)];
    for &(l, want) in edge_fixtures {
        let got = naive_count(cell(0, 3), l);
        let ok = got == want;
        println!("[{}] edge=(0,3) L={l}: got={got} expected={want}", mark(ok));
        if !ok {
            failures += 1;
        }
    }

    println!();
    if failures == 0 {
        println!("All oracle checks passed. Naive count is trustworthy for DP cross-checks.");
    } else {
        println!("{failures} oracle check(s) FAILED.");
        std::process::exit(1);
    }
}

fn mark(ok: bool) -> &'static str {
    if ok { "OK " } else { "FAIL" }
}

/// Cross-check `Dp::count` against `naive_count` for every (start, length)
/// up to some small bound. This is the target spec for Phase 3 — when the
/// transitions are implemented correctly, this should pass clean.
fn validate_dp_against_naive() {
    let dp = Dp::new();
    let max_l = 4;
    let mut total = 0usize;
    let mut failures = Vec::new();

    for l in 0..=max_l {
        for r in 0..GRID_H {
            for c in 0..GRID_W {
                let start = cell(r, c);
                let got = dp.count(start, l);
                let want = naive_count(start, l);
                total += 1;
                if got != want {
                    failures.push((r, c, l, got, want));
                }
            }
        }
    }

    if failures.is_empty() {
        println!("validate: all {total} (start, length) cases match naive (L up to {max_l}).");
        return;
    }

    eprintln!(
        "validate: {}/{} cases failed (Phase 3 not complete).",
        failures.len(),
        total
    );
    // Show only the first few to keep output readable.
    for &(r, c, l, got, want) in failures.iter().take(5) {
        eprintln!("  start=({r},{c}) L={l}: dp={got} naive={want}");
    }
    if failures.len() > 5 {
        eprintln!("  ... ({} more)", failures.len() - 5);
    }
    std::process::exit(1);
}
