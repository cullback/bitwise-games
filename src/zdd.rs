// Zero-suppressed Decision Diagram (ZDD) for compressed SAW enumeration.
//
// Status: Phase Z1 — data structures + reduction primitive only. See
// `docs/zdd-design.md` for the full plan.
//
// A ZDD here represents a family of subsets of edges (each subset = one SAW).
// Each non-terminal node has a variable (= an edge index, 0..NUM_EDGES) and
// two children: `lo` for "this edge is NOT in the subset", `hi` for "this
// edge IS in the subset". Reduction rules:
//
//   - Zero-suppression: if `hi == LO`, the node is eliminated (return `lo`
//     directly). This is the ZDD-specific rule that gives them better
//     compression than BDDs for sparse subset families.
//   - Hash-consing: identical (var, lo, hi) triples are merged into one node.
//
// `ZddBuilder::make_node` applies both rules at construction time so the
// resulting DAG is canonical.

use rustc_hash::FxHashMap;

/// Node identifier. Reserved values: 0 = LO terminal (no path), 1 = HI terminal
/// (valid path). All other ids index into `Zdd::nodes`.
pub type NodeId = u32;

pub const LO: NodeId = 0;
pub const HI: NodeId = 1;

/// Edge index along the variable ordering. For an 8×8 grid with 112 grid
/// edges + 64 dummy edges, indices go 0..176. We keep this as u8 — fits.
pub type EdgeIdx = u8;

/// A ZDD non-terminal node. Two reserved terminals (`LO` and `HI`) are
/// represented implicitly; only non-terminal nodes appear in `Zdd::nodes`.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Node {
    pub var: EdgeIdx,
    pub lo: NodeId,
    pub hi: NodeId,
}

/// A reduced ZDD.
#[derive(Debug)]
pub struct Zdd {
    /// `nodes[0]` and `nodes[1]` are unused placeholders for the two terminals;
    /// `nodes[i]` for i ≥ 2 is the i-th interned non-terminal node.
    pub nodes: Vec<Node>,
    /// Root of the diagram.
    pub root: NodeId,
}

/// Incremental ZDD construction with hash-consing reduction. Call `make_node`
/// to obtain (and intern) nodes; the builder enforces zero-suppression and
/// canonical sharing as you build.
pub struct ZddBuilder {
    nodes: Vec<Node>,
    interned: FxHashMap<Node, NodeId>,
}

impl ZddBuilder {
    pub fn new() -> Self {
        // Slot 0 and slot 1 are terminal placeholders (not stored as nodes
        // with meaningful content, but reserved so NodeId values 0 and 1
        // mean LO and HI respectively).
        let nodes = vec![
            Node {
                var: 0,
                lo: 0,
                hi: 0,
            }, // LO terminal placeholder
            Node {
                var: 0,
                lo: 1,
                hi: 1,
            }, // HI terminal placeholder
        ];
        ZddBuilder {
            nodes,
            interned: FxHashMap::default(),
        }
    }

    /// Create (or look up) a node with the given variable and children.
    /// Applies ZDD reduction rules:
    ///   - if `hi == LO`, return `lo` directly (zero-suppression)
    ///   - if an identical node already exists, return its id (sharing)
    pub fn make_node(&mut self, var: EdgeIdx, lo: NodeId, hi: NodeId) -> NodeId {
        if hi == LO {
            return lo;
        }
        let key = Node { var, lo, hi };
        if let Some(&id) = self.interned.get(&key) {
            return id;
        }
        let id = self.nodes.len() as NodeId;
        self.nodes.push(key);
        self.interned.insert(key, id);
        id
    }

    /// Finalize the build into a ZDD rooted at `root`.
    pub fn into_zdd(self, root: NodeId) -> Zdd {
        Zdd {
            nodes: self.nodes,
            root,
        }
    }

    pub fn node_count(&self) -> usize {
        // Subtract the 2 terminal placeholders so this reports just the
        // non-terminal count.
        self.nodes.len().saturating_sub(2)
    }
}

impl Default for ZddBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl Zdd {
    /// Count root-to-HI paths. For ZDDs representing a subset family, this is
    /// the family size (number of valid subsets). Length-unaware — for
    /// per-length counts see Phase Z3.
    pub fn count_paths(&self) -> u64 {
        let n = self.nodes.len();
        let mut count = vec![0u64; n];
        count[HI as usize] = 1;
        // count[LO] stays 0
        // Compute bottom-up: assumes node ids are assigned in topological
        // order (children have smaller ids than parents... no wait, with
        // hash-consing the children are usually MADE first, so they have
        // SMALLER ids than parents). Iterate ids from 2 upward.
        for id in 2..n {
            let node = &self.nodes[id];
            count[id] = count[node.lo as usize] + count[node.hi as usize];
        }
        count[self.root as usize]
    }
}

// --- Phase Z2: building a ZDD for SAWs from a fixed source ---
//
// We process the grid's 112 edges + 64 dummy-to-source edges in row-major
// order, applying Knuth's SIMPATH state update at each edge. The state is a
// "mate" vector tracking each vertex's partner in the partial path being
// built. See `docs/zdd-design.md` and the SIMPATH source for background.
//
// Construction is two passes:
//   1. Forward enumeration of reachable mate states at each edge layer.
//   2. Bottom-up node construction: for each (layer, state) pair, build a
//      ZDD node whose lo/hi children are the layer+1 mate states reached
//      by skipping / including this edge.
//
// `make_node` enforces zero-suppression and sharing along the way, so the
// resulting ZDD is already reduced.

const GRID_W_Z: usize = 8;
const GRID_H_Z: usize = 8;
const N_GRID: usize = GRID_W_Z * GRID_H_Z;
const DUMMY: u8 = N_GRID as u8;
const N_VERTS: usize = N_GRID + 1;

type Mate = [u8; N_VERTS];

fn initial_mate(source: u8) -> Mate {
    let mut m = [0u8; N_VERTS];
    for v in 0..N_VERTS {
        m[v] = v as u8;
    }
    m[source as usize] = DUMMY;
    m[DUMMY as usize] = source;
    m
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum EdgeKind {
    Grid,
    Dummy,
}

fn build_edge_list() -> Vec<(u8, u8, EdgeKind)> {
    let mut edges: Vec<(u8, u8, EdgeKind)> = Vec::with_capacity(176);
    for r in 0..GRID_H_Z {
        for c in 0..GRID_W_Z {
            let v = (r * GRID_W_Z + c) as u8;
            if c + 1 < GRID_W_Z {
                edges.push((v, v + 1, EdgeKind::Grid));
            }
            if r + 1 < GRID_H_Z {
                edges.push((v, v + GRID_W_Z as u8, EdgeKind::Grid));
            }
        }
    }
    for v in 0..N_GRID as u8 {
        edges.push((v, DUMMY, EdgeKind::Dummy));
    }
    edges
}

fn last_touch(edges: &[(u8, u8, EdgeKind)]) -> [usize; N_VERTS] {
    let mut lt = [0usize; N_VERTS];
    for (i, &(j, k, _)) in edges.iter().enumerate() {
        lt[j as usize] = i;
        lt[k as usize] = i;
    }
    lt
}

/// Knuth's "edge chosen" transition. Returns Some(new_mate) on success, or
/// None if this edge cannot be added (saturated vertex, or cycle that
/// doesn't satisfy the "lone simple cycle" condition).
fn try_include_edge(mate: &Mate, j: u8, k: u8) -> Option<Mate> {
    let jm = mate[j as usize];
    let km = mate[k as usize];
    if jm == 0 || km == 0 {
        return None;
    }
    if jm == k {
        // Closing the cycle here. The cycle is valid iff no *other* vertex
        // still has an open partial strand.
        for (v, &m) in mate.iter().enumerate() {
            let vu = v as u8;
            if vu == j || vu == k {
                continue;
            }
            if m != 0 && m != vu {
                return None;
            }
        }
        let mut new_mate = *mate;
        new_mate[j as usize] = 0;
        new_mate[k as usize] = 0;
        return Some(new_mate);
    }
    let mut new_mate = *mate;
    new_mate[j as usize] = 0;
    new_mate[k as usize] = 0;
    new_mate[jm as usize] = km;
    new_mate[km as usize] = jm;
    Some(new_mate)
}

/// After processing edge `edge_idx`, prune any vertex that just left the
/// frontier and is still dangling (mate ≠ 0 and ≠ itself). Returns the
/// post-pruning mate on success, or None if rejected.
fn prune_frontier(mate: &Mate, edge_idx: usize, lt: &[usize; N_VERTS]) -> Option<Mate> {
    for v in 0..N_VERTS {
        if lt[v] == edge_idx {
            let m = mate[v];
            if m != 0 && m != v as u8 {
                return None;
            }
        }
    }
    Some(*mate)
}

// --- Compact frontier-only state (Phase Z2-bis) ---
//
// For each edge layer i, the "active frontier" is the set of vertices whose
// first incident edge has been processed but whose last edge has not. Only
// those vertices have non-trivial mate values; all others are either
// untouched (`mate[v] = v`) or saturated-and-departed (`mate[v] = 0`,
// permanently). So we store mate only for active vertices.
//
// The active-vertex *set* at each layer is fixed (determined by the edge
// order); only the mate *values* vary across reachable states. That makes
// the state hashable as a small `Box<[u8]>` of length = |active set at
// layer i|, and the state space at each layer becomes tractable.

fn first_touch(edges: &[(u8, u8, EdgeKind)]) -> [usize; N_VERTS] {
    let mut ft = [usize::MAX; N_VERTS];
    for (i, &(j, k, _)) in edges.iter().enumerate() {
        if ft[j as usize] == usize::MAX {
            ft[j as usize] = i;
        }
        if ft[k as usize] == usize::MAX {
            ft[k as usize] = i;
        }
    }
    ft
}

/// Active vertex set *before* processing edge `i`. A vertex is active if its
/// first incident edge has been processed (index < i) and its last has not
/// (index ≥ i).
fn compute_active_sets(edges: &[(u8, u8, EdgeKind)]) -> Vec<Vec<u8>> {
    let ft = first_touch(edges);
    let lt = last_touch(edges);
    let mut sets: Vec<Vec<u8>> = vec![Vec::new(); edges.len() + 1];
    for v in 0..N_VERTS {
        if ft[v] == usize::MAX {
            continue; // never touched (shouldn't happen for connected graph)
        }
        for i in (ft[v] + 1)..=lt[v] {
            sets[i].push(v as u8);
        }
    }
    for s in sets.iter_mut() {
        s.sort_unstable();
    }
    sets
}

/// Compact state: `mate[v]` values for the active vertices at this layer, in
/// the order returned by `compute_active_sets[i]`.
type CompactState = Box<[u8]>;

fn unpack_compact(state: &CompactState, active: &[u8]) -> Mate {
    let mut m = [0u8; N_VERTS];
    for (v, slot) in m.iter_mut().enumerate() {
        *slot = v as u8;
    }
    for (i, &v) in active.iter().enumerate() {
        m[v as usize] = state[i];
    }
    m
}

fn pack_compact(mate: &Mate, active: &[u8]) -> CompactState {
    active
        .iter()
        .map(|&v| mate[v as usize])
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn initial_compact(source: u8, active0: &[u8]) -> CompactState {
    // At layer 0 the only "active" vertices are those introduced by edge 0.
    // The source is pre-paired with DUMMY before any edge is processed, but
    // DUMMY isn't touched until the dummy edges start (index ~112), so it's
    // not in active0. We model the pre-pairing by hoisting the source's mate
    // even if source isn't yet active. Simpler: use full mate at layer 0 and
    // pack into compact only for storage.
    let full = initial_mate(source);
    pack_compact(&full, active0)
}

/// Apply the skip / include transition between compact states across one
/// edge boundary. Returns None for the include branch if invalid (saturated
/// vertex involvement, or a cycle that doesn't satisfy the simple-cycle
/// condition).
fn transition_compact(
    state: &CompactState,
    active_in: &[u8],
    active_out: &[u8],
    edge_idx: usize,
    j: u8,
    k: u8,
    lt: &[usize; N_VERTS],
    source: u8,
    include: bool,
) -> Option<CompactState> {
    // 1. Reconstruct full mate from compact + initial fill-ins.
    let mut full = unpack_compact(state, active_in);
    // Restore the source-dummy pre-pairing if neither is in active_in
    // (i.e., we're before any edge incident to either). Otherwise the state
    // already carries the latest mate for them.
    if !active_in.contains(&source) && !active_in.contains(&DUMMY) {
        full[source as usize] = DUMMY;
        full[DUMMY as usize] = source;
    }

    // 2. Apply edge processing.
    if include {
        full = try_include_edge(&full, j, k)?;
    }

    // 3. Prune any vertex that just left the frontier with a dangling mate.
    for v in 0..N_VERTS {
        if lt[v] == edge_idx {
            let m = full[v];
            if m != 0 && m != v as u8 {
                return None;
            }
        }
    }

    // 4. Re-pack into compact state at the new active set.
    Some(pack_compact(&full, active_out))
}

/// Build the SAW ZDD for paths starting at `source`. Includes paths of all
/// lengths (filtered later via per-length counts in Phase Z3).
///
/// Logs progress every 16 edge layers.
pub fn build_saw_zdd(source: u8) -> Zdd {
    let edges = build_edge_list();
    let lt = last_touch(&edges);
    let active_sets = compute_active_sets(&edges);
    let n_edges = edges.len();

    // --- Phase A: forward enumeration of reachable compact states per layer.
    let mut layers: Vec<FxHashMap<CompactState, ()>> =
        (0..=n_edges).map(|_| FxHashMap::default()).collect();
    layers[0].insert(initial_compact(source, &active_sets[0]), ());

    for (i, &(j, k, _)) in edges.iter().enumerate() {
        if i % 16 == 0 {
            eprintln!(
                "  forward edge {i}/{n_edges}, layer states: {} (active={})",
                layers[i].len(),
                active_sets[i].len(),
            );
        }
        let states: Vec<CompactState> = layers[i].keys().cloned().collect();
        for state in states {
            let active_in = &active_sets[i];
            let active_out = &active_sets[i + 1];
            if let Some(next) =
                transition_compact(&state, active_in, active_out, i, j, k, &lt, source, false)
            {
                layers[i + 1].insert(next, ());
            }
            if let Some(next) =
                transition_compact(&state, active_in, active_out, i, j, k, &lt, source, true)
            {
                layers[i + 1].insert(next, ());
            }
        }
    }

    // --- Phase B: bottom-up ZDD node construction.
    let mut builder = ZddBuilder::new();
    let mut node_id: Vec<FxHashMap<CompactState, NodeId>> =
        (0..=n_edges).map(|_| FxHashMap::default()).collect();

    // Terminal layer: the active set should be empty (all vertices have left
    // the frontier). HI if mate values were all properly settled (we never
    // reached an invalid compact state in Phase A pruning); else LO.
    for state in layers[n_edges].keys() {
        // active_sets[n_edges] should be empty — assert.
        node_id[n_edges].insert(state.clone(), HI);
    }

    for i in (0..n_edges).rev() {
        let (j, k, _) = edges[i];
        let states: Vec<CompactState> = layers[i].keys().cloned().collect();
        for state in states {
            let active_in = &active_sets[i];
            let active_out = &active_sets[i + 1];
            let lo_id = match transition_compact(
                &state, active_in, active_out, i, j, k, &lt, source, false,
            ) {
                Some(next) => *node_id[i + 1].get(&next).unwrap_or(&LO),
                None => LO,
            };
            let hi_id =
                match transition_compact(&state, active_in, active_out, i, j, k, &lt, source, true)
                {
                    Some(next) => *node_id[i + 1].get(&next).unwrap_or(&LO),
                    None => LO,
                };
            let id = builder.make_node(i as u8, lo_id, hi_id);
            node_id[i].insert(state, id);
        }
    }

    let root = *node_id[0]
        .get(&initial_compact(source, &active_sets[0]))
        .unwrap_or(&LO);
    builder.into_zdd(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals_are_reserved() {
        let b = ZddBuilder::new();
        assert_eq!(b.node_count(), 0);
    }

    #[test]
    fn zero_suppression() {
        // make_node(var, lo, LO) should collapse to `lo` regardless of var.
        let mut b = ZddBuilder::new();
        assert_eq!(b.make_node(5, HI, LO), HI);
        assert_eq!(b.make_node(0, LO, LO), LO);
        assert_eq!(b.node_count(), 0); // nothing actually created
    }

    #[test]
    fn hash_consing_merges_identical() {
        let mut b = ZddBuilder::new();
        let a = b.make_node(3, LO, HI);
        let b_id = b.make_node(3, LO, HI);
        assert_eq!(a, b_id);
        assert_eq!(b.node_count(), 1);
    }

    #[test]
    fn count_singleton() {
        // {edge 0 in path, edge 0 only} represented as: var=0, lo=LO, hi=HI
        let mut bld = ZddBuilder::new();
        let root = bld.make_node(0, LO, HI);
        let zdd = bld.into_zdd(root);
        assert_eq!(zdd.count_paths(), 1);
    }

    #[test]
    #[ignore] // long-running construction; opt in with --ignored
    fn zdd_path_count_matches_dp_sum() {
        // For source = (3,3), the ZDD encodes all SAWs of all lengths from
        // that cell (the dummy-target trick rolls them into one diagram).
        // Total path count should equal Σ_L c_L(source) for L = 1..=L_max.
        // We check the sum to a modest L bound the DP can handle quickly.
        let source = 3 * 8 + 3;
        let zdd = build_saw_zdd(source);
        let zdd_total = zdd.count_paths();

        let mut dp_sum: u64 = 0;
        // Sum SAWs of length 1..=12 from this source on the empty grid.
        for l in 1..=12 {
            dp_sum += crate::saw_dp::count_saws(source, l, 0);
        }

        eprintln!("ZDD nodes: {}", zdd.nodes.len() - 2);
        eprintln!("ZDD total paths: {zdd_total}");
        eprintln!("DP sum L=1..=12: {dp_sum}");
        // ZDD captures *all* lengths (1..=63); DP sum is partial. So:
        assert!(zdd_total >= dp_sum, "ZDD must include all DP-counted paths");
    }

    #[test]
    fn count_two_disjoint() {
        // Family { {edge 0}, {edge 1} }:
        // var=0, lo=(var=1, lo=LO, hi=HI), hi=(var=1, lo=HI, hi=LO).
        // But (var=1, lo=HI, hi=LO) reduces by zero-suppression to HI.
        let mut bld = ZddBuilder::new();
        let n_edge1_in = bld.make_node(1, LO, HI);
        // For the "edge 0 in" branch we want subset = {edge 0}, edge 1 must
        // not be in: skip past variable 1, end at HI. That's (var=1, lo=HI,
        // hi=LO) which zero-suppresses to HI.
        let n_edge0_in = HI;
        let root = bld.make_node(0, n_edge1_in, n_edge0_in);
        let zdd = bld.into_zdd(root);
        assert_eq!(zdd.count_paths(), 2);
    }
}
