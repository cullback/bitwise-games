// 4-ary decision diagram over the SAW family on a W×H grid.
//
// One node per (cell-layer, frontier-state). Each node has up to 4 children
// corresponding to the cell's `(right_edge?, down_edge?)` decision —
// `(skip, right_only, down_only, both)`. Counts at each node sum the number
// of completed SAWs (closed-strand + empty-frontier) of each length L
// reachable through that subtree, so rank/unrank are O(W·H) walks.
//
// Construction mirrors `saw_dp::count_saws_for_all_lengths`: we run the
// slot-based frontier DP cell-by-cell, but instead of just summing counts
// we record the per-decision child node ids. Identical (var, children)
// tuples merge at construction time (hash-cons).

#![allow(dead_code)]

use rustc_hash::FxHashMap;

/// Node ids; 0 = LO terminal (no valid SAW in this subtree), 1 = HI
/// terminal (subtree contains exactly the empty completion).
pub type NodeId = u32;
pub const LO: NodeId = 0;
pub const HI: NodeId = 1;

/// A single node in the 4-ary DD. `var` is the cell index (row-major,
/// row*W + col) where this node's decision is made. `children[k]` is the
/// successor for decision `k`, indexed by the (right, down) bit pair:
///   k = 0 → (false, false) skip cell
///   k = 1 → (true,  false) include right edge only
///   k = 2 → (false, true ) include down  edge only
///   k = 3 → (true,  true ) include both edges
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Node {
    pub var: u8,
    pub children: [NodeId; 4],
}

/// Walk-ready ZDD with subtree counts.
pub struct Zdd {
    pub nodes: Vec<Node>,
    pub root: NodeId,
    /// counts[node_id][L] = number of SAWs of length L (in edges) reachable
    /// from this node by following any path of child-decisions to HI.
    pub counts: Vec<Vec<u64>>,
    pub max_length: usize,
}

/// Build a 4-ary DD covering SAWs of length 0..=max_length that start at
/// `start_cell` on a W×H grid. Two passes:
///
///   1. Top-down enumeration of reachable canonical frontier states per
///      cell layer (same walk `saw_dp` does, but storing the state set
///      rather than counts).
///   2. Bottom-up node allocation with hash-consing on `(var, children)` —
///      two distinct frontier states that resolve to the same 4-tuple of
///      children collapse into one ZDD node. This is the key step that
///      keeps the DAG small.
///
/// The returned ZDD has unique (var, children) nodes but counts are not yet
/// annotated — call `annotate_counts` for that.
pub fn build_reduced<const W: usize, const H: usize>(start_cell: u8, max_length: usize) -> Zdd {
    use rustc_hash::FxHashSet;

    let start_r = (start_cell as usize) / W;
    let start_c = (start_cell as usize) % W;
    let n_cells = W * H;

    // === Pass 1: forward state enumeration ===
    //
    // layers[i] = set of canonical_pack(state, edges) reachable just BEFORE
    // processing cell i. layers[n_cells] is the terminal layer.

    let mut layers: Vec<FxHashSet<u64>> = vec![FxHashSet::default(); n_cells + 1];
    layers[0].insert(pack::<W>(&Frontier::<W>::empty(), 0));

    for cell_idx in 0..n_cells {
        let r = cell_idx / W;
        let c = cell_idx % W;
        let is_start = r == start_r && c == start_c;
        let right_ok = c < W - 1;
        let down_ok = r < H - 1;

        // Avoid clone by collecting current layer keys before mutating.
        let current_keys: Vec<u64> = layers[cell_idx].iter().copied().collect();
        for key in current_keys {
            let (state, edges) = unpack::<W>(key);
            let base_arc = state.next_free_arc_id();
            for &(right, down) in &[(false, false), (true, false), (false, true), (true, true)] {
                if right && !right_ok {
                    continue;
                }
                if down && !down_ok {
                    continue;
                }
                let mut local_arc = base_arc;
                let Some(new_state) =
                    transition::<W>(&state, c, is_start, false, right, down, &mut local_arc)
                else {
                    continue;
                };
                let new_edges = edges + (right as usize) + (down as usize);
                if new_edges > max_length {
                    continue;
                }
                let new_key = canonical_pack::<W>(&new_state, new_edges);
                layers[cell_idx + 1].insert(new_key);
            }
        }
    }

    // === Pass 2: bottom-up node allocation with hash-consing ===

    let mut nodes: Vec<Node> = vec![
        Node {
            var: u8::MAX,
            children: [LO; 4],
        }, // LO
        Node {
            var: u8::MAX,
            children: [LO; 4],
        }, // HI
    ];
    let mut hash_cons: FxHashMap<(u8, [NodeId; 4]), NodeId> = FxHashMap::default();

    // node_map[cell_idx][state_key] = node id we allocated for this state at
    // this layer. We only need the *current* and *next* layer's maps at a
    // time, so we keep just two.
    let mut next_map: FxHashMap<u64, NodeId> = FxHashMap::default();

    // Terminal layer: map each reachable post-grid state to LO or HI.
    for &key in &layers[n_cells] {
        let (state, edges) = unpack::<W>(key);
        let id = if state.closed == 1 && state.open_count() == 0 && edges <= max_length {
            HI
        } else {
            LO
        };
        next_map.insert(key, id);
    }

    // Build up from the last cell back to cell 0.
    for cell_idx in (0..n_cells).rev() {
        let r = cell_idx / W;
        let c = cell_idx % W;
        let is_start = r == start_r && c == start_c;
        let var = cell_idx as u8;
        let right_ok = c < W - 1;
        let down_ok = r < H - 1;

        let mut current_map: FxHashMap<u64, NodeId> = FxHashMap::default();

        for &key in &layers[cell_idx] {
            let (state, edges) = unpack::<W>(key);
            let base_arc = state.next_free_arc_id();
            let mut children = [LO; 4];
            for (idx, &(right, down)) in
                [(false, false), (true, false), (false, true), (true, true)]
                    .iter()
                    .enumerate()
            {
                if right && !right_ok {
                    continue;
                }
                if down && !down_ok {
                    continue;
                }
                let mut local_arc = base_arc;
                let Some(new_state) =
                    transition::<W>(&state, c, is_start, false, right, down, &mut local_arc)
                else {
                    continue;
                };
                let new_edges = edges + (right as usize) + (down as usize);
                if new_edges > max_length {
                    continue;
                }
                let new_key = canonical_pack::<W>(&new_state, new_edges);
                children[idx] = *next_map.get(&new_key).expect("next layer state missing");
            }
            // Hash-cons: nodes with identical (var, children) merge.
            let id = *hash_cons.entry((var, children)).or_insert_with(|| {
                let new_id = nodes.len() as NodeId;
                nodes.push(Node { var, children });
                new_id
            });
            current_map.insert(key, id);
        }

        next_map = current_map;
    }

    let root_key = pack::<W>(&Frontier::<W>::empty(), 0);
    let root = *next_map.get(&root_key).expect("root state missing");

    Zdd {
        nodes,
        root,
        counts: Vec::new(),
        max_length,
    }
}

/// Older unreduced builder kept for comparison.
pub fn build_unreduced<const W: usize, const H: usize>(start_cell: u8, max_length: usize) -> Zdd {
    let start_r = (start_cell as usize) / W;
    let start_c = (start_cell as usize) % W;

    // Reserve LO=0, HI=1.
    let mut nodes: Vec<Node> = vec![
        Node {
            var: u8::MAX,
            children: [LO; 4],
        }, // LO placeholder
        Node {
            var: u8::MAX,
            children: [LO; 4],
        }, // HI placeholder
    ];

    // current_layer maps `canonical_pack(state, edges)` -> node id for the
    // node we'll fill in at this cell. We allocate the node up-front so we
    // have an id to point children at; we'll fill its `children` after we
    // resolve next-layer states.
    let mut current_layer: FxHashMap<u64, NodeId> = FxHashMap::default();
    let initial = pack::<W>(&Frontier::<W>::empty(), 0);
    let root = alloc_node(&mut nodes, 0);
    current_layer.insert(initial, root);

    for r in 0..H {
        for c in 0..W {
            let is_start = r == start_r && c == start_c;
            let var = (r * W + c) as u8;
            let right_ok = c < W - 1;
            let down_ok = r < H - 1;

            let mut next_layer: FxHashMap<u64, NodeId> = FxHashMap::default();

            // For each node at the current layer, compute its 4 children.
            // We have to collect into a Vec because we mutate nodes below.
            let entries: Vec<(u64, NodeId)> = current_layer.iter().map(|(&k, &v)| (k, v)).collect();
            for (key, node_id) in entries {
                let (state, edges) = unpack::<W>(key);
                let base_arc = state.next_free_arc_id();
                let mut children = [LO; 4];

                for (idx, &(right, down)) in
                    [(false, false), (true, false), (false, true), (true, true)]
                        .iter()
                        .enumerate()
                {
                    if right && !right_ok {
                        continue;
                    }
                    if down && !down_ok {
                        continue;
                    }
                    let mut local_arc = base_arc;
                    let Some(new_state) =
                        transition::<W>(&state, c, is_start, false, right, down, &mut local_arc)
                    else {
                        continue;
                    };
                    let new_edges = edges + (right as usize) + (down as usize);
                    if new_edges > max_length {
                        continue;
                    }
                    let new_key = canonical_pack::<W>(&new_state, new_edges);

                    let is_last_cell = r == H - 1 && c == W - 1;
                    let child_id = if is_last_cell {
                        // Terminal layer: HI if the new state is a completed SAW.
                        let (final_state, final_edges) = unpack::<W>(new_key);
                        if final_state.closed == 1
                            && final_state.open_count() == 0
                            && final_edges <= max_length
                        {
                            HI
                        } else {
                            LO
                        }
                    } else {
                        *next_layer
                            .entry(new_key)
                            .or_insert_with(|| alloc_node(&mut nodes, var.wrapping_add(1)))
                    };
                    children[idx] = child_id;
                }

                nodes[node_id as usize] = Node { var, children };
            }

            current_layer = next_layer;
        }
    }

    Zdd {
        nodes,
        root,
        counts: Vec::new(),
        max_length,
    }
}

fn alloc_node(nodes: &mut Vec<Node>, var: u8) -> NodeId {
    let id = nodes.len() as NodeId;
    nodes.push(Node {
        var,
        children: [LO; 4],
    });
    id
}

// =============================================================================
// Frontier state + transition — direct copy of `saw_dp`'s logic, kept here so
// the ZDD builder can call into it cleanly.
// =============================================================================

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
        if deg > 1 {
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
            if is_start {
                // Standalone start: closed strand of length 0.
                new_state.closed = new_state.closed.saturating_add(1);
            }
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

// Frontier packing/unpacking. Same encoding as saw_dp.rs.

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

    let emit =
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

// =============================================================================
// Count annotation: bottom-up DP over the DAG.
// =============================================================================

impl Zdd {
    /// For every node, compute `counts[node][L]` = number of completed SAWs
    /// (root-to-HI walks) of length L reachable from this node. After this
    /// returns, `self.counts[self.root as usize]` gives the per-length
    /// totals — should match `saw_dp::count_saws_for(start, L, 0)`.
    pub fn annotate_counts(&mut self) {
        let n = self.nodes.len();
        let l_max = self.max_length;
        let mut counts: Vec<Vec<u64>> = vec![vec![0u64; l_max + 1]; n];
        // HI terminal contributes 1 to L=0 (a "completed walk of length 0
        // remaining"). LO contributes nothing.
        counts[HI as usize][0] = 1;

        // Process in reverse var order. We don't actually need a topological
        // sort because edges go from lower `var` to higher `var` (or
        // terminals), but to be safe we iterate by var descending.
        // Build per-var node lists.
        let mut by_var: FxHashMap<u8, Vec<NodeId>> = FxHashMap::default();
        for (id, node) in self.nodes.iter().enumerate().skip(2) {
            by_var.entry(node.var).or_default().push(id as NodeId);
        }
        let mut vars: Vec<u8> = by_var.keys().copied().collect();
        vars.sort_unstable_by(|a, b| b.cmp(a));

        for var in vars {
            let ids = by_var.remove(&var).unwrap();
            for id in ids {
                let node = self.nodes[id as usize];
                // For each child decision, contributing edges = popcount of (right, down):
                //   k=0 (skip)        -> 0 edges
                //   k=1 (right_only)  -> 1 edge
                //   k=2 (down_only)   -> 1 edge
                //   k=3 (both)        -> 2 edges
                let extra_edges: [usize; 4] = [0, 1, 1, 2];
                let mut accum = vec![0u64; l_max + 1];
                for k in 0..4 {
                    let child = node.children[k];
                    if child == LO {
                        continue;
                    }
                    let child_counts = &counts[child as usize];
                    let ext = extra_edges[k];
                    for cl in 0..=l_max {
                        if cl + ext > l_max {
                            break;
                        }
                        accum[cl + ext] = accum[cl + ext].saturating_add(child_counts[cl]);
                    }
                }
                counts[id as usize] = accum;
            }
        }

        self.counts = counts;
    }

    /// Total number of SAWs of length L (from the start cell this ZDD was
    /// built for).
    pub fn count_at_length(&self, length: usize) -> u64 {
        self.counts[self.root as usize][length]
    }

    /// Given a path of length L starting at the cell this ZDD was built for,
    /// compute its rank within the (length=L, start=this_start) bucket.
    ///
    /// Ranking is "ZDD walk order": at each cell we descend into child k,
    /// summing the subtree counts of children k' < k that admit a length-L
    /// completion. k = 0 (skip) < 1 (right) < 2 (down) < 3 (both).
    pub fn rank_of_path<const W: usize, const H: usize>(&self, path: &[u8]) -> u64 {
        let length = path.len() - 1;
        // Convert path → per-cell decision sequence (k value per cell).
        let decisions = path_to_cell_decisions::<W, H>(path);

        let mut node = self.root;
        let mut rank = 0u64;
        let mut remaining = length;
        let extra_edges: [usize; 4] = [0, 1, 1, 2];

        for &k in &decisions {
            let n = self.nodes[node as usize];
            // Sum counts of earlier-ordered children that can still complete
            // a length-`remaining` SAW.
            for k_prev in 0..k as usize {
                let child = n.children[k_prev];
                if child == LO {
                    continue;
                }
                let cl = remaining.checked_sub(extra_edges[k_prev]);
                if let Some(cl) = cl {
                    rank = rank.saturating_add(self.counts[child as usize][cl]);
                }
            }
            // Descend into the chosen child.
            let chosen = n.children[k as usize];
            remaining -= extra_edges[k as usize];
            node = chosen;
        }
        rank
    }

    /// Inverse of `rank_of_path`.
    pub fn path_at_rank<const W: usize, const H: usize>(
        &self,
        mut rank: u64,
        length: usize,
        start_cell: u8,
    ) -> Vec<u8> {
        let n_cells = W * H;
        let mut decisions: Vec<u8> = Vec::with_capacity(n_cells);
        let mut node = self.root;
        let mut remaining = length;
        let extra_edges: [usize; 4] = [0, 1, 1, 2];

        for _ in 0..n_cells {
            let n = self.nodes[node as usize];
            let mut chosen = 0u8;
            for k in 0..4 {
                let child = n.children[k];
                let cl = remaining.checked_sub(extra_edges[k]);
                let c = match cl {
                    Some(cl) if child != LO => self.counts[child as usize][cl],
                    _ => 0,
                };
                if rank < c {
                    chosen = k as u8;
                    break;
                }
                rank -= c;
            }
            decisions.push(chosen);
            remaining -= extra_edges[chosen as usize];
            node = n.children[chosen as usize];
        }
        cell_decisions_to_path::<W, H>(&decisions, start_cell, length)
    }
}

/// For each cell in row-major order, return the k value (0..4) corresponding
/// to whether the path includes that cell's right and/or down edges.
fn path_to_cell_decisions<const W: usize, const H: usize>(path: &[u8]) -> Vec<u8> {
    let n_cells = W * H;
    // Build set of path edges as (cell, "right"|"down") tuples.
    let mut right_edges = vec![false; n_cells];
    let mut down_edges = vec![false; n_cells];
    for i in 0..(path.len() - 1) {
        let a = path[i] as usize;
        let b = path[i + 1] as usize;
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        if hi == lo + 1 {
            // Horizontal edge from `lo` to `lo + 1`. Owned by cell `lo`.
            right_edges[lo] = true;
        } else if hi == lo + W {
            // Vertical edge from `lo` to `lo + W`. Owned by cell `lo`.
            down_edges[lo] = true;
        } else {
            panic!("path step {}→{} is not a grid adjacency", a, b);
        }
    }
    (0..n_cells)
        .map(|c| (right_edges[c] as u8) | ((down_edges[c] as u8) << 1))
        .collect()
}

fn cell_decisions_to_path<const W: usize, const H: usize>(
    decisions: &[u8],
    start_cell: u8,
    length: usize,
) -> Vec<u8> {
    // Reconstruct the path by walking from start_cell, following edges chosen
    // by the decisions. Each cell knows which incident edges (right/down) are
    // in the path; we use this to walk the connected subgraph.
    let n_cells = W * H;
    let mut adj: Vec<u8> = vec![0; n_cells]; // bitmask of 4 directions: 1=up,2=left,4=right,8=down
    for cell in 0..n_cells {
        let k = decisions[cell];
        let r = cell / W;
        let c = cell % W;
        if k & 1 != 0 && c + 1 < W {
            // Right edge from `cell` → cell+1.
            adj[cell] |= 4;
            adj[cell + 1] |= 2;
        }
        if k & 2 != 0 && r + 1 < H {
            // Down edge from `cell` → cell+W.
            adj[cell] |= 8;
            adj[cell + W] |= 1;
        }
    }

    let mut path: Vec<u8> = Vec::with_capacity(length + 1);
    path.push(start_cell);
    let mut current = start_cell as usize;
    let mut prev: Option<usize> = None;
    for _ in 0..length {
        let mask = adj[current];
        // Pick the neighbor that's not `prev`.
        let next = if mask & 1 != 0 && Some(current - W) != prev {
            current - W
        } else if mask & 2 != 0 && Some(current - 1) != prev {
            current - 1
        } else if mask & 4 != 0 && Some(current + 1) != prev {
            current + 1
        } else if mask & 8 != 0 && Some(current + W) != prev {
            current + W
        } else {
            panic!("path reconstruction stuck at cell {current}");
        };
        path.push(next as u8);
        prev = Some(current);
        current = next;
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-check: ZDD counts at each length must match saw_dp's counts.
    #[test]
    fn counts_match_saw_dp_4x4() {
        use crate::saw_dp;
        const W: usize = 4;
        const H: usize = 4;
        const L: usize = 15; // Hamiltonian on 4×4
        for start in 0..(W * H) as u8 {
            let mut zdd = build_unreduced::<W, H>(start, L);
            zdd.annotate_counts();
            for length in 0..=L {
                let dp = saw_dp::count_saws_for::<W, H>(start, length, 0);
                let zd = zdd.count_at_length(length);
                assert_eq!(
                    zd, dp,
                    "mismatch at start={start} L={length}: zdd={zd} saw_dp={dp}",
                );
            }
        }
    }

    #[test]
    #[ignore]
    fn node_count_scan() {
        // Reduced vs unreduced ZDD node counts. Run with `--ignored --nocapture`.
        use std::time::Instant;
        eprintln!(
            "\n{:>5} {:>10} {:>14} {:>10} {:>14} {:>10}",
            "grid", "raw_ms", "raw_nodes", "raw_MB", "red_nodes", "red_MB"
        );
        macro_rules! run {
            ($w:expr, $h:expr) => {{
                let l = ($w * $h) - 1;
                let t = Instant::now();
                let raw = build_unreduced::<$w, $h>(0, l);
                let raw_ms = t.elapsed().as_millis();
                let red = build_reduced::<$w, $h>(0, l);
                let raw_n = raw.nodes.len();
                let red_n = red.nodes.len();
                let raw_mb = raw_n * std::mem::size_of::<Node>() / 1_048_576;
                let red_mb = red_n * std::mem::size_of::<Node>() / 1_048_576;
                eprintln!(
                    "{:>5} {:>10} {:>14} {:>10} {:>14} {:>10}",
                    concat!($w, "x", $h),
                    raw_ms,
                    raw_n,
                    raw_mb,
                    red_n,
                    red_mb
                );
            }};
        }
        run!(4, 4);
        run!(5, 5);
        run!(6, 6);
        run!(7, 7);
        run!(8, 8);
        run!(9, 8);
    }

    #[test]
    fn round_trip_3x3() {
        // For every (start, length, every SAW path), encode and decode should
        // round-trip via the ZDD. We also assert ranks span [0, count) exactly.
        const W: usize = 3;
        const H: usize = 3;
        const L_MAX: usize = 8;
        for start in 0..(W * H) as u8 {
            let mut zdd = build_reduced::<W, H>(start, L_MAX);
            zdd.annotate_counts();
            for length in 0..=L_MAX {
                let count = zdd.count_at_length(length);
                let mut seen: std::collections::HashSet<u64> = Default::default();
                let mut paths: std::collections::HashSet<Vec<u8>> = Default::default();
                for r in 0..count {
                    let path = zdd.path_at_rank::<W, H>(r, length, start);
                    assert_eq!(path.len(), length + 1);
                    assert_eq!(path[0], start);
                    let r2 = zdd.rank_of_path::<W, H>(&path);
                    assert_eq!(
                        r2, r,
                        "round-trip start={start} L={length} r={r} path={path:?}"
                    );
                    seen.insert(r);
                    paths.insert(path);
                }
                assert_eq!(seen.len() as u64, count);
                assert_eq!(paths.len() as u64, count);
            }
        }
    }

    #[test]
    fn reduced_counts_match_saw_dp_4x4() {
        use crate::saw_dp;
        const W: usize = 4;
        const H: usize = 4;
        const L: usize = 15;
        for start in 0..(W * H) as u8 {
            let mut zdd = build_reduced::<W, H>(start, L);
            zdd.annotate_counts();
            for length in 0..=L {
                let dp = saw_dp::count_saws_for::<W, H>(start, length, 0);
                let zd = zdd.count_at_length(length);
                assert_eq!(zd, dp, "reduced 4x4 start={start} L={length}");
            }
        }
    }

    #[test]
    fn counts_match_saw_dp_3x3_hamiltonian() {
        use crate::saw_dp;
        const W: usize = 3;
        const H: usize = 3;
        const L: usize = 8;
        for start in 0..9 {
            let mut zdd = build_unreduced::<W, H>(start, L);
            zdd.annotate_counts();
            for length in 0..=L {
                let dp = saw_dp::count_saws_for::<W, H>(start, length, 0);
                let zd = zdd.count_at_length(length);
                assert_eq!(zd, dp, "3×3 start={start} L={length}");
            }
        }
    }
}
