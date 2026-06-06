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
