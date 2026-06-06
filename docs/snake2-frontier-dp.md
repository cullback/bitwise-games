# Snake2: Frontier-State DP design

Working notes for replacing snake2's naive `count_extensions` with a
frontier-state transfer matrix. The encoding scheme (Option B) is the same
across both implementations — only the count function changes.

## Goal

Replace `examples/snake2.rs` `count_extensions(current, visited, remaining)`
— currently O(μ^remaining) backtracking, caps MAX_LEN ≈ 22 — with an O(N · S)
per-query function (N cells, S frontier states), unlocking MAX_LEN ≈ 50.

## Encoding (target, unchanged across the work)

```
| apple_bits: 4 | combined_rank: 60 |   → 64 bits
```

- `combined_rank` indexes every length-1..L_max SAW on the 8×8 grid across
  every starting cell. Each (head, length, body_path) triple is one integer.
- Direction is implicit (head facing = direction from body[0] back to head).
- Head, length, and body shape are all decoded from the rank by peeling.
- `combined_rank ≥ Σ_head cum[head][L_max]` is the DEAD sentinel.

## Count function we need

```rust
fn count_saws(current: u8, visited: u64, remaining: usize) -> u64
```

"Number of length-`remaining` SAWs from `current`, avoiding cells in
`visited`." Used at every decode step to navigate the rank tree.

## Algorithm: frontier-based DP

Process grid cells in row-major order. After processing cell k, the
**frontier** is the boundary between processed and unprocessed cells. For
each frontier position, track the path's connectivity _through the processed
region_.

### Frontier state representation

For each column c ∈ [0, 8), one tag:

- `Empty`: no path passes through this column at the frontier row
- `Free`: an open path endpoint, not yet matched (one of: the START, or the
  "moving end" of the partial path)
- `Arc(id)`: an open endpoint paired with another `Arc(id)` of the same id,
  meaning these two columns connect via the path in the processed region

Matching is non-crossing (the arcs nest). For width 8, the number of valid
configurations is bounded by Motzkin(8) ≈ 323, plus extra states for the
free endpoints' positions — call it ≈ 1000 total states.

### Transitions

At each cell, decide whether the path uses it (`in_path` ∈ {false, true}).
If `in_path`, also decide which of the cell's neighbors (in `visited` _not_
allowed; in processed-but-`Arc(id)` or unprocessed) the path enters from
and exits to. Update the frontier state.

### DP table

```
f[cell_index][frontier_state][length] -> u64 count
```

For each starting head, build a separate DP table. The initial frontier
state has the head marked as `Free` (the start endpoint).

### Per-head storage

- 64 starts × ~1000 states × 50 lengths × 8 bytes ≈ **~26 MB** (loose upper bound)
- Likely much smaller after pruning unreachable states
- Acceptable as a static asset baked via `include_bytes!`

## Rank/unrank flow

### Decode

```
combined_rank → (head, length, body_path)
```

1. Find head: largest h with `Σ_{h'<h} cum[h'][MAX_LEN] ≤ rank`. Subtract off.
2. Find length: largest L with `cum[head][L-1] ≤ rank`. Subtract off.
3. Walk path from head, length L:
   ```
   for step in 1..=L:
       for each legal next move m at current cell (avoiding visited):
           sub = count_saws(next_cell(m), visited | {next_cell(m)}, L - step)
           if rank < sub: take m, break
           else: rank -= sub
       advance to chosen cell
   ```

### Encode

Inverse: walk the path, sum counts of "earlier" candidate moves.

## Implementation phases

### Phase 1 ✅ (committed)

- Working snake2.rs with naive count, MAX_LEN=22
- saw_count.rs investigation tool confirming bit-budget headroom
- This doc

### Phase 2: Frontier DP skeleton ✅

- `src/bin/build_saw_tables.rs` binary with frontier state types and module structure
- Stub transitions that compile but don't yet compute correct counts
- `verify` subcommand cross-checks the naive oracle against OEIS A001411
  (matching at L ≤ 3 from center cells) and hand-computed corner/edge fixtures
- `validate` subcommand cross-checks `Dp::count` against the naive oracle for
  every (start, length) with L ≤ 4 — currently fails 320/320, becomes the
  target spec for Phase 3

### Phase 3: Correct transitions

- Implement the path-edge decision logic (which neighbors connect)
- Handle arc creation, extension, merging
- Validate against naive on 4×4 grid, lengths up to 10

### Phase 4: 8×8 scale-up + table emission

- Run full DP for all 64 heads, all lengths to MAX_LEN
- Emit `assets/saw_tables.bin` (binary blob)
- Validate counts match `saw_count` totals

### Phase 5: snake2 integration

- Bump MAX_LEN, switch to table-driven count
- Wire `combined_rank` (Option B) layout: 4-bit apple, 60-bit rank
- Update tests, verify round-trip at high length
- Measure decode time

### Phase 6: polish

- Profile decode; if slow, restructure tables for cache friendliness
- Document the encoding in snake2.rs top docstring

## Key references

- Knuth, TAOCP Volume 4A, §7.1.4: BDDs / ZDDs / "Simpath"
- Kawahara, Inoue, Iwashita, Minato (2017): "Frontier-based search for
  enumerating all constrained subgraphs with compressed representation"
- Graphillion library (Python, Inoue et al.) — same algorithm, off-the-shelf

## Open design questions

- Frontier state encoding: bit-packed integer vs typed enum? Start with
  enum for clarity, bit-pack later if hot path needs it.
- Arc id assignment: canonicalize on creation (smallest unused id)?
- Per-head table layout: pack lengths together (cache-friendly for one
  head) or pack states together (cache-friendly across lengths)?

These are decisions for Phase 4 once correctness is solid.
