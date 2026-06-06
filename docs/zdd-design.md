# ZDD-based SAW rank/unrank for Snake2

Working notes for the BDD/ZDD approach to constant-per-step decode in snake2.
This is the "Option A" path discussed in the snake2 optimization arc — replacing
runtime frontier DP with a precomputed, reduced decision diagram annotated with
per-length subtree counts, so encode/decode is `O(edges)` lookups.

## Why ZDD

The frontier DP we have is correct and polynomial-in-L, but its per-query cost
is dominated by hashing/iteration over ~10K live states × 64 cells. At L=22 the
worst-case decode is ~1.2 s — well over the 200 ms frame budget at 5 FPS.

A ZDD compresses the full enumeration tree into a DAG by merging subtrees that
behave identically going forward. Once built and annotated with per-length
subtree counts, **rank/unrank is a constant-cost walk along 112 edges** — no
counts to compute at runtime, no states to iterate over.

The build phase is offline and may be slow; the user explicitly OK'd that.

## Pieces

### 1. Edge order

Process grid edges in row-major order, two per cell (right then down). Plus
one virtual "dummy" edge per cell connecting to a dummy vertex (Knuth's
`simpath-ham-any.ch` trick — makes "any target" work by pre-pairing source
with dummy).

Total edges per ZDD: 8·7 horizontal + 8·7 vertical + 64 dummy = **176 edges**.

### 2. ZDD node

```rust
struct Node {
    var: u8,           // edge index, 0..NUM_EDGES
    lo: NodeId,        // child when this edge is NOT in path
    hi: NodeId,        // child when this edge IS in path
}
```

Two reserved terminals: `LO = 0` ("no valid path"), `HI = 1` ("valid path").

### 3. Construction

Process edges in fixed order. At edge `e`, for each (frontier state, edges so
far) reachable from root, create two children: one for "include e", one for
"skip e". This is what `count_saws` already does internally; we just record
the DAG explicitly instead of summing counts.

Identical (var, lo, hi) tuples get merged (ZDD reduction). Nodes with
`hi = LO` (high branch leads to dead-end) collapse to their `lo` child
(zero-suppression).

### 4. Length annotation

Each ZDD node `v` carries `count[L] = number of root-to-HI paths through v
with exactly L "high" choices remaining`. Computed bottom-up after reduction:

- `HI.count[0] = 1`, `HI.count[L>0] = 0`
- `LO.count[*] = 0`
- internal: `v.count[L] = v.lo.count[L] + v.hi.count[L − 1]`

For our use, L_max ≈ 50.

### 5. Rank / unrank

**Unrank(root, length L, rank R) → edge decisions**:

```
node, edges_done = root, 0
for step in 1..=NUM_EDGES:
    if node is terminal: break
    remaining_L = L - edges_done
    cnt_lo = node.lo.count[remaining_L]
    if R < cnt_lo:
        emit 0; node = node.lo
    else:
        emit 1; R -= cnt_lo; edges_done += 1; node = node.hi
```

**Rank(root, length L, decisions) → R**:

```
node, edges_done, R = root, 0, 0
for d in decisions:
    if d == 1:
        R += node.lo.count[L - edges_done]
        edges_done += 1
    node = node.{lo,hi}
```

Both are O(NUM_EDGES) per call — no counts to compute, no states to iterate.

### 6. Per-head ZDDs

The starting cell is part of the initial frontier state (pre-paired with dummy
via Knuth's trick). We build **one ZDD per head**. By 8-fold D4 grid symmetry
we only need 10 canonical-head ZDDs; the rest are obtained by reflecting the
edge sequence.

### 7. Storage

Rough estimate per canonical-head ZDD:

- 10K nodes after reduction × (1 var byte + 2 × 4-byte child ids) = 90 KB
- count tables: 10K nodes × 50 lengths × 8 bytes = 4 MB
- per-canonical-head total: ~4 MB
- all 10 canonical heads: **~40 MB**

Bake into the snake2 binary via `include_bytes!`.

## Build pipeline

Add a new `bin/build_zdd_tables` that:

1. Runs the frontier DP we already have, recording every (state, edges) it
   produces at each edge step
2. Reduces the resulting DAG (merge identical subtrees, zero-suppress)
3. Computes per-length counts (bottom-up DP over the reduced DAG)
4. Emits a binary blob per canonical head

The `count_saws` function in the main library reads the blob lazily on first
use, then provides constant-time rank/unrank.

## Implementation phases

### Phase Z1 ✅ — Data structures (committed)

- `src/zdd.rs` skeleton with `Node`, `ZddBuilder`
- `make_node` enforces zero-suppression and hash-consing canonically
- 5 unit tests pass

### Phase Z2 ⚠ — Construction attempted, hit state-size wall

Implemented `build_saw_zdd(source)` using Knuth's mate-array representation
edge-by-edge (112 grid edges + 64 dummy = 176 layers). Forward enumeration
phase OOMs around layer 16 with already ~9K reachable mate states. Each
mate is 65 bytes (one per grid cell + dummy), so storing
N_LAYERS × N_STATES × 65 bytes is GB-scale.

**Root cause**: the mate-array representation is too verbose. Knuth's
SIMPATH packs mate values for only the _active frontier vertices_ — at most
~10 bytes per state for an 8×8 grid — using a custom queue in `mem[]`.
Without that compression, the forward enumeration doesn't fit in memory.

### Phase Z2-bis ⚠ — Frontier-only state isn't enough

Implemented active-vertex tracking via `first_touch`/`last_touch` and a
compact `Box<[u8]>` mate-only-for-active-vertices encoding. The
per-state byte count dropped from 65 → ~16 around layer 16, but **the
state count is what's killing us, not the per-state size**: at layer 16
the forward enumeration already has 9K distinct states, and the growth
trajectory extrapolates to millions by layer 50. With 176 layers each
holding 10K-1M states + memo overhead, we OOM before reaching the
dummy-edge phase.

**Root cause is canonicalization**, not storage. The slot-based DP in
`saw_dp.rs` deliberately relabels arc ids in left-to-right first-seen
order so that structurally-identical configurations with different
internal id assignments collide into one canonical state. The mate
representation uses _absolute vertex ids_, so two configurations with
the same topology but different vertex labels are distinct states and
never merge — the state space blows up.

For our problem, switching to a slot-based + canonicalized representation
on top of the edge-by-edge ZDD framework is the only realistic path.
That means refactoring our cell-based slot DP into per-edge transitions
(splitting each cell's 4-way decision into two binary edge decisions
with a "mid-cell" partial state). That's another ~200 lines of careful
work, plus the partial-state semantics need to be correct or the ZDD
encodes the wrong family.

### Phase Z2-ter — Slot-based per-edge ZDD construction (not yet started)

The right next direction. Split:

1. Define a "mid-cell" Frontier extension: same fields as `saw_dp::Frontier`
   plus a flag for "right edge of cell (r, c) decided but down not yet."
2. Two transition functions: `apply_right_edge`, `apply_down_edge`. Each
   takes a slot Frontier and produces 0/1 successor frontiers depending on
   the include/skip decision.
3. Build ZDD edge-by-edge using canonical packed-u64 keys (same packing as
   `saw_dp.rs`).
4. Verify against `saw_dp::count_saws` at every length.

### Phase Z3 — Reduction

- Implement Sieling-Wegener bottom-up reduction
- Verify reduced DAG has fewer nodes but same path count

### Phase Z3 — Length annotation

- Compute `count[L]` arrays bottom-up
- Validate against frontier DP counts at every (head, length)

### Phase Z4 — Rank/unrank

- Implement walk-based rank/unrank
- Round-trip test: encode(decode(R)) == R for many R values

### Phase Z5 — Build pipeline

- `bin/build_zdd_tables` emits binary blobs
- `include_bytes!` in snake2

### Phase Z6 — Snake2 integration

- Replace runtime `saw_dp::count_saws` calls with ZDD walks
- Measure: target ≤ 50 µs per encode/decode
- Bump MAX_LEN to whatever the math permits

## Open questions

- **Reduction algorithm efficiency**: Sieling-Wegener is `O(|raw DAG| log)`. For
  raw DAG size ~10⁶ that's fine. If raw DAG is much larger we'd need
  hash-consing during construction.
- **Count table memory**: 4 MB per head feels OK; total 40 MB OK to embed.
  If too big, consider sparse count storage (most counts are 0).
- **Encoding edge sequence ↔ snake cell sequence**: the ZDD walk gives us an
  edge bitmask. We need to reconstruct snake cells in head→tail order. This is
  a graph traversal of the edge-induced subgraph (a SAW). Straightforward.
- **D4 symmetry exploitation**: 10 canonical heads vs 64. Symmetry transforms
  the edge sequence too — implement carefully.

## References

- Knuth, TAOCP Volume 4A §7.1.4 — BDDs and ZDDs
- `simpath.w`, `simpath-reduce.w` (Knuth's reference) — `/tmp/knuth/` in dev env
- Sieling & Wegener (1993) — original reduction algorithm
- Knuth's "Simpath" lecture (YouTube) — algorithmic overview
- Kawahara, Inoue, Iwashita, Minato (2017) — modern formal treatment
