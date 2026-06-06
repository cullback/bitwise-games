#![allow(dead_code)]
// Self-avoiding walk rank/unrank on a W×W grid.
//
// A SAW is a sequence of distinct grid cells where consecutive cells are
// orthogonally adjacent. This module assigns each SAW a unique `u64` rank
// in a fixed canonical order — by length, then start cell, then path in
// the lexicographic ordering of next-cell choices — and provides:
//
//   - count(start, length) → number of length-`length` SAWs starting at `start`
//   - total_at_length / total_up_to / total — running sums of count
//   - decode(rank)        → the SAW at this rank
//   - encode(path)        → the rank of this SAW
//
// The grid size `W` is a const generic. Visited cells are tracked in a
// u128 bitmask, capping `W` at 11 (121 cells).
//
// `Saw` is built either directly (nav only) or via a builder that adds
// optional accelerators:
//
//   - `.prefix_depth(K)`: precompute counts for every SAW prefix of length
//     1..=K. Decode/encode skip exponential-cost recursion for the first
//     K steps. Memory grows with prefix count × (L_max − K).
//   - `.enumerated()`: enumerate every SAW into a flat byte arena. Decode
//     is O(1); encode is one hashmap lookup. Memory grows with the total
//     path-byte count + entries.
//
// Both accelerators can be combined. `decode` prefers enumeration over
// prefix table over naive backtracking.

use rustc_hash::FxHashMap;

pub struct Saw<const W: usize, const H: usize> {
    /// cum_per_start[start][length] = count(start, length).
    cum_per_start: Vec<Vec<u64>>,
    /// cum_lengths[L] = total SAWs of length < L over every start.
    cum_lengths: Vec<u64>,
    max_length: usize,
    prefix: Option<PrefixTables>,
    enumeration: Option<Enumeration>,
}

struct PrefixTables {
    /// counts[(current, visited)][r] = count_extensions(current, visited, r).
    counts: FxHashMap<(u8, u128), Box<[u64]>>,
}

struct Enumeration {
    /// All paths concatenated; path i lives at `arena[offsets[i]..offsets[i+1]]`.
    arena: Vec<u8>,
    offsets: Vec<u32>,
    rank_of: FxHashMap<Box<[u8]>, u64>,
}

pub struct SawBuilder<const W: usize, const H: usize> {
    max_length: usize,
    prefix_depth: Option<usize>,
    enumerated: bool,
}

const fn n_cells<const W: usize, const H: usize>() -> usize {
    W * H
}

/// Neighbors of `cell` in canonical (ascending cell-index) order. Cells are
/// indexed row-major: `row * W + col`, with `0 ≤ row < H` and `0 ≤ col < W`.
fn neighbors<const W: usize, const H: usize>(cell: u8) -> impl Iterator<Item = u8> {
    let w = W as i32;
    let h = H as i32;
    let r = (cell as i32) / w;
    let c = (cell as i32) % w;
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
    if r + 1 < h {
        out[3] = Some((cell as i32 + w) as u8);
    }
    out.into_iter().flatten()
}

/// Naive backtracking count: number of length-`remaining` SAW extensions
/// from `pos` avoiding `visited`. O(μ^remaining).
fn count_extensions<const W: usize, const H: usize>(
    pos: u8,
    visited: u128,
    remaining: usize,
) -> u64 {
    if remaining == 0 {
        return 1;
    }
    let mut total = 0u64;
    for n in neighbors::<W, H>(pos) {
        if (visited >> n) & 1 != 0 {
            continue;
        }
        total += count_extensions::<W, H>(n, visited | (1u128 << n), remaining - 1);
    }
    total
}

impl<const W: usize, const H: usize> Saw<W, H> {
    /// Build the rank tables (nav only). Equivalent to
    /// `Saw::builder(max_length).build()`.
    pub fn new(max_length: usize) -> Self {
        Self::builder(max_length).build()
    }

    pub fn builder(max_length: usize) -> SawBuilder<W, H> {
        SawBuilder {
            max_length,
            prefix_depth: None,
            enumerated: false,
        }
    }

    /// Construct a `Saw<W>` directly from precomputed cum tables. Skips the
    /// expensive `count_extensions` enumeration at startup. Use the
    /// `print_const_tables` test in this module to generate the literals.
    pub fn from_tables(
        cum_per_start: Vec<Vec<u64>>,
        cum_lengths: Vec<u64>,
        max_length: usize,
    ) -> Self {
        assert_eq!(cum_per_start.len(), W * H);
        for v in &cum_per_start {
            assert_eq!(v.len(), max_length + 1);
        }
        assert_eq!(cum_lengths.len(), max_length + 2);
        Saw {
            cum_per_start,
            cum_lengths,
            max_length,
            prefix: None,
            enumeration: None,
        }
    }

    pub fn max_length(&self) -> usize {
        self.max_length
    }

    pub fn count(&self, start: u8, length: usize) -> u64 {
        self.cum_per_start[start as usize][length]
    }

    pub fn total_up_to(&self, length: usize) -> u64 {
        self.cum_lengths[length + 1]
    }

    pub fn total_at_length(&self, length: usize) -> u64 {
        self.cum_lengths[length + 1] - self.cum_lengths[length]
    }

    pub fn total(&self) -> u64 {
        self.cum_lengths[self.max_length + 1]
    }

    /// Rough byte footprint of all internal tables.
    pub fn memory_bytes(&self) -> usize {
        let mut total = std::mem::size_of_val(self);
        for v in &self.cum_per_start {
            total += v.len() * std::mem::size_of::<u64>();
        }
        total += self.cum_lengths.len() * std::mem::size_of::<u64>();
        if let Some(pfx) = &self.prefix {
            let entry = std::mem::size_of::<(u8, u128)>() + std::mem::size_of::<Box<[u64]>>();
            total += pfx.counts.len() * entry;
            for v in pfx.counts.values() {
                total += v.len() * std::mem::size_of::<u64>();
            }
        }
        if let Some(en) = &self.enumeration {
            total += en.arena.len();
            total += en.offsets.len() * std::mem::size_of::<u32>();
            total += en.rank_of.len() * (std::mem::size_of::<(Box<[u8]>, u64)>() + 16);
        }
        total
    }

    /// Conditional count used by nav-based decode/encode. Uses the prefix
    /// table when the (current, visited) pair is present; otherwise falls
    /// back to naive recursion.
    fn count_for(&self, current: u8, visited: u128, remaining: usize) -> u64 {
        if let Some(pfx) = &self.prefix {
            if let Some(arr) = pfx.counts.get(&(current, visited)) {
                if remaining < arr.len() {
                    return arr[remaining];
                }
            }
        }
        count_extensions::<W, H>(current, visited, remaining)
    }

    pub fn decode(&self, rank: u64) -> Vec<u8> {
        assert!(
            rank < self.total(),
            "rank {rank} out of bounds; total is {}",
            self.total()
        );

        if let Some(en) = &self.enumeration {
            let i = rank as usize;
            return en.arena[en.offsets[i] as usize..en.offsets[i + 1] as usize].to_vec();
        }

        // Peel length.
        let mut length = 0usize;
        while length <= self.max_length && rank >= self.cum_lengths[length + 1] {
            length += 1;
        }
        let mut local = rank - self.cum_lengths[length];

        // Peel start cell.
        let n = n_cells::<W, H>();
        let mut start = 0usize;
        while start < n && local >= self.cum_per_start[start][length] {
            local -= self.cum_per_start[start][length];
            start += 1;
        }
        debug_assert!(start < n);

        let mut path = Vec::with_capacity(length + 1);
        path.push(start as u8);
        let mut visited = 1u128 << start;
        let mut current = start as u8;
        let mut remaining = length;

        while remaining > 0 {
            let mut picked = None;
            for next in neighbors::<W, H>(current) {
                if (visited >> next) & 1 != 0 {
                    continue;
                }
                let new_visited = visited | (1u128 << next);
                let sub_count = self.count_for(next, new_visited, remaining - 1);
                if local < sub_count {
                    picked = Some(next);
                    break;
                }
                local -= sub_count;
            }
            let next = picked.expect("decode: count tree exhausted");
            path.push(next);
            visited |= 1u128 << next;
            current = next;
            remaining -= 1;
        }

        path
    }

    pub fn encode(&self, path: &[u8]) -> u64 {
        let length = path.len().checked_sub(1).expect("path must be non-empty");
        assert!(length <= self.max_length, "path longer than max_length");
        let start = path[0] as usize;
        let n = n_cells::<W, H>();
        assert!(start < n, "path start out of bounds");

        if let Some(en) = &self.enumeration {
            return *en.rank_of.get(path).expect("path not in enumeration");
        }

        let mut rank = self.cum_lengths[length];
        for s in 0..start {
            rank += self.cum_per_start[s][length];
        }

        let mut visited = 1u128 << start;
        let mut current = path[0];
        for step in 1..=length {
            let actual = path[step];
            assert!(
                (visited >> actual) & 1 == 0,
                "path is not a SAW: cell {actual} revisited"
            );
            let mut matched = false;
            for next in neighbors::<W, H>(current) {
                if (visited >> next) & 1 != 0 {
                    continue;
                }
                if next == actual {
                    matched = true;
                    break;
                }
                let new_visited = visited | (1u128 << next);
                rank += self.count_for(next, new_visited, length - step);
            }
            assert!(matched, "path is not a SAW: step {step} not adjacent");
            visited |= 1u128 << actual;
            current = actual;
        }

        rank
    }
}

impl<const W: usize, const H: usize> SawBuilder<W, H> {
    /// Precompute conditional counts for every SAW prefix of length 1..=K.
    /// Decode/encode then use array lookups for the first K steps and fall
    /// back to naive recursion for the rest. Build cost is dominated by
    /// the deepest layer's `count_extensions(_, _, L_max − K)` calls.
    pub fn prefix_depth(mut self, k: usize) -> Self {
        self.prefix_depth = Some(k);
        self
    }

    /// Enumerate every SAW into a flat byte arena for O(1) decode and
    /// O(1) encode via hashmap. Memory-heavy; use for combos that fit in RAM.
    pub fn enumerated(mut self) -> Self {
        self.enumerated = true;
        self
    }

    pub fn build(self) -> Saw<W, H> {
        assert!(
            W > 0 && H > 0 && W * H <= 128,
            "Saw<W, H> requires W*H ≤ 128 (visited mask fits in u128)"
        );
        if let Some(k) = self.prefix_depth {
            assert!(k <= self.max_length, "prefix_depth must be ≤ max_length");
        }

        let n = n_cells::<W, H>();
        let max_length = self.max_length;

        // Cumulative counts (always built).
        let mut cum_per_start = vec![vec![0u64; max_length + 1]; n];
        for start in 0..n {
            for length in 0..=max_length {
                cum_per_start[start][length] =
                    count_extensions::<W, H>(start as u8, 1u128 << start, length);
            }
        }
        let mut cum_lengths = vec![0u64; max_length + 2];
        for length in 0..=max_length {
            let length_total: u64 = (0..n).map(|s| cum_per_start[s][length]).sum();
            cum_lengths[length + 1] = cum_lengths[length] + length_total;
        }

        let prefix = self.prefix_depth.map(|k| {
            let mut counts: FxHashMap<(u8, u128), Box<[u64]>> = FxHashMap::default();
            for start in 0..n {
                enumerate_prefix_states::<W, H>(
                    start as u8,
                    1u128 << start,
                    1,
                    k,
                    max_length,
                    &mut counts,
                );
            }
            PrefixTables { counts }
        });

        let enumeration = if self.enumerated {
            let mut arena: Vec<u8> = Vec::new();
            let mut offsets: Vec<u32> = vec![0];
            for length in 0..=max_length {
                for start in 0..n {
                    let mut path = vec![start as u8];
                    enumerate_paths::<W, H>(
                        start as u8,
                        1u128 << start,
                        length,
                        &mut path,
                        &mut arena,
                        &mut offsets,
                    );
                }
            }
            let mut rank_of: FxHashMap<Box<[u8]>, u64> = FxHashMap::default();
            rank_of.reserve(offsets.len() - 1);
            for i in 0..(offsets.len() - 1) {
                let path = &arena[offsets[i] as usize..offsets[i + 1] as usize];
                rank_of.insert(path.to_vec().into_boxed_slice(), i as u64);
            }
            Some(Enumeration {
                arena,
                offsets,
                rank_of,
            })
        } else {
            None
        };

        Saw {
            cum_per_start,
            cum_lengths,
            max_length,
            prefix,
            enumeration,
        }
    }
}

fn enumerate_prefix_states<const W: usize, const H: usize>(
    current: u8,
    visited: u128,
    next_depth: usize,
    max_depth: usize,
    max_length: usize,
    counts: &mut FxHashMap<(u8, u128), Box<[u64]>>,
) {
    for next in neighbors::<W, H>(current) {
        if (visited >> next) & 1 != 0 {
            continue;
        }
        let new_visited = visited | (1u128 << next);
        let key = (next, new_visited);
        if !counts.contains_key(&key) {
            let r_max = max_length - next_depth;
            let mut entries = vec![0u64; r_max + 1];
            entries[0] = 1;
            for r in 1..=r_max {
                entries[r] = count_extensions::<W, H>(next, new_visited, r);
            }
            counts.insert(key, entries.into_boxed_slice());
        }
        if next_depth < max_depth {
            enumerate_prefix_states::<W, H>(
                next,
                new_visited,
                next_depth + 1,
                max_depth,
                max_length,
                counts,
            );
        }
    }
}

fn enumerate_paths<const W: usize, const H: usize>(
    pos: u8,
    visited: u128,
    remaining: usize,
    path: &mut Vec<u8>,
    arena: &mut Vec<u8>,
    offsets: &mut Vec<u32>,
) {
    if remaining == 0 {
        arena.extend_from_slice(path);
        offsets.push(arena.len() as u32);
        return;
    }
    for next in neighbors::<W, H>(pos) {
        if (visited >> next) & 1 != 0 {
            continue;
        }
        path.push(next);
        enumerate_paths::<W, H>(
            next,
            visited | (1u128 << next),
            remaining - 1,
            path,
            arena,
            offsets,
        );
        path.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- OEIS A001411 (SAWs on Z² from origin) cross-check ---
    //
    // For a cell at the center of a W×W grid with boundary distance ≥ L,
    // the count of length-L SAWs equals A001411[L].

    const OEIS_A001411: &[u64] = &[1, 4, 12, 36, 100, 284, 780];

    #[test]
    fn count_matches_oeis_at_center_of_11x11() {
        let saw = Saw::<11, 11>::new(5);
        let center = 5 * 11 + 5;
        for l in 0..=5 {
            assert_eq!(saw.count(center, l), OEIS_A001411[l]);
        }
    }

    #[test]
    fn count_corner_4x4_hand_computed() {
        let saw = Saw::<4, 4>::new(3);
        let corner = 0;
        assert_eq!(saw.count(corner, 0), 1);
        assert_eq!(saw.count(corner, 1), 2);
        assert_eq!(saw.count(corner, 2), 4);
        assert_eq!(saw.count(corner, 3), 10);
    }

    #[test]
    #[ignore]
    fn print_const_tables_9x8_full() {
        // Run with: cargo test --release --example snake2 print_const_tables -- --ignored --nocapture
        // Pipe to examples/snake2/saw_tables.rs.
        //
        // Uses the polynomial `saw_dp` counter instead of naive `Saw::new`,
        // so MAX_LEN can go all the way to the Hamiltonian (W*H − 1) without
        // exponential build time.
        use crate::saw_dp;
        const W: usize = 9;
        const H: usize = 8;
        const MAX_LEN: usize = 71; // W*H − 1; full-board fill possible
        let n_cells = W * H;

        let mut cum_per_start: Vec<Vec<u64>> = vec![vec![0; MAX_LEN + 1]; n_cells];
        for start in 0..n_cells {
            let counts = saw_dp::count_saws_for_all_lengths::<W, H>(start as u8, MAX_LEN, 0);
            cum_per_start[start] = counts;
        }
        let mut cum_lengths = vec![0u64; MAX_LEN + 2];
        for l in 0..=MAX_LEN {
            let at_l: u64 = (0..n_cells).map(|s| cum_per_start[s][l]).sum();
            cum_lengths[l + 1] = cum_lengths[l] + at_l;
        }

        println!("// AUTO-GENERATED — do not edit.");
        println!(
            "// 9×8 grid (W=9, H=8), MAX_LEN = {MAX_LEN}. Re-run print_const_tables_9x8_full to refresh."
        );
        println!();
        println!("pub const MAX_LENGTH: usize = {MAX_LEN};");
        println!();
        println!(
            "pub const CUM_PER_START: [[u64; {}]; {n_cells}] = [",
            MAX_LEN + 1
        );
        for row in &cum_per_start {
            print!("    [");
            for (i, v) in row.iter().enumerate() {
                if i > 0 {
                    print!(", ");
                }
                print!("{v}");
            }
            println!("],");
        }
        println!("];");
        println!();
        println!("pub const CUM_LENGTHS: [u64; {}] = [", MAX_LEN + 2);
        print!("    ");
        for (i, v) in cum_lengths.iter().enumerate() {
            if i > 0 {
                print!(", ");
            }
            print!("{v}");
        }
        println!();
        println!("];");
    }

    #[test]
    fn snake2_fresh_board_round_trip() {
        // Snake2's fresh_board state: head at (3,2)=26, body at (3,1)=25.
        let saw = Saw::<8, 8>::new(14);
        let cells = vec![26u8, 25u8];
        let rank = saw.encode(&cells);
        let path = saw.decode(rank);
        assert_eq!(path, cells, "round trip failed: rank={rank} → {path:?}");
        eprintln!("snake2 fresh-board rank = {rank}, total = {}", saw.total());
    }

    #[test]
    fn round_trip_3x3_all_saws() {
        let saw = Saw::<3, 3>::new(8);
        for rank in 0..saw.total() {
            let path = saw.decode(rank);
            assert_eq!(saw.encode(&path), rank, "round-trip at rank {rank}");
        }
    }

    #[test]
    fn round_trip_4x4_exhaustive_short() {
        let saw = Saw::<4, 4>::new(4);
        for rank in 0..saw.total() {
            let path = saw.decode(rank);
            assert_eq!(saw.encode(&path), rank);
        }
    }

    #[test]
    fn lengths_sorted_first() {
        let saw = Saw::<4, 4>::new(3);
        assert_eq!(saw.decode(0), vec![0u8]);
        let after_l0 = saw.decode(saw.total_up_to(0));
        assert_eq!(after_l0.len(), 2);
        assert_eq!(after_l0[0], 0);
    }

    #[test]
    fn starts_sorted_within_length() {
        let saw = Saw::<4, 4>::new(1);
        let l0_total = saw.total_up_to(0);
        let mut at = l0_total;
        for start in 0..16u8 {
            for _ in 0..saw.count(start, 1) {
                assert_eq!(saw.decode(at)[0], start);
                at += 1;
            }
        }
    }

    // --- All three build configurations agree at every rank ---

    #[test]
    fn all_variants_agree_on_3x3() {
        let nav = Saw::<3, 3>::new(8);
        let pfx = Saw::<3, 3>::builder(8).prefix_depth(4).build();
        let enm = Saw::<3, 3>::builder(8).enumerated().build();
        let both = Saw::<3, 3>::builder(8).prefix_depth(4).enumerated().build();
        assert_eq!(nav.total(), pfx.total());
        assert_eq!(nav.total(), enm.total());
        assert_eq!(nav.total(), both.total());
        for rank in 0..nav.total() {
            let a = nav.decode(rank);
            assert_eq!(a, pfx.decode(rank));
            assert_eq!(a, enm.decode(rank));
            assert_eq!(a, both.decode(rank));
            assert_eq!(pfx.encode(&a), rank);
            assert_eq!(enm.encode(&a), rank);
            assert_eq!(both.encode(&a), rank);
        }
    }

    #[test]
    fn variants_agree_spot_check_4x4() {
        let nav = Saw::<4, 4>::new(6);
        let pfx = Saw::<4, 4>::builder(6).prefix_depth(3).build();
        let total = nav.total();
        let mut r = 0u64;
        while r < total {
            let a = nav.decode(r);
            let b = pfx.decode(r);
            assert_eq!(a, b, "decode mismatch at rank {r}");
            assert_eq!(pfx.encode(&b), r);
            r += 50;
        }
    }

    // --- Timing studies (run with `--ignored --nocapture`) ---

    #[test]
    #[ignore]
    fn timing_8x8_l14() {
        use std::time::Instant;
        let max_length = 14usize;
        let t = Instant::now();
        let nav = Saw::<8, 8>::new(max_length);
        eprintln!(
            "\nSaw<8> L={max_length} nav build: {} ms",
            t.elapsed().as_millis()
        );

        let t = Instant::now();
        let pfx = Saw::<8, 8>::builder(max_length).prefix_depth(6).build();
        eprintln!(
            "Saw<8> L={max_length} prefix_depth(6) build: {} ms ({} MB)",
            t.elapsed().as_millis(),
            pfx.memory_bytes() / 1_048_576,
        );

        let l_start = nav.cum_lengths[max_length];
        let l_total = nav.cum_lengths[max_length + 1] - l_start;
        let sample: u64 = 100;
        let step = (l_total / sample).max(1);

        let t = Instant::now();
        for i in 0..sample {
            let _ = nav.decode(l_start + i * step);
        }
        let nav_us = t.elapsed().as_micros();

        let t = Instant::now();
        for i in 0..sample {
            let _ = pfx.decode(l_start + i * step);
        }
        let pfx_us = t.elapsed().as_micros();

        eprintln!(
            "Nav    decode: {:.2} µs each\nPrefix decode: {:.2} µs each (speedup {:.1}x)",
            nav_us as f64 / sample as f64,
            pfx_us as f64 / sample as f64,
            nav_us as f64 / pfx_us.max(1) as f64,
        );
    }

    #[test]
    #[ignore]
    fn scan_7x7_l18_prefix_depths() {
        use std::time::Instant;
        let max_length = 18usize;
        let nav = Saw::<7, 7>::new(max_length);
        let l_start = nav.cum_lengths[max_length];
        let l_total = nav.cum_lengths[max_length + 1] - l_start;
        let sample: u64 = 30;
        let step = (l_total / sample).max(1);
        let t = Instant::now();
        for i in 0..sample {
            let _ = nav.decode(l_start + i * step);
        }
        eprintln!(
            "\nNav baseline (7×7 L={max_length}): {:.2} µs each",
            t.elapsed().as_micros() as f64 / sample as f64,
        );
        eprintln!(
            "{:>4} {:>10} {:>10} {:>14}",
            "K", "build_ms", "memory_MB", "decode_µs/rank"
        );
        for k in [4, 6, 8, 10] {
            let t = Instant::now();
            let pfx = Saw::<7, 7>::builder(max_length).prefix_depth(k).build();
            let build_ms = t.elapsed().as_millis();
            let mem_mb = pfx.memory_bytes() / 1_048_576;
            let t = Instant::now();
            for i in 0..sample {
                let _ = pfx.decode(l_start + i * step);
            }
            let decode_us = t.elapsed().as_micros() as f64 / sample as f64;
            eprintln!("{k:>4} {build_ms:>10} {mem_mb:>10} {decode_us:>14.2}");
        }
    }

    #[test]
    #[ignore]
    fn max_length_in_60_bits_8x8() {
        // Find the largest L such that the cumulative SAW count on 8×8
        // up to length L fits in 60 bits.
        //
        // One DP pass per start cell yields all lengths up to 63 (the
        // Hamiltonian max on 8×8), so total work is 64 passes — orders of
        // magnitude cheaper than calling `count_saws` per (start, length).
        use crate::saw_dp;
        use std::io::Write;
        use std::time::Instant;
        let max_l = 63usize;
        let budget: u128 = 1u128 << 60;
        let mut per_length: Vec<u128> = vec![0; max_l + 1];
        let t = Instant::now();
        for start in 0..64u8 {
            let s = saw_dp::count_saws_all_lengths(start, max_l, 0);
            for (l, &c) in s.iter().enumerate() {
                per_length[l] += c as u128;
            }
            eprintln!("start={start} done at {:.1}s", t.elapsed().as_secs_f64());
            let _ = std::io::stderr().flush();
        }

        eprintln!(
            "\nAll counts in {:.1}s\n{:>4} {:>22} {:>22} {:>10}",
            t.elapsed().as_secs_f64(),
            "L",
            "count_at_L",
            "cumulative",
            "log2(cum)",
        );
        let mut cum: u128 = 0;
        let mut max_fit: Option<usize> = None;
        for (l, &at_l) in per_length.iter().enumerate() {
            cum += at_l;
            eprintln!(
                "{:>4} {:>22} {:>22} {:>10.3}",
                l,
                at_l,
                cum,
                (cum as f64).log2(),
            );
            if cum <= budget {
                max_fit = Some(l);
            }
        }
        eprintln!("\nMax L that fits in 60 bits: {:?}", max_fit);
    }

    #[test]
    #[ignore]
    fn enumerate_full_space_9x9() {
        // Enumerate every SAW on 9×9 and report cumulative bits per length.
        use crate::saw_dp;
        use std::io::Write;
        use std::time::Instant;
        let max_l = 80usize; // 9*9 - 1 = 80 (Hamiltonian max)
        let mut per_length: Vec<u128> = vec![0; max_l + 1];
        let t = Instant::now();
        for start in 0..81u8 {
            let s = saw_dp::count_saws_for_all_lengths::<9, 9>(start, max_l, 0);
            for (l, &c) in s.iter().enumerate() {
                per_length[l] += c as u128;
            }
            eprintln!("start={start} done at {:.1}s", t.elapsed().as_secs_f64());
            let _ = std::io::stderr().flush();
        }

        eprintln!(
            "\nAll 9×9 counts in {:.1}s\n{:>4} {:>24} {:>26} {:>10}",
            t.elapsed().as_secs_f64(),
            "L",
            "count_at_L",
            "cumulative",
            "log2(cum)",
        );
        let mut cum: u128 = 0;
        for (l, &at_l) in per_length.iter().enumerate() {
            cum += at_l;
            eprintln!(
                "{:>4} {:>24} {:>26} {:>10.3}",
                l,
                at_l,
                cum,
                (cum as f64).log2(),
            );
        }
        eprintln!(
            "\nGrand total 9×9 SAWs: {cum} = 2^{:.3}",
            (cum as f64).log2()
        );
    }

    #[test]
    #[ignore]
    fn enumerate_full_space_8x9() {
        // 8 wide × 9 tall = 72 cells. log2(72) ≈ 6.17 — apple needs 7 bits.
        use crate::saw_dp;
        use std::io::Write;
        use std::time::Instant;
        let max_l = 71usize;
        let mut per_length: Vec<u128> = vec![0; max_l + 1];
        let t = Instant::now();
        for start in 0..72u8 {
            let s = saw_dp::count_saws_for_all_lengths::<8, 9>(start, max_l, 0);
            for (l, &c) in s.iter().enumerate() {
                per_length[l] += c as u128;
            }
            eprintln!("start={start} done at {:.1}s", t.elapsed().as_secs_f64());
            let _ = std::io::stderr().flush();
        }

        eprintln!(
            "\nAll 8×9 counts in {:.1}s\n{:>4} {:>24} {:>26} {:>10}",
            t.elapsed().as_secs_f64(),
            "L",
            "count_at_L",
            "cumulative",
            "log2(cum)",
        );
        let mut cum: u128 = 0;
        for (l, &at_l) in per_length.iter().enumerate() {
            cum += at_l;
            eprintln!(
                "{:>4} {:>24} {:>26} {:>10.3}",
                l,
                at_l,
                cum,
                (cum as f64).log2(),
            );
        }
        eprintln!(
            "\nGrand total 8×9 SAWs: {cum} = 2^{:.3}",
            (cum as f64).log2()
        );
    }

    #[test]
    #[ignore]
    fn enumerated_timing_5x5_l10() {
        use std::time::Instant;
        let max_length = 10usize;
        let nav = Saw::<5, 5>::new(max_length);
        let t = Instant::now();
        let enm = Saw::<5, 5>::builder(max_length).enumerated().build();
        eprintln!(
            "\nSaw<5> L={max_length} enumerated build: {} ms ({} KB)",
            t.elapsed().as_millis(),
            enm.memory_bytes() / 1024,
        );
        let total = nav.total();
        let step = (total / 1000).max(1);

        let t = Instant::now();
        let mut r = 0;
        let mut sampled = 0;
        while r < total {
            let _ = nav.decode(r);
            let _ = nav.encode(&nav.decode(r));
            sampled += 1;
            r += step;
        }
        let nav_us = t.elapsed().as_micros();

        let t = Instant::now();
        let mut r = 0;
        let mut sampled_e = 0;
        while r < total {
            let p = enm.decode(r);
            let _ = enm.encode(&p);
            sampled_e += 1;
            r += step;
        }
        let enm_us = t.elapsed().as_micros();

        eprintln!(
            "Nav        round-trip: {} in {} µs ({:.2} µs each)",
            sampled,
            nav_us,
            nav_us as f64 / sampled as f64,
        );
        eprintln!(
            "Enumerated round-trip: {} in {} µs ({:.3} µs each)",
            sampled_e,
            enm_us,
            enm_us as f64 / sampled_e as f64,
        );
    }
}
