# Minesweeper

A minesweeper variant where every board is provably clearable without guessing, and the entire game state — board choice, every reveal, every flag — fits in 64 bits.

The biggest departure from classic minesweeper: **flagging a non-mine kills you**. Flags aren't tentative — they're a commitment that the cell under them is a mine, on penalty of losing the game. This collapses the usual two-bit-per-cell state (hidden / revealed / flagged) to one bit, and turns flagging into a real decision instead of a scratchpad annotation.

## Controls

- **Mouse**: hover over a cell
- **Z**: reveal — kills if the cell is a mine
- **X**: flag — kills if the cell is _not_ a mine

On the first Z/X press, the seed rotates to one whose board has your hovered cell as a deductively-safe cascade origin — so the first click always opens at the cell you actually pointed at.

## Run it

```
cargo run --release --example minesweeper -- 42
```

The argument is a 7-bit seed (`0..127`). Same seed + same first-click cell = same board.

## How it differs from classic minesweeper

|              | Classic                | This                                  |
| ------------ | ---------------------- | ------------------------------------- |
| Flags        | Tentative scratchpad   | **Commitments — wrong flag = death**  |
| Guessing     | Sometimes forced       | Never; every board solvable           |
| First click  | Moves mines under hood | Rotates seed to a valid board         |
| Board size   | 9×9 / 16×16 / 16×30    | 8×9, always                           |
| Mine density | 12% – 21%              | 18% (between Intermediate and Expert) |
| End screen   | Modal overlay          | Right-side panel shows `WIN` / `DIE`  |
| State        | Unbounded              | 64 bits, full round-trip              |

## State packing (64 bits)

| bits   | field                                                                             |
| ------ | --------------------------------------------------------------------------------- |
| `0..7` | seed (128 boards)                                                                 |
| `7`    | dead flag                                                                         |
| `8..`  | per-cell assertion bits, one per _interactive_ cell (mine or numbered) — up to 56 |

Zero cells aren't stored. They display as revealed when every numbered cell on their region's border is revealed.

The `engine::Constraints { max_numbered: Some(43), .. }` config keeps every generated board within the 56-bit cell budget (`43 numbered + 13 mines = 56`).

## No-guess guarantee

Every accepted board is clearable by the engine's deductive solver, which combines:

- **Rule A**: a number's `count − flagged_neighbors == hidden_neighbor_count` → flag all hidden
- **Rule B**: a number's `flagged_neighbors == count` → reveal all hidden
- **Subset propagation**: if revealed number X's hidden-neighbor set is a subset of revealed number Y's, the difference is its own constraint. Saturated or empty derived constraints force flags or reveals.

Rules A and B alone won't crack every board — most named multi-cell patterns (1-1, 1-2-1, 1-2-2-1, the triangle patterns) all reduce to subset reasoning. The solver discovers them without hard-coded matchers:

- <https://minesweeper-pro.com/advanced-patterns/>
- <https://minesweepergame.com/strategy/patterns.php>

Boards requiring deduction beyond subset (rare; usually needs constraint-graph search) are rejected at generation.

## First-click rotation

The first input doesn't reveal the cell under your cursor in the _current_ board — instead, it searches the 128 seeds for one whose board has your cell as a deductively-solvable zero, and switches to that seed before cascading. The clicked cell ends up as the cascade origin in whatever board the search lands on.

The search start is a hash of `(initial_seed, target_cell)`, so the CLI seed acts as a salt: different starting seeds + same click = different final boards. Coverage test confirms every cell has at least 17 valid landing seeds (avg 29).

This trades classic minesweeper's "move mines under the first click" trick for a closed-form mapping that doesn't need to store the click location anywhere.

## Engine module

`engine.rs` is a standalone board generator + solver, separable from the rest:

```rust
use engine::{Config, Constraints, RuleSet, generate, solve, three_bv};

let cfg = Config {
    rows: 9, cols: 8, mines: 13,
    rules: RuleSet::full(),                    // A + B + subset
    constraints: Constraints {
        min_cascade_size: Some(10),            // big opening
        max_numbered: Some(43),                // bit budget
        ..Default::default()
    },
    max_attempts: 5000,
};

let board = generate(&cfg, seed)?;
let difficulty = three_bv(&board);             // Bechtel's Board Benchmark Value
```

Toggle `RuleSet::basic()` to get easier boards solvable by rules A and B alone (about 28% of full-rule boards become unsolvable under the basic ruleset).

Up to ~10×10 (128-cell limit set by the `u128` hidden-set bitmask in the subset rule).

## Files

- `main.rs` — game state, rendering, input handling
- `engine.rs` — pluggable board generator and deductive solver
- `density.rs` — one-off experiment: largest-zero-region cascade vs. row-major-first cascade. Established that the largest-region heuristic cuts "claustrophobic" openings (<10 cells) from 8% to under 1%.
- `notes.md` — design research: density tuning, no-guess literature, phase transition pointers
- `minesweeper.aseprite` — 14 tile sprites (12×12)
- `../../assets/6x4-alphanum.aseprite` — shared 4×6 font, 36 glyphs (`0`–`9`, `A`–`Z`)
