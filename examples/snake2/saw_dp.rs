// Frontier-state DP for counting self-avoiding walks on a W×W grid.
//
// `count_saws(start, target_length, forbidden)` returns the number of length-
// `target_length` SAWs that start at cell `start` and avoid all cells set in
// the `forbidden` bitmask. Cells are indexed row-major: `row * W + col`. The
// hardcoded 8×8 entry points keep `forbidden` as a u64; const-generic entry
// points take u128 so grids up to 11×11 = 121 cells fit.
//
// The algorithm processes grid cells in row-major order and maintains a
// "frontier state" describing the topology of partial path strands crossing
// the scan line (a non-crossing matching of column endpoints, plus an
// in-flight horizontal). State is canonicalized so equivalent matchings
// collide; that bounds the live state space to ~Motzkin(W) × small.
//
// Used by the ignored bit-budget enumeration tests (`enumerate_full_space_*`
// in `saw_rank.rs`); not on the snake2 game's runtime path — startup uses
// the precomputed `saw_tables` constants.

#![allow(dead_code)]

use rustc_hash::FxHashMap;

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
enum Slot {
    Empty,
    Free,
    Arc(u8),
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Frontier<const W: usize> {
    slots: [Slot; W],
    h: Slot,
    closed: u8,
}

impl<const W: usize> Frontier<W> {
    fn empty() -> Self {
        Frontier {
            slots: [Slot::Empty; W],
            h: Slot::Empty,
            closed: 0,
        }
    }

    /// One past the largest existing `Slot::Arc(id)` in the frontier. After
    /// canonicalization, ids run 0..k for some k ≤ W/2, so this is a tight
    /// upper bound on what we need to pass into `transition` as the next
    /// fresh id. Used to keep arc-id allocation local per transition rather
    /// than monotonically growing across the whole DP run.
    fn next_free_arc_id(&self) -> u8 {
        let mut max: i16 = -1;
        for s in &self.slots {
            if let Slot::Arc(id) = s {
                max = max.max(*id as i16);
            }
        }
        if let Slot::Arc(id) = self.h {
            max = max.max(id as i16);
        }
        (max + 1) as u8
    }

    fn open_count(&self) -> usize {
        let from_slots = self
            .slots
            .iter()
            .filter(|s| !matches!(s, Slot::Empty))
            .count();
        let from_h = !matches!(self.h, Slot::Empty) as usize;
        from_slots + from_h
    }
}

fn transition<const W: usize>(
    state: &Frontier<W>,
    c: usize,
    is_start: bool,
    is_forbidden: bool,
    right: bool,
    down: bool,
    next_arc_id: &mut u8,
) -> Option<Frontier<W>> {
    let left = !matches!(state.h, Slot::Empty);
    let up = !matches!(state.slots[c], Slot::Empty);
    let deg = (left as usize) + (up as usize) + (right as usize) + (down as usize);

    if is_forbidden {
        if deg != 0 {
            return None;
        }
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

    if state.closed >= 1 && deg > 0 {
        return None;
    }

    let mut new_state = state.clone();

    match deg {
        0 => {
            new_state.h = Slot::Empty;
            new_state.slots[c] = Slot::Empty;
            Some(new_state)
        }
        1 => match (left, up, right, down) {
            (true, false, false, false) => {
                let closed_strand = close_strand_endpoint(&mut new_state, state.h, None);
                new_state.h = Slot::Empty;
                new_state.slots[c] = Slot::Empty;
                if closed_strand {
                    new_state.closed = new_state.closed.saturating_add(1);
                }
                Some(new_state)
            }
            (false, true, false, false) => {
                let closed_strand = close_strand_endpoint(&mut new_state, state.slots[c], Some(c));
                new_state.h = Slot::Empty;
                new_state.slots[c] = Slot::Empty;
                if closed_strand {
                    new_state.closed = new_state.closed.saturating_add(1);
                }
                Some(new_state)
            }
            (false, false, true, false) => {
                new_state.h = Slot::Free;
                new_state.slots[c] = Slot::Empty;
                Some(new_state)
            }
            (false, false, false, true) => {
                new_state.h = Slot::Empty;
                new_state.slots[c] = Slot::Free;
                Some(new_state)
            }
            _ => None,
        },
        2 => match (left, up, right, down) {
            (true, true, false, false) => {
                let closed_strand = merge_strands(&mut new_state, state.h, state.slots[c], c)?;
                new_state.h = Slot::Empty;
                new_state.slots[c] = Slot::Empty;
                if closed_strand {
                    new_state.closed = new_state.closed.saturating_add(1);
                }
                Some(new_state)
            }
            (true, false, true, false) => {
                new_state.h = state.h;
                new_state.slots[c] = Slot::Empty;
                Some(new_state)
            }
            (true, false, false, true) => {
                new_state.h = Slot::Empty;
                new_state.slots[c] = state.h;
                Some(new_state)
            }
            (false, true, true, false) => {
                new_state.h = state.slots[c];
                new_state.slots[c] = Slot::Empty;
                Some(new_state)
            }
            (false, true, false, true) => {
                new_state.h = Slot::Empty;
                new_state.slots[c] = state.slots[c];
                Some(new_state)
            }
            (false, false, true, true) => {
                let id = *next_arc_id;
                *next_arc_id += 1;
                new_state.h = Slot::Arc(id);
                new_state.slots[c] = Slot::Arc(id);
                Some(new_state)
            }
            _ => None,
        },
        _ => None,
    }
}

fn close_strand_endpoint<const W: usize>(
    state: &mut Frontier<W>,
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

fn merge_strands<const W: usize>(
    state: &mut Frontier<W>,
    a: Slot,
    b: Slot,
    c: usize,
) -> Option<bool> {
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
            None
        }
        (Slot::Arc(i), Slot::Arc(j)) if i == j => None,
        (Slot::Arc(i), Slot::Arc(j)) => {
            for (k, s) in state.slots.iter_mut().enumerate() {
                if k == c {
                    continue;
                }
                if matches!(s, Slot::Arc(id) if *id == j) {
                    *s = Slot::Arc(i);
                }
            }
            Some(false)
        }
        _ => None,
    }
}

// --- Packing ---
//
// Canonical Frontier + edge-count packs into a u64 key:
//   bits  0..(4*W) — slots[0..W], 4 bits each
//   next 4 bits    — h slot
//   next 4 bits    — closed counter
//   next 8 bits    — edges so far
//
// Requires 4*W + 16 ≤ 64, i.e. W ≤ 12. Slot encoding: Empty=0, Free=1,
// Arc(id)=2+id. After canonicalization, id < W/2 + 1, so the 4-bit slot
// field accommodates W ≤ 26.

#[inline(always)]
fn pack_slot(s: Slot) -> u64 {
    match s {
        Slot::Empty => 0,
        Slot::Free => 1,
        Slot::Arc(id) => 2 + id as u64,
    }
}

#[inline(always)]
fn unpack_slot(b: u64) -> Slot {
    match b & 0xf {
        0 => Slot::Empty,
        1 => Slot::Free,
        x => Slot::Arc((x - 2) as u8),
    }
}

#[inline(always)]
fn pack<const W: usize>(state: &Frontier<W>, edges: usize) -> u64 {
    let mut v = 0u64;
    for (i, s) in state.slots.iter().enumerate() {
        v |= pack_slot(*s) << (i * 4);
    }
    let h_shift = 4 * W;
    v |= pack_slot(state.h) << h_shift;
    v |= (state.closed as u64) << (h_shift + 4);
    v |= (edges as u64) << (h_shift + 8);
    v
}

#[inline(always)]
fn canonical_pack<const W: usize>(state: &Frontier<W>, edges: usize) -> u64 {
    let mut mapping: [(u8, u8); 8] = [(0, 0); 8];
    let mut mapping_len: usize = 0;
    let mut next_id: u8 = 0;
    let mut v = 0u64;

    let mut emit =
        |s: Slot, mapping: &mut [(u8, u8); 8], mapping_len: &mut usize, next_id: &mut u8| -> u64 {
            match s {
                Slot::Empty => 0,
                Slot::Free => 1,
                Slot::Arc(id) => {
                    for &(orig, new) in &mapping[..*mapping_len] {
                        if orig == id {
                            return 2 + new as u64;
                        }
                    }
                    let new = *next_id;
                    mapping[*mapping_len] = (id, new);
                    *mapping_len += 1;
                    *next_id += 1;
                    2 + new as u64
                }
            }
        };

    for (i, s) in state.slots.iter().enumerate() {
        v |= emit(*s, &mut mapping, &mut mapping_len, &mut next_id) << (i * 4);
    }
    let h_shift = 4 * W;
    v |= emit(state.h, &mut mapping, &mut mapping_len, &mut next_id) << h_shift;
    v |= (state.closed as u64) << (h_shift + 4);
    v |= (edges as u64) << (h_shift + 8);
    v
}

#[inline(always)]
fn unpack<const W: usize>(v: u64) -> (Frontier<W>, usize) {
    let mut slots = [Slot::Empty; W];
    for i in 0..W {
        slots[i] = unpack_slot(v >> (i * 4));
    }
    let h_shift = 4 * W;
    let h = unpack_slot(v >> h_shift);
    let closed = ((v >> (h_shift + 4)) & 0xf) as u8;
    let edges = ((v >> (h_shift + 8)) & 0xff) as usize;
    (Frontier { slots, h, closed }, edges)
}

/// Count length-`target_length` SAWs starting at `start_cell` on a W×H grid
/// (W columns, H rows; cells indexed row-major: `row * W + col`), with cells
/// in `forbidden` excluded from the path.
pub fn count_saws_for<const W: usize, const H: usize>(
    start_cell: u8,
    target_length: usize,
    forbidden: u128,
) -> u64 {
    const_assert_width::<W>();
    if target_length == 0 {
        return if (forbidden >> start_cell) & 1 == 0 {
            1
        } else {
            0
        };
    }

    let start_r = (start_cell as usize) / W;
    let start_c = (start_cell as usize) % W;

    let mut dp: FxHashMap<u64, u64> = FxHashMap::default();
    dp.insert(pack::<W>(&Frontier::<W>::empty(), 0), 1);

    for r in 0..H {
        for c in 0..W {
            let is_start = r == start_r && c == start_c;
            let is_forbidden = (forbidden >> (r * W + c)) & 1 != 0;
            let mut next: FxHashMap<u64, u64> = FxHashMap::default();

            let right_ok = c < W - 1;
            let down_ok = r < H - 1;

            for (&key, &count) in &dp {
                let (state, edges) = unpack::<W>(key);
                // Arc ids in `state` are canonical (0..k); start fresh ids
                // just past the highest existing one. Keeps the counter small
                // even at full Hamiltonian lengths.
                let base_arc = state.next_free_arc_id();
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
                        let mut local_arc = base_arc;
                        if let Some(new_state) = transition::<W>(
                            &state,
                            c,
                            is_start,
                            is_forbidden,
                            right,
                            down,
                            &mut local_arc,
                        ) {
                            let new_key = canonical_pack::<W>(&new_state, new_edges);
                            *next.entry(new_key).or_insert(0) += count;
                        }
                    }
                }
            }

            dp = next;
        }
    }

    let mut total = 0u64;
    for (&key, &count) in &dp {
        let (state, edges) = unpack::<W>(key);
        if edges != target_length {
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

/// Counts of length-`l` SAWs from `start_cell` on a W×H grid for every l
/// in 0..=max_length. One frontier DP pass yields all lengths; this is
/// ~max_length times cheaper than calling `count_saws_for` per length.
pub fn count_saws_for_all_lengths<const W: usize, const H: usize>(
    start_cell: u8,
    max_length: usize,
    forbidden: u128,
) -> Vec<u64> {
    const_assert_width::<W>();
    let mut totals = vec![0u64; max_length + 1];
    if (forbidden >> start_cell) & 1 == 0 {
        totals[0] = 1;
    }
    if max_length == 0 {
        return totals;
    }

    let start_r = (start_cell as usize) / W;
    let start_c = (start_cell as usize) % W;

    let mut dp: FxHashMap<u64, u64> = FxHashMap::default();
    dp.insert(pack::<W>(&Frontier::<W>::empty(), 0), 1);

    for r in 0..H {
        for c in 0..W {
            let is_start = r == start_r && c == start_c;
            let is_forbidden = (forbidden >> (r * W + c)) & 1 != 0;
            let mut next: FxHashMap<u64, u64> = FxHashMap::default();

            let right_ok = c < W - 1;
            let down_ok = r < H - 1;

            for (&key, &count) in &dp {
                let (state, edges) = unpack::<W>(key);
                let base_arc = state.next_free_arc_id();
                for right in [false, true] {
                    if right && !right_ok {
                        continue;
                    }
                    for down in [false, true] {
                        if down && !down_ok {
                            continue;
                        }
                        let new_edges = edges + (right as usize) + (down as usize);
                        if new_edges > max_length {
                            continue;
                        }
                        let mut local_arc = base_arc;
                        if let Some(new_state) = transition::<W>(
                            &state,
                            c,
                            is_start,
                            is_forbidden,
                            right,
                            down,
                            &mut local_arc,
                        ) {
                            let new_key = canonical_pack::<W>(&new_state, new_edges);
                            *next.entry(new_key).or_insert(0) += count;
                        }
                    }
                }
            }

            dp = next;
        }
    }

    for (&key, &count) in &dp {
        let (state, edges) = unpack::<W>(key);
        if state.open_count() != 0 {
            continue;
        }
        if state.closed != 1 {
            continue;
        }
        if edges <= max_length {
            totals[edges] += count;
        }
    }
    totals
}

const fn const_assert_width<const W: usize>() {
    assert!(W > 0 && W <= 12, "saw_dp requires 1 ≤ W ≤ 12");
}

// --- 8×8 wrappers (existing API, u64 forbidden) ---

pub fn count_saws(start_cell: u8, target_length: usize, forbidden: u64) -> u64 {
    count_saws_for::<8, 8>(start_cell, target_length, forbidden as u128)
}

pub fn count_saws_all_lengths(start_cell: u8, max_length: usize, forbidden: u64) -> Vec<u64> {
    count_saws_for_all_lengths::<8, 8>(start_cell, max_length, forbidden as u128)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batched_matches_single_length_8x8() {
        for start in [0u8, 7, 27, 56, 63] {
            let batched = count_saws_all_lengths(start, 12, 0);
            for l in 0..=12 {
                let single = count_saws(start, l, 0);
                assert_eq!(batched[l], single, "mismatch at start={start} L={l}");
            }
        }
    }

    #[test]
    fn matches_oeis_on_9x9_center() {
        // OEIS A001411 (SAWs on Z² from origin) — the center of a 9×9
        // grid has boundary distance 4, so lengths 0..=4 match.
        const OEIS: &[u64] = &[1, 4, 12, 36, 100];
        let center: u8 = 4 * 9 + 4;
        for (l, &expected) in OEIS.iter().enumerate() {
            let got = count_saws_for::<9, 9>(center, l, 0);
            assert_eq!(got, expected, "9×9 center, L={l}");
        }
    }

    #[test]
    fn rectangular_8x9_matches_square_at_boundary_safe_lengths() {
        // OEIS A001411 holds at any cell whose boundary distance ≥ L.
        // In an 8-wide × 9-tall grid, the cell at (row=4, col=3) is 3 from
        // every edge — so lengths 0..=3 match A001411.
        const OEIS: &[u64] = &[1, 4, 12, 36];
        let cell: u8 = 4 * 8 + 3;
        for (l, &expected) in OEIS.iter().enumerate() {
            let got = count_saws_for::<8, 9>(cell, l, 0);
            assert_eq!(got, expected, "8×9 interior cell, L={l}");
        }
    }
}
