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

/// Frontier state across all `GRID_W` columns plus the horizontal "in-flight"
/// edge between the previously-processed cell and the next one in the same row.
///
/// `h` is the active horizontal: when we processed cell (r, c-1), we may have
/// decided to extend the path rightward — that commits a path edge into (r, c).
/// `h` records the open end of that edge until (r, c) consumes it.
///
/// `slots[j]` is the active vertical at column j for the next row to be
/// touched at that column. Between rows it represents downward edges from the
/// row just finished; mid-row it's a mix (see comment in the DP loop).
#[derive(Clone, PartialEq, Eq, Hash)]
struct Frontier {
    slots: [Slot; GRID_W],
    h: Slot,
    /// Number of fully-closed path components built so far. For a SAW we
    /// require exactly 1 at the end. During DP this gets incremented when
    /// two `Free` ends merge at a cell (closing a strand with 2 placed
    /// endpoints); any cell that would *start* a new strand after one has
    /// already closed is rejected by the transition.
    closed: u8,
}

impl Frontier {
    fn empty() -> Self {
        Frontier {
            slots: [Slot::Empty; GRID_W],
            h: Slot::Empty,
            closed: 0,
        }
    }

    /// Total number of free + arc endpoints. Useful as a sanity check.
    #[allow(dead_code)]
    fn open_count(&self) -> usize {
        let from_slots = self
            .slots
            .iter()
            .filter(|s| !matches!(s, Slot::Empty))
            .count();
        let from_h = !matches!(self.h, Slot::Empty) as usize;
        from_slots + from_h
    }

    /// Relabel `Arc` ids so they appear in left-to-right first-seen order
    /// across `slots[0..W]` then `h`. Two states differing only by id naming
    /// canonicalize to the same value and therefore collide in the DP table.
    #[allow(dead_code)]
    fn canonical(&self) -> Self {
        let mut next_id: u8 = 0;
        // Arc ids can grow across the DP without bound (they only get
        // small after canonicalization). Use a HashMap-style sparse mapping
        // via a Vec to be safe for any incoming id.
        let mut mapping: Vec<(u8, u8)> = Vec::new();
        let mut relabel = |slot: Slot| -> Slot {
            match slot {
                Slot::Arc(id) => {
                    if let Some(&(_, new)) = mapping.iter().find(|&&(orig, _)| orig == id) {
                        Slot::Arc(new)
                    } else {
                        let new = next_id;
                        mapping.push((id, new));
                        next_id += 1;
                        Slot::Arc(new)
                    }
                }
                other => other,
            }
        };
        let mut new_slots = [Slot::Empty; GRID_W];
        for (i, s) in self.slots.iter().enumerate() {
            new_slots[i] = relabel(*s);
        }
        let new_h = relabel(self.h);
        Frontier {
            slots: new_slots,
            h: new_h,
            closed: self.closed,
        }
    }
}

impl fmt::Debug for Frontier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[")?;
        for s in self.slots.iter() {
            write!(f, "{s:?}")?;
        }
        write!(f, "|h={:?}]", self.h)
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
    fn count(&self, start_cell: u8, target_length: usize) -> u64 {
        bitwise_games::saw_dp::count_saws(start_cell, target_length, 0)
    }

    /// Same as `count`, but with `visited` cells forbidden (used during decode).
    #[allow(dead_code)]
    fn count_avoiding(&self, current: u8, visited: u64, remaining: usize) -> u64 {
        bitwise_games::saw_dp::count_saws(current, remaining, visited & !(1u64 << current))
    }
}

// --- Frontier DP for SAW counting ---
//
// Process cells in row-major order. For each cell at (r, c) we decide the
// status of its "outgoing" edges (right into (r, c+1), down into (r+1, c)).
// The cell's "incoming" edges (left from h, up from slots[c]) were decided
// when the previous cells in the same row / row above were processed.
//
// Cell degree d = left + up + right + down. Constraint: d ∈ {0, 1, 2} (no
// path vertex has 3+ incident edges in a SAW). For the start cell, d = 1
// (it's a path endpoint). For forbidden cells, d = 0.
//
// We track total path edges (= SAW length). The final answer is the sum
// over states with edges == target_length and a valid completion state
// (all slots Empty, h Empty, exactly one closed path component built).

/// Apply transition for processing cell at column `c`. Returns `None` for
/// invalid combinations, or `Some(new_state)` for the resulting frontier.
///
/// `next_arc_id` is a counter passed by reference; if a new arc is created
/// here, it's incremented. Final state is canonicalized by the caller.
fn transition(
    state: &Frontier,
    c: usize,
    is_start: bool,
    is_forbidden: bool,
    right: bool,
    down: bool,
    next_arc_id: &mut u8,
) -> Option<Frontier> {
    let left = !matches!(state.h, Slot::Empty);
    let up = !matches!(state.slots[c], Slot::Empty);
    let deg = (left as usize) + (up as usize) + (right as usize) + (down as usize);

    if is_forbidden {
        if deg != 0 {
            return None;
        }
        // Cell skipped; h and slot[c] both Empty as required.
        let mut new_state = state.clone();
        new_state.h = Slot::Empty;
        new_state.slots[c] = Slot::Empty;
        return Some(new_state);
    }

    if is_start {
        if deg != 1 {
            return None;
        }
    } else if deg > 2 {
        return None;
    }

    // Closed-path lockout: once any strand has fully closed, additional path
    // edges would create a second component. Reject deg > 0 in that case.
    if state.closed >= 1 && deg > 0 {
        return None;
    }

    let mut new_state = state.clone();

    match deg {
        0 => {
            // Cell not on path. left and up must have been Empty (else we'd
            // be dropping an open strand).
            new_state.h = Slot::Empty;
            new_state.slots[c] = Slot::Empty;
            Some(new_state)
        }
        1 => {
            // Cell is an endpoint of the path: exactly one incident edge.
            match (left, up, right, down) {
                (true, false, false, false) => {
                    // Left edge only: cell terminates the h strand.
                    let closed_strand = close_strand_endpoint(&mut new_state, state.h, None);
                    new_state.h = Slot::Empty;
                    new_state.slots[c] = Slot::Empty;
                    if closed_strand {
                        new_state.closed = new_state.closed.saturating_add(1);
                    }
                    Some(new_state)
                }
                (false, true, false, false) => {
                    // Up edge only: cell terminates the slot[c] strand.
                    let closed_strand =
                        close_strand_endpoint(&mut new_state, state.slots[c], Some(c));
                    new_state.h = Slot::Empty;
                    new_state.slots[c] = Slot::Empty;
                    if closed_strand {
                        new_state.closed = new_state.closed.saturating_add(1);
                    }
                    Some(new_state)
                }
                (false, false, true, false) => {
                    // Right edge only: cell is endpoint of new strand (or
                    // the start cell laying its first edge). Outgoing edge
                    // is a Free (this strand has 1 endpoint = this cell,
                    // and 1 open end going forward).
                    new_state.h = Slot::Free;
                    new_state.slots[c] = Slot::Empty;
                    Some(new_state)
                }
                (false, false, false, true) => {
                    // Down edge only: same idea.
                    new_state.h = Slot::Empty;
                    new_state.slots[c] = Slot::Free;
                    Some(new_state)
                }
                _ => None, // impossible: deg=1 means exactly one is true
            }
        }
        2 => {
            // Cell is interior: two incident edges. The cell connects two
            // path strands (or extends one). Several subcases by which two.
            match (left, up, right, down) {
                (true, true, false, false) => {
                    // Both incoming: merge h and slot[c] strands at this cell.
                    let closed_strand = merge_strands(&mut new_state, state.h, state.slots[c], c)?;
                    new_state.h = Slot::Empty;
                    new_state.slots[c] = Slot::Empty;
                    if closed_strand {
                        new_state.closed = new_state.closed.saturating_add(1);
                    }
                    Some(new_state)
                }
                (true, false, true, false) => {
                    // Left + right: strand passes straight through. h's slot
                    // moves to the new h.
                    new_state.h = state.h;
                    new_state.slots[c] = Slot::Empty;
                    Some(new_state)
                }
                (true, false, false, true) => {
                    // Left + down: strand turns. h's slot moves to slot[c].
                    new_state.h = Slot::Empty;
                    new_state.slots[c] = state.h;
                    Some(new_state)
                }
                (false, true, true, false) => {
                    // Up + right: strand turns.
                    new_state.h = state.slots[c];
                    new_state.slots[c] = Slot::Empty;
                    Some(new_state)
                }
                (false, true, false, true) => {
                    // Up + down: strand passes straight through.
                    new_state.h = Slot::Empty;
                    new_state.slots[c] = state.slots[c];
                    Some(new_state)
                }
                (false, false, true, true) => {
                    // Right + down: cell starts a new arc whose two open
                    // ends are right and down. Assign a fresh Arc id.
                    let id = *next_arc_id;
                    *next_arc_id += 1;
                    new_state.h = Slot::Arc(id);
                    new_state.slots[c] = Slot::Arc(id);
                    Some(new_state)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Cell has degree 1 with the only incident edge being one already in the
/// state (either h or one of the slots). The strand whose open end was at
/// this edge terminates at this cell (the cell becomes the strand's second
/// endpoint).
///
/// `consumed_slot_idx` is `Some(c)` when we're consuming `slots[c]` (so the
/// partner Arc must be found *elsewhere*) or `None` when we're consuming `h`.
///
/// Returns `true` if this closes a strand (the input was `Free`), `false`
/// if it just shifts an Arc's other end to `Free`.
fn close_strand_endpoint(
    state: &mut Frontier,
    slot: Slot,
    consumed_slot_idx: Option<usize>,
) -> bool {
    match slot {
        Slot::Free => true,
        Slot::Arc(id) => {
            for (i, s) in state.slots.iter_mut().enumerate() {
                if Some(i) == consumed_slot_idx {
                    continue;
                }
                if matches!(s, Slot::Arc(j) if *j == id) {
                    *s = Slot::Free;
                    return false;
                }
            }
            if consumed_slot_idx.is_some() && matches!(state.h, Slot::Arc(j) if j == id) {
                state.h = Slot::Free;
            }
            false
        }
        Slot::Empty => false,
    }
}

/// Merge two strands at a cell where both incoming edges (h and slot[c])
/// terminate. Returns Some(closed_strand) where `closed_strand` is true iff
/// the merge closes a path (two Frees meeting). Returns None for cycles.
///
/// `c` is the column being consumed (along with `h`). When searching for
/// arc partners, we must skip both `h` and `slots[c]` — the caller will set
/// those to `Empty` regardless of any modification here.
fn merge_strands(state: &mut Frontier, a: Slot, b: Slot, c: usize) -> Option<bool> {
    match (a, b) {
        (Slot::Free, Slot::Free) => Some(true),
        (Slot::Free, Slot::Arc(id)) | (Slot::Arc(id), Slot::Free) => {
            for (i, s) in state.slots.iter_mut().enumerate() {
                if i == c {
                    continue;
                }
                if matches!(s, Slot::Arc(j) if *j == id) {
                    *s = Slot::Free;
                    return Some(false);
                }
            }
            // `h` is also being consumed (set to Empty after) so don't search it.
            None
        }
        (Slot::Arc(i), Slot::Arc(j)) if i == j => None, // cycle
        (Slot::Arc(i), Slot::Arc(j)) => {
            for (k, s) in state.slots.iter_mut().enumerate() {
                if k == c {
                    continue;
                }
                if matches!(s, Slot::Arc(id) if *id == j) {
                    *s = Slot::Arc(i);
                }
            }
            // `h` is the input `a` — we don't relabel it (it'll be cleared).
            Some(false)
        }
        _ => None,
    }
}

/// Count length-`target_length` SAWs starting from `start_cell` on the 8×8
/// grid, with cells in `forbidden` excluded from the path.
fn frontier_dp_count(start_cell: u8, target_length: usize, forbidden: u64) -> u64 {
    // Length 0 special case: just the start cell, no path edges. Always 1
    // (as long as the start isn't forbidden).
    if target_length == 0 {
        return if (forbidden >> start_cell) & 1 == 0 {
            1
        } else {
            0
        };
    }

    let start_r = (start_cell / 8) as usize;
    let start_c = (start_cell % 8) as usize;

    // dp: (canonicalized frontier, total edges) -> count
    use std::collections::HashMap;
    let mut dp: HashMap<(Frontier, usize), u64> = HashMap::new();
    dp.insert((Frontier::empty(), 0), 1);

    let mut next_arc_id: u8 = 0;

    for r in 0..GRID_H {
        for c in 0..GRID_W {
            let is_start = r == start_r && c == start_c;
            let is_forbidden = (forbidden >> (r * 8 + c)) & 1 != 0;
            let mut next: HashMap<(Frontier, usize), u64> = HashMap::new();

            let right_ok = c < GRID_W - 1;
            let down_ok = r < GRID_H - 1;

            for ((state, edges), &count) in &dp {
                for right in [false, true] {
                    if right && !right_ok {
                        continue;
                    }
                    for down in [false, true] {
                        if down && !down_ok {
                            continue;
                        }
                        let new_edges = edges + (right as usize) + (down as usize);
                        if new_edges > target_length {
                            continue;
                        }
                        let mut local_arc = next_arc_id;
                        if let Some(new_state) = transition(
                            state,
                            c,
                            is_start,
                            is_forbidden,
                            right,
                            down,
                            &mut local_arc,
                        ) {
                            let canonical = new_state.canonical();
                            *next.entry((canonical, new_edges)).or_insert(0) += count;
                            if local_arc > next_arc_id {
                                next_arc_id = local_arc;
                            }
                        }
                    }
                }
            }

            dp = next;
        }
    }

    // Final aggregation: states with edges == target_length, no open ends,
    // and exactly one closed path component (= a SAW).
    let mut total = 0u64;
    for ((state, edges), count) in &dp {
        if *edges != target_length {
            continue;
        }
        if state.open_count() != 0 {
            continue;
        }
        if state.closed != 1 {
            continue;
        }
        total += count;
    }
    total
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
        "debug" => {
            // debug <start_r> <start_c> <length>
            let sr: usize = args[1].parse().expect("start_r");
            let sc: usize = args[2].parse().expect("start_c");
            let l: usize = args[3].parse().expect("length");
            debug_compare(sr, sc, l);
        }
        "emit" => {
            // TODO(Phase 4): run full DP for all heads, all lengths, write
            // tables to `assets/saw_tables.bin`.
            eprintln!("emit: not yet implemented (Phase 4)");
            std::process::exit(1);
        }
        _ => {
            eprintln!("Usage: build_saw_tables [smoke|verify|validate|debug|emit]");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fr(slots: [Slot; GRID_W], h: Slot) -> Frontier {
        Frontier {
            slots,
            h,
            closed: 0,
        }
    }

    #[test]
    fn canonical_relabels_in_first_seen_order() {
        // Original ids 5, 2, 5 → canonical 0, 1, 0.
        let original = fr(
            [
                Slot::Arc(5),
                Slot::Empty,
                Slot::Arc(2),
                Slot::Empty,
                Slot::Arc(5),
                Slot::Empty,
                Slot::Empty,
                Slot::Empty,
            ],
            Slot::Arc(2),
        );
        let want = fr(
            [
                Slot::Arc(0),
                Slot::Empty,
                Slot::Arc(1),
                Slot::Empty,
                Slot::Arc(0),
                Slot::Empty,
                Slot::Empty,
                Slot::Empty,
            ],
            Slot::Arc(1),
        );
        assert_eq!(original.canonical(), want);
    }

    #[test]
    fn canonical_preserves_free_and_empty() {
        let original = fr(
            [
                Slot::Free,
                Slot::Arc(3),
                Slot::Empty,
                Slot::Arc(7),
                Slot::Free,
                Slot::Arc(3),
                Slot::Empty,
                Slot::Arc(7),
            ],
            Slot::Empty,
        );
        let canon = original.canonical();
        assert_eq!(canon.slots[0], Slot::Free);
        assert_eq!(canon.slots[4], Slot::Free);
        assert_eq!(canon.slots[2], Slot::Empty);
        assert_eq!(canon.slots[6], Slot::Empty);
        assert_eq!(canon.h, Slot::Empty);
        // 3 first → 0, 7 second → 1
        assert_eq!(canon.slots[1], Slot::Arc(0));
        assert_eq!(canon.slots[3], Slot::Arc(1));
        assert_eq!(canon.slots[5], Slot::Arc(0));
        assert_eq!(canon.slots[7], Slot::Arc(1));
    }

    #[test]
    fn canonical_idempotent() {
        let s = fr(
            [
                Slot::Arc(0),
                Slot::Arc(1),
                Slot::Free,
                Slot::Arc(0),
                Slot::Empty,
                Slot::Empty,
                Slot::Arc(1),
                Slot::Empty,
            ],
            Slot::Free,
        );
        assert_eq!(s.canonical(), s.canonical().canonical());
    }

    #[test]
    fn canonical_unifies_equivalent_states() {
        // Same structure, different ids: should canonicalize to the same value.
        let a = fr(
            [
                Slot::Arc(0),
                Slot::Empty,
                Slot::Arc(0),
                Slot::Arc(1),
                Slot::Empty,
                Slot::Arc(1),
                Slot::Empty,
                Slot::Empty,
            ],
            Slot::Empty,
        );
        let b = fr(
            [
                Slot::Arc(9),
                Slot::Empty,
                Slot::Arc(9),
                Slot::Arc(4),
                Slot::Empty,
                Slot::Arc(4),
                Slot::Empty,
                Slot::Empty,
            ],
            Slot::Empty,
        );
        assert_eq!(a.canonical(), b.canonical());
    }

    #[test]
    fn open_count_includes_h() {
        let s = fr(
            [
                Slot::Free,
                Slot::Empty,
                Slot::Arc(0),
                Slot::Arc(0),
                Slot::Empty,
                Slot::Empty,
                Slot::Empty,
                Slot::Empty,
            ],
            Slot::Free,
        );
        assert_eq!(s.open_count(), 4); // 1 free + 2 arc + 1 in h
    }
}

/// Cross-check `Dp::count` against `naive_count` for every (start, length)
/// up to some small bound. This is the target spec for Phase 3 — when the
/// transitions are implemented correctly, this should pass clean.
fn validate_dp_against_naive() {
    let dp = Dp::new();
    let max_l = 12;
    let mut total = 0usize;
    let mut failures = Vec::new();
    let mut per_l_total = vec![0usize; max_l + 1];
    let mut per_l_fail = vec![0usize; max_l + 1];

    for l in 0..=max_l {
        for r in 0..GRID_H {
            for c in 0..GRID_W {
                let start = cell(r, c);
                let got = dp.count(start, l);
                let want = naive_count(start, l);
                total += 1;
                per_l_total[l] += 1;
                if got != want {
                    failures.push((r, c, l, got, want));
                    per_l_fail[l] += 1;
                }
            }
        }
    }

    if failures.is_empty() {
        println!("validate: all {total} (start, length) cases match naive (L up to {max_l}).");
        return;
    }

    println!("validate: per-length results");
    for l in 0..=max_l {
        let pass = per_l_total[l] - per_l_fail[l];
        println!("  L={l}: {pass}/{} pass", per_l_total[l]);
    }
    eprintln!(
        "validate: {}/{} cases failed overall (Phase 3 transitions in progress).",
        failures.len(),
        total
    );
    for &(r, c, l, got, want) in failures.iter().take(8) {
        eprintln!("  start=({r},{c}) L={l}: dp={got} naive={want}");
    }
    if failures.len() > 8 {
        eprintln!("  ... ({} more)", failures.len() - 8);
    }
    std::process::exit(1);
}

// --- Debug: enumerate SAWs and run each through DP transitions ---

fn enumerate_saws(start: u8, length: usize) -> Vec<Vec<u8>> {
    fn dfs(pos: u8, visited: u64, path: &mut Vec<u8>, remaining: usize, out: &mut Vec<Vec<u8>>) {
        if remaining == 0 {
            out.push(path.clone());
            return;
        }
        let mut cand = NEIGHBOR_MASKS[pos as usize] & !visited;
        while cand != 0 {
            let next = cand.trailing_zeros() as u8;
            cand &= cand - 1;
            path.push(next);
            dfs(next, visited | (1u64 << next), path, remaining - 1, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    let mut path = vec![start];
    dfs(start, 1u64 << start, &mut path, length, &mut out);
    out
}

/// Run a specific SAW through the DP transitions. Returns Ok(()) if the DP
/// would accept this configuration, or Err with reason if rejected at some
/// cell.
fn dp_check_path(cells: &[u8], start: u8) -> Result<(), String> {
    let mut path_edges_h: [[bool; GRID_W]; GRID_H] = [[false; GRID_W]; GRID_H];
    let mut path_edges_v: [[bool; GRID_W]; GRID_H] = [[false; GRID_W]; GRID_H];
    for w in cells.windows(2) {
        let a = w[0] as usize;
        let b = w[1] as usize;
        let (ar, ac) = (a / 8, a % 8);
        let (br, bc) = (b / 8, b % 8);
        if ar == br && (ac + 1 == bc || bc + 1 == ac) {
            // horizontal edge at row ar between cols min, max
            path_edges_h[ar][ac.min(bc)] = true;
        } else if ac == bc && (ar + 1 == br || br + 1 == ar) {
            // vertical edge at col ac between rows min, max
            path_edges_v[ar.min(br)][ac] = true;
        }
    }

    let mut state = Frontier::empty();
    let mut edges = 0usize;
    let mut next_arc = 0u8;

    for r in 0..GRID_H {
        for c in 0..GRID_W {
            let is_start = r * 8 + c == start as usize;
            let right_chosen = c < GRID_W - 1 && path_edges_h[r][c];
            let down_chosen = r < GRID_H - 1 && path_edges_v[r][c];
            let mut local = next_arc;
            match transition(
                &state,
                c,
                is_start,
                false,
                right_chosen,
                down_chosen,
                &mut local,
            ) {
                Some(new_state) => {
                    state = new_state.canonical();
                    edges += (right_chosen as usize) + (down_chosen as usize);
                    next_arc = local;
                }
                None => {
                    return Err(format!(
                        "rejected at cell ({r},{c}): left={} up={} right={} down={} closed={} h={:?} slot[c]={:?}",
                        !matches!(state.h, Slot::Empty),
                        !matches!(state.slots[c], Slot::Empty),
                        right_chosen,
                        down_chosen,
                        state.closed,
                        state.h,
                        state.slots[c],
                    ));
                }
            }
        }
    }

    if edges != cells.len() - 1 {
        return Err(format!(
            "edges mismatch: {edges} vs expected {}",
            cells.len() - 1
        ));
    }
    if state.open_count() != 0 {
        return Err(format!("open_count != 0 at end: {}", state.open_count()));
    }
    if state.closed != 1 {
        return Err(format!("closed != 1 at end: {}", state.closed));
    }
    Ok(())
}

fn fmt_path(path: &[u8]) -> String {
    path.iter()
        .map(|c| format!("({},{})", c / 8, c % 8))
        .collect::<Vec<_>>()
        .join(" -> ")
}

fn debug_compare(sr: usize, sc: usize, l: usize) {
    let start = cell(sr, sc);
    let naive_paths = enumerate_saws(start, l);
    let naive_total = naive_paths.len();
    let dp_total = frontier_dp_count(start, l, 0);

    println!("naive total: {naive_total}, dp total: {dp_total}");

    let mut accepted = 0;
    let mut rejected_with_reasons: Vec<(Vec<u8>, String)> = Vec::new();

    for path in &naive_paths {
        match dp_check_path(path, start) {
            Ok(()) => accepted += 1,
            Err(reason) => rejected_with_reasons.push((path.clone(), reason)),
        }
    }

    println!("dp_check accepted: {accepted}/{naive_total}");
    if !rejected_with_reasons.is_empty() {
        println!("Rejected paths:");
        for (path, reason) in &rejected_with_reasons {
            println!("  {} :: {}", fmt_path(path), reason);
        }
    }
}
