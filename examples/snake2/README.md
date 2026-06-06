# snake2 — self-avoiding-walk rank encoding

The snake's body is a self-avoiding walk (SAW) on the grid. We assign every
SAW a unique integer **rank**, store that integer in the game state, and
walk encode/decode to convert between rank and path. The state becomes a
single packed `u64`.

## The problem

> Enumerate every (snake body, apple position) configuration on a `W×H`
> grid, assign each a unique rank in `[0, N)`, and find encode/decode
> functions that let us treat rank as the source of truth.

The body alone is a SAW. We order SAWs by `(length, start_cell, lex(next-
cell choices))` and rank them within that order. Ranks are **grouped by
length** — ranks `[0, total_at_length(0))` are length-0 walks, the next
block is length-1, and so on — so length is recoverable from rank without
storing it separately.

## Library

- `src/saw_rank.rs` — `Saw<W>` with a builder for optional accelerators
  (`prefix_depth(K)` precomputes prefix counts; `enumerated()` builds a
  full path arena). One `decode`/`encode` that picks the fastest available.
- `src/saw_dp.rs` — frontier-state DP for counting SAWs by length.
  Const-generic over `W` (columns) and `H` (rows): works for any
  `W ≤ 12, W × H ≤ 256`.

## Bit budgets

Pack `(snake, apple)` into 64 bits. Naive layout = `apple_bits + walk_bits`.

| Grid  | Cells | Apple bits | Walk bits available |
| ----- | ----- | ---------- | ------------------- |
| 8×8   | 64    | 6          | 58                  |
| 8×9   | 72    | 7          | 57                  |
| 9×9   | 81    | 7          | 57                  |
| 10×10 | 100   | 7          | 57                  |

## What we measured

We enumerated every SAW on three grids with the batched frontier DP
(`count_saws_for_all_lengths::<W, H>`), one pass per start cell.

| Grid | Total SAWs  | log₂      | Fits 64-bit u64?     | Time  |
| ---- | ----------- | --------- | -------------------- | ----- |
| 8×8  | 1.53 × 10¹⁵ | **50.44** | yes (7 bits slack)   | 68 s  |
| 8×9  | 9.28 × 10¹⁶ | **56.37** | yes (exactly)        | 102 s |
| 9×9  | 9.83 × 10¹⁸ | **63.09** | no (apple overflows) | 674 s |

### 9×9 length cap

The walk alone needs 64 bits — adding the apple busts the budget. Capping
walk length at **L=47** brings cumulative SAWs to 1.19 × 10¹⁷ ≈ 2^56.72,
which fits in 57 walk bits. The snake covers up to ~59% of the 9×9 board
(48 cells) before encoding overflows.

### Larger grids

Extrapolating from the 8×8 boundary-clamping ratio, 10×10 sits around
2^80+ and 11×11 around 2^100+. Neither fits a `u64`.

## Conclusions

- **8×9 = 72 cells is the sweet spot** — the largest grid that fits in a
  `u64` snake+apple state with no length cap. 57 walk + 7 apple = 64.
- **8×8** fits with 7 bits of slack.
- **9×9** is borderline. Cap length at 47 or move to a joint
  (snake, apple) encoding that exploits the smaller apple space for long
  snakes.
- **10×10 and up** are out of reach for this scheme on a `u64`.

## Files

- `main.rs` — the snake2 game (8×8 proof of concept, MAX_LEN=22)
- `../../src/saw_rank.rs` — `Saw<W>` rank/unrank
- `../../src/saw_dp.rs` — `count_saws_for_all_lengths::<W, H>` SAW counter
