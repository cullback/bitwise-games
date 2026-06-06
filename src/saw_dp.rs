// Frontier-state DP for counting self-avoiding walks on an 8×8 grid.
//
// `count_saws(start, target_length, forbidden)` returns the number of length-
// `target_length` SAWs that start at cell `start` and avoid all cells set in
// the `forbidden` bitmask. Cells are indexed row-major: `row * 8 + col`.
//
// The algorithm processes grid cells in row-major order and maintains a
// "frontier state" describing the topology of partial path strands crossing
// the scan line (a non-crossing matching of column endpoints, plus an
// in-flight horizontal). State is canonicalized so equivalent matchings
// collide; that bounds the live state space to ~Motzkin(8) × small.
//
// See `docs/snake2-frontier-dp.md` for design notes.

const GRID_W: usize = 8;
const GRID_H: usize = 8;

const NEIGHBOR_MASKS_COUNT: usize = GRID_W * GRID_H;
const _: () = assert!(NEIGHBOR_MASKS_COUNT == 64);

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
enum Slot {
    Empty,
    Free,
    Arc(u8),
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Frontier {
    slots: [Slot; GRID_W],
    h: Slot,
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

    fn open_count(&self) -> usize {
        let from_slots = self
            .slots
            .iter()
            .filter(|s| !matches!(s, Slot::Empty))
            .count();
        let from_h = !matches!(self.h, Slot::Empty) as usize;
        from_slots + from_h
    }

    fn canonical(&self) -> Self {
        // Max distinct arc ids in a valid frontier is bounded by the number
        // of arc pairs, which is ≤ GRID_W / 2 = 4. Use a stack array — heap
        // allocation here was hot enough to dominate the DP runtime.
        let mut next_id: u8 = 0;
        let mut mapping: [(u8, u8); 8] = [(0, 0); 8];
        let mut mapping_len = 0usize;
        let mut relabel = |slot: Slot| -> Slot {
            match slot {
                Slot::Arc(id) => {
                    for &(orig, new) in &mapping[..mapping_len] {
                        if orig == id {
                            return Slot::Arc(new);
                        }
                    }
                    let new = next_id;
                    mapping[mapping_len] = (id, new);
                    mapping_len += 1;
                    next_id += 1;
                    Slot::Arc(new)
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
//   bits  0..32  — slots[0..8], 4 bits each
//   bits 32..36  — h slot, 4 bits
//   bits 36..40  — closed counter, 4 bits (only ever 0 or 1 in practice)
//   bits 40..48  — edges so far, 8 bits (sufficient for any L up to 255)
//
// Slot encoding: Empty=0, Free=1, Arc(id)=2+id. After canonicalization,
// id ∈ 0..=3 since at most 4 arc pairs fit in 8 slots; values 2..=5.

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
fn pack(state: &Frontier, edges: usize) -> u64 {
    let mut v = 0u64;
    for (i, s) in state.slots.iter().enumerate() {
        v |= pack_slot(*s) << (i * 4);
    }
    v |= pack_slot(state.h) << 32;
    v |= (state.closed as u64) << 36;
    v |= (edges as u64) << 40;
    v
}

#[inline(always)]
fn unpack(v: u64) -> (Frontier, usize) {
    let mut slots = [Slot::Empty; GRID_W];
    for i in 0..GRID_W {
        slots[i] = unpack_slot(v >> (i * 4));
    }
    let h = unpack_slot(v >> 32);
    let closed = ((v >> 36) & 0xf) as u8;
    let edges = ((v >> 40) & 0xff) as usize;
    (Frontier { slots, h, closed }, edges)
}

/// Count length-`target_length` SAWs starting at `start_cell` on the 8×8 grid,
/// with cells in `forbidden` excluded from the path. `start_cell` should not
/// itself be set in `forbidden`.
pub fn count_saws(start_cell: u8, target_length: usize, forbidden: u64) -> u64 {
    if target_length == 0 {
        return if (forbidden >> start_cell) & 1 == 0 {
            1
        } else {
            0
        };
    }

    let start_r = (start_cell / 8) as usize;
    let start_c = (start_cell % 8) as usize;

    use rustc_hash::FxHashMap;
    // Key packs (canonical frontier, edges so far) into a single u64.
    let mut dp: FxHashMap<u64, u64> = FxHashMap::default();
    dp.insert(pack(&Frontier::empty(), 0), 1);

    let mut next_arc_id: u8 = 0;

    for r in 0..GRID_H {
        for c in 0..GRID_W {
            let is_start = r == start_r && c == start_c;
            let is_forbidden = (forbidden >> (r * 8 + c)) & 1 != 0;
            let mut next: FxHashMap<u64, u64> = FxHashMap::default();

            let right_ok = c < GRID_W - 1;
            let down_ok = r < GRID_H - 1;

            for (&key, &count) in &dp {
                let (state, edges) = unpack(key);
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
                            &state,
                            c,
                            is_start,
                            is_forbidden,
                            right,
                            down,
                            &mut local_arc,
                        ) {
                            let canonical = new_state.canonical();
                            let new_key = pack(&canonical, new_edges);
                            *next.entry(new_key).or_insert(0) += count;
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

    let mut total = 0u64;
    for (&key, &count) in &dp {
        let (state, edges) = unpack(key);
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
