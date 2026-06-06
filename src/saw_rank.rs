// Self-avoiding walk rank/unrank on a W×W grid.
//
// A SAW is a sequence of distinct grid cells where consecutive cells are
// orthogonally adjacent. This module assigns each SAW a unique `u64` rank
// in a fixed canonical order (by length, then by start cell, then by path
// in the lexicographic ordering of next-cell choices), and provides:
//
//   - count(start, length) → number of length-L SAWs starting at `start`
//   - total_up_to(length)  → sum over (start, L' ≤ L) of count(start, L')
//   - decode(rank)         → the SAW at this rank
//   - encode(path)         → the rank of this SAW
//
// The grid size W is a const generic. Visited cells are tracked in a u128
// bitmask, which limits W to 11 (121 cells). Larger grids would need
// a different bitset.
//
// Conditional counts during rank navigation use naive backtracking. That
// makes encode/decode `O(μ^L)` per query — fine for short paths and small
// grids, painful past L ≈ 25. For high-performance use, plug in a
// frontier-state DP (see `saw_dp` module) for the count function.

/// SAW rank/unrank tables for a W×W grid up to `max_length` cells.
///
/// The struct stores `cum_per_start[start][length] = count(start, length)`
/// and `cum_lengths[length] = sum over s, L' < length of count(s, L')` so
/// rank arithmetic at the length / start level is O(W²).
pub struct Saw<const W: usize> {
    /// `cum_per_start[start][length]` = number of length-`length` SAWs from
    /// `start`. Includes length 0 (just the start cell, value 1).
    cum_per_start: Vec<Vec<u64>>,
    /// `cum_lengths[L]` = total number of SAWs of length < L, summed over
    /// every starting cell. Used to peel "length" off a rank.
    /// `cum_lengths[max_length + 1]` is the grand total.
    cum_lengths: Vec<u64>,
    max_length: usize,
}

const fn n_cells<const W: usize>() -> usize {
    W * W
}

/// Neighbors of `cell` in canonical (ascending cell-index) order.
fn neighbors<const W: usize>(cell: u8) -> impl Iterator<Item = u8> {
    let w = W as i32;
    let r = (cell as i32) / w;
    let c = (cell as i32) % w;
    // Ascending cell-index order: up, left, right, down.
    let mut out = [None, None, None, None];
    if r > 0 {
        out[0] = Some((cell as i32 - w) as u8);
    }
    if c > 0 {
        out[1] = Some(cell - 1);
    }
    if c + 1 < w {
        out[2] = Some(cell + 1);
    }
    if r + 1 < w {
        out[3] = Some((cell as i32 + w) as u8);
    }
    out.into_iter().flatten()
}

/// Count length-`remaining` SAW extensions from `pos`, avoiding cells set
/// in `visited`. Naive backtracking — `O(μ^remaining)`.
fn count_extensions<const W: usize>(pos: u8, visited: u128, remaining: usize) -> u64 {
    if remaining == 0 {
        return 1;
    }
    let mut total = 0u64;
    for n in neighbors::<W>(pos) {
        if (visited >> n) & 1 != 0 {
            continue;
        }
        total += count_extensions::<W>(n, visited | (1u128 << n), remaining - 1);
    }
    total
}

impl<const W: usize> Saw<W> {
    /// Build rank tables for SAWs of length 0..=`max_length` on a W×W grid.
    pub fn new(max_length: usize) -> Self {
        assert!(
            W > 0 && W <= 11,
            "Saw<W> requires 1 ≤ W ≤ 11 (visited mask fits in u128)"
        );
        let n = n_cells::<W>();
        let mut cum_per_start = vec![vec![0u64; max_length + 1]; n];
        for start in 0..n {
            for length in 0..=max_length {
                cum_per_start[start][length] =
                    count_extensions::<W>(start as u8, 1u128 << start, length);
            }
        }
        let mut cum_lengths = vec![0u64; max_length + 2];
        for length in 0..=max_length {
            let length_total: u64 = (0..n).map(|s| cum_per_start[s][length]).sum();
            cum_lengths[length + 1] = cum_lengths[length] + length_total;
        }
        Saw {
            cum_per_start,
            cum_lengths,
            max_length,
        }
    }

    pub fn max_length(&self) -> usize {
        self.max_length
    }

    /// `count(start, length)` = number of length-`length` SAWs from `start`.
    pub fn count(&self, start: u8, length: usize) -> u64 {
        self.cum_per_start[start as usize][length]
    }

    /// Total number of SAWs of length 0..=`length` across every starting cell.
    pub fn total_up_to(&self, length: usize) -> u64 {
        self.cum_lengths[length + 1]
    }

    /// Total SAWs of length exactly `length` across every starting cell.
    pub fn total_at_length(&self, length: usize) -> u64 {
        self.cum_lengths[length + 1] - self.cum_lengths[length]
    }

    /// Grand total across every length and starting cell.
    pub fn total(&self) -> u64 {
        self.cum_lengths[self.max_length + 1]
    }

    /// Decode `rank` into the SAW it represents. Panics if `rank ≥ total()`.
    pub fn decode(&self, rank: u64) -> Vec<u8> {
        assert!(
            rank < self.total(),
            "rank {rank} out of bounds; total is {}",
            self.total()
        );

        // Peel off length.
        let mut length = 0usize;
        while length <= self.max_length && rank >= self.cum_lengths[length + 1] {
            length += 1;
        }
        let mut local = rank - self.cum_lengths[length];

        // Peel off start cell.
        let n = n_cells::<W>();
        let mut start = 0usize;
        while start < n && local >= self.cum_per_start[start][length] {
            local -= self.cum_per_start[start][length];
            start += 1;
        }
        debug_assert!(start < n);

        // Walk the path of length `length` from `start`, using `local` to pick
        // each next cell.
        let mut path = Vec::with_capacity(length + 1);
        path.push(start as u8);
        let mut visited = 1u128 << start;
        let mut current = start as u8;
        let mut remaining = length;

        while remaining > 0 {
            let mut picked = None;
            for n in neighbors::<W>(current) {
                if (visited >> n) & 1 != 0 {
                    continue;
                }
                let sub_visited = visited | (1u128 << n);
                let sub_count = count_extensions::<W>(n, sub_visited, remaining - 1);
                if local < sub_count {
                    picked = Some(n);
                    break;
                }
                local -= sub_count;
            }
            let next = picked.expect("decode: count tree exhausted, rank inconsistent");
            path.push(next);
            visited |= 1u128 << next;
            current = next;
            remaining -= 1;
        }

        path
    }

    /// Encode `path` to its rank. `path` must be a valid SAW of length ≤
    /// `max_length` on this grid.
    pub fn encode(&self, path: &[u8]) -> u64 {
        let length = path.len().checked_sub(1).expect("path must be non-empty");
        assert!(length <= self.max_length, "path longer than max_length");
        let start = path[0] as usize;
        let n = n_cells::<W>();
        assert!(start < n, "path start out of bounds");

        // Length offset.
        let mut rank = self.cum_lengths[length];
        // Start offset (within this length).
        for s in 0..start {
            rank += self.cum_per_start[s][length];
        }

        // Path offset (within this start + length).
        let mut visited = 1u128 << start;
        let mut current = path[0];
        for step in 1..=length {
            let actual = path[step];
            assert!(
                (visited >> actual) & 1 == 0,
                "path is not a SAW: cell {actual} revisited"
            );
            let mut matched = false;
            for n in neighbors::<W>(current) {
                if (visited >> n) & 1 != 0 {
                    continue;
                }
                if n == actual {
                    matched = true;
                    break;
                }
                let sub_visited = visited | (1u128 << n);
                rank += count_extensions::<W>(n, sub_visited, length - step);
            }
            assert!(matched, "path is not a SAW: step {step} not adjacent");
            visited |= 1u128 << actual;
            current = actual;
        }

        rank
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- OEIS A001411 (SAWs on Z² from origin) cross-check ---
    //
    // For a cell at the center of a W×W grid with `boundary_distance` ≥ L,
    // the count of length-L SAWs equals A001411[L] (the boundary doesn't
    // affect any walk of that length). We pick W=11, center=(5,5), so
    // boundary distance = 5, validating L ∈ {0..5}.

    const OEIS_A001411: &[u64] = &[1, 4, 12, 36, 100, 284, 780];

    #[test]
    fn count_matches_oeis_at_center_of_11x11() {
        let saw = Saw::<11>::new(5);
        let center = 5 * 11 + 5;
        for l in 0..=5 {
            assert_eq!(
                saw.count(center, l),
                OEIS_A001411[l],
                "count(center, {l}) should match A001411[{l}]"
            );
        }
    }

    #[test]
    fn count_corner_4x4_hand_computed() {
        // Hand-checked corner walks on a 4×4 grid.
        let saw = Saw::<4>::new(3);
        let corner = 0;
        assert_eq!(saw.count(corner, 0), 1);
        assert_eq!(saw.count(corner, 1), 2);
        assert_eq!(saw.count(corner, 2), 4);
        assert_eq!(saw.count(corner, 3), 10);
    }

    // --- Round-trip: encode(decode(rank)) == rank ---

    #[test]
    fn round_trip_3x3_all_saws() {
        // 3×3 = 9 cells. Up to length 8 = full Hamiltonian.
        let saw = Saw::<3>::new(8);
        let total = saw.total();
        for rank in 0..total {
            let path = saw.decode(rank);
            let re = saw.encode(&path);
            assert_eq!(
                rank, re,
                "round-trip failed at rank {rank}, path = {path:?}"
            );
        }
    }

    #[test]
    fn round_trip_4x4_exhaustive_short() {
        // 4×4, all SAWs up to length 4 (~thousands of them).
        let saw = Saw::<4>::new(4);
        let total = saw.total();
        for rank in 0..total {
            let path = saw.decode(rank);
            let re = saw.encode(&path);
            assert_eq!(
                rank, re,
                "round-trip failed at rank {rank}, path = {path:?}"
            );
        }
    }

    #[test]
    fn lengths_sorted_first() {
        // Decode(0) should be the first length-0 SAW (just cell 0).
        // Decode(total_up_to(0)) should be the first length-1 SAW from cell 0.
        let saw = Saw::<4>::new(3);
        let first = saw.decode(0);
        assert_eq!(first, vec![0u8]);
        let after_l0 = saw.decode(saw.total_up_to(0));
        assert_eq!(after_l0.len(), 2);
        assert_eq!(after_l0[0], 0);
    }

    // --- Scale exploration: characterize where the library starts hurting ---

    #[test]
    #[ignore]
    fn scale_table_init_times() {
        // Print init time + memory footprint for various (W, L) combos.
        use std::time::Instant;
        eprintln!(
            "\n{:>4} {:>4} {:>14} {:>14} {:>10}",
            "W", "L", "total_saws", "log2(total)", "build_ms"
        );
        macro_rules! run {
            ($w:expr, $l:expr) => {{
                let t = Instant::now();
                let saw = Saw::<$w>::new($l);
                let total = saw.total();
                let elapsed = t.elapsed().as_millis();
                eprintln!(
                    "{:>4} {:>4} {:>14} {:>14.2} {:>10}",
                    $w,
                    $l,
                    total,
                    (total as f64).log2(),
                    elapsed
                );
            }};
        }
        run!(3, 8);
        run!(4, 8);
        run!(5, 10);
        run!(6, 12);
        run!(7, 14);
        run!(8, 14);
        run!(8, 18);
        run!(9, 14);
        run!(10, 14);
        run!(11, 14);
    }

    #[test]
    #[ignore]
    fn scale_round_trip_4x4_full_hamiltonian() {
        // Full Hamiltonian round-trip on 4×4: every SAW of length 0..15.
        // 16-cell grid → length up to 15 is the longest possible SAW.
        use std::time::Instant;
        let t = Instant::now();
        let saw = Saw::<4>::new(15);
        let build_ms = t.elapsed().as_millis();
        let total = saw.total();
        eprintln!(
            "\n4×4 max-length build: total={total} (log2={:.2}), build={build_ms} ms",
            (total as f64).log2()
        );
        let t = Instant::now();
        let mut sampled = 0;
        // Spot-check 1000 evenly spaced ranks (full enumeration is too many).
        let step = (total / 1000).max(1);
        let mut rank = 0;
        while rank < total {
            let path = saw.decode(rank);
            let re = saw.encode(&path);
            assert_eq!(rank, re);
            sampled += 1;
            rank += step;
        }
        let elapsed = t.elapsed().as_millis();
        eprintln!(
            "4×4 round-trip sample: {sampled} pairs in {elapsed} ms ({:.2} µs each)",
            (elapsed as f64) * 1000.0 / sampled as f64
        );
    }

    #[test]
    fn starts_sorted_within_length() {
        // Within length 1, decode in start-cell order: first all SAWs from
        // start 0, then start 1, etc.
        let saw = Saw::<4>::new(1);
        let l0_total = saw.total_up_to(0);
        let mut at = l0_total;
        for start in 0..16u8 {
            let n_from_start = saw.count(start, 1);
            for _ in 0..n_from_start {
                let p = saw.decode(at);
                assert_eq!(p[0], start, "decode(rank={at}) should start at {start}");
                at += 1;
            }
        }
    }
}
