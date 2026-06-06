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
#[allow(dead_code)] // Used once Phase 3 transitions land.
const GRID_H: usize = 8;
#[allow(dead_code)]
const N_CELLS: usize = GRID_W * GRID_H;

/// Row-major cell index.
fn cell(r: usize, c: usize) -> u8 {
    (r * GRID_W + c) as u8
}

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
        "validate" => {
            // TODO(Phase 3): cross-check against naive count_extensions on a
            // small grid (4×4) for lengths 1..=10. Fail if any mismatch.
            eprintln!("validate: not yet implemented (Phase 3)");
            std::process::exit(1);
        }
        "emit" => {
            // TODO(Phase 4): run full DP for all heads, all lengths, write
            // tables to `assets/saw_tables.bin`.
            eprintln!("emit: not yet implemented (Phase 4)");
            std::process::exit(1);
        }
        _ => {
            eprintln!("Usage: build_saw_tables [smoke|validate|emit]");
            std::process::exit(2);
        }
    }
}
