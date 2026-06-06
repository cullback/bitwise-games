// Enumerates self-avoiding walks (SAWs) on the 8x8 grid and reports counts by
// length and starting cell. Used to size the body-rank field for a hypothetical
// SAW-rank encoding of the Snake state.
//
// Usage: cargo run --release --bin saw_count -- [max_len] [start_idx]
//   max_len:   maximum walk length to enumerate (default 22)
//   start_idx: index into the canonical (D4 fundamental domain) start list,
//              or "all" / omitted to run every canonical start.
//
// Progress: enumerates all length-PREFIX_LEN prefixes upfront, then DFS-extends
// each. After each prefix completes, prints elapsed time, fraction done, and a
// linear ETA. Per-prefix runtimes vary (prefixes ending near walls finish
// faster), so the ETA is approximate but improves as the run progresses.

use std::env;
use std::time::Instant;

const W: usize = 8;
const N: usize = W * W;
const PREFIX_LEN: usize = 5;

const NEIGHBOR_MASKS: [u64; N] = {
    let mut masks = [0u64; N];
    let mut i = 0;
    while i < N {
        let r = (i / W) as isize;
        let c = (i % W) as isize;
        let mut mask = 0u64;
        if r > 0 {
            mask |= 1u64 << ((r as usize - 1) * W + c as usize);
        }
        if r < (W as isize) - 1 {
            mask |= 1u64 << ((r as usize + 1) * W + c as usize);
        }
        if c > 0 {
            mask |= 1u64 << (r as usize * W + (c as usize - 1));
        }
        if c < (W as isize) - 1 {
            mask |= 1u64 << (r as usize * W + (c as usize + 1));
        }
        masks[i] = mask;
        i += 1;
    }
    masks
};

/// DFS extension from (pos, visited) at depth `len`, accumulating per-length
/// counts up to `max_len`. Counts include the starting (pos, visited) state at
/// counts[len].
fn dfs(pos: usize, visited: u64, counts: &mut [u128], len: usize, max_len: usize) {
    counts[len] += 1;
    if len == max_len {
        return;
    }
    let mut candidates = NEIGHBOR_MASKS[pos] & !visited;
    while candidates != 0 {
        let next = candidates.trailing_zeros() as usize;
        candidates &= candidates - 1;
        dfs(next, visited | (1u64 << next), counts, len + 1, max_len);
    }
}

/// Enumerate every length-`prefix_len` SAW from `start`, returning (end_pos,
/// visited_mask) and accumulating per-length counts for lengths 0..=prefix_len.
fn collect_prefixes(start: usize, prefix_len: usize, counts: &mut [u128]) -> Vec<(usize, u64)> {
    let mut prefixes = Vec::new();
    fn go(
        pos: usize,
        visited: u64,
        len: usize,
        prefix_len: usize,
        counts: &mut [u128],
        prefixes: &mut Vec<(usize, u64)>,
    ) {
        counts[len] += 1;
        if len == prefix_len {
            prefixes.push((pos, visited));
            return;
        }
        let mut candidates = NEIGHBOR_MASKS[pos] & !visited;
        while candidates != 0 {
            let next = candidates.trailing_zeros() as usize;
            candidates &= candidates - 1;
            go(
                next,
                visited | (1u64 << next),
                len + 1,
                prefix_len,
                counts,
                prefixes,
            );
        }
    }
    go(start, 1u64 << start, 0, prefix_len, counts, &mut prefixes);
    prefixes
}

fn count_from(start: usize, max_len: usize) -> Vec<u128> {
    let mut counts = vec![0u128; max_len + 1];

    if max_len <= PREFIX_LEN {
        dfs(start, 1u64 << start, &mut counts, 0, max_len);
        return counts;
    }

    let prefixes = collect_prefixes(start, PREFIX_LEN, &mut counts);
    let total = prefixes.len();
    eprintln!("  {total} length-{PREFIX_LEN} prefixes; extending each to length {max_len}");

    let t0 = Instant::now();
    // Track per-length contribution from extension only (excludes the prefix-
    // length contributions we already accumulated in collect_prefixes).
    let mut ext_counts = vec![0u128; max_len + 1];
    let log_every = (total / 20).max(1);
    for (i, &(pos, visited)) in prefixes.iter().enumerate() {
        // Extend this prefix: counts at depth >PREFIX_LEN
        let mut sub = vec![0u128; max_len + 1];
        dfs(pos, visited, &mut sub, PREFIX_LEN, max_len);
        // sub[PREFIX_LEN] = 1 for the prefix itself (already counted above);
        // skip it and accumulate the rest.
        for l in (PREFIX_LEN + 1)..=max_len {
            ext_counts[l] += sub[l];
        }

        if (i + 1) % log_every == 0 || i + 1 == total {
            let elapsed = t0.elapsed().as_secs_f64();
            let frac = (i + 1) as f64 / total as f64;
            let est_total = elapsed / frac;
            let est_remain = est_total - elapsed;
            eprintln!(
                "  [{}/{}] {:.1}% — {:.1}s elapsed, ~{:.1}s remaining (~{:.1}s total)",
                i + 1,
                total,
                frac * 100.0,
                elapsed,
                est_remain,
                est_total,
            );
        }
    }

    for l in (PREFIX_LEN + 1)..=max_len {
        counts[l] += ext_counts[l];
    }
    counts
}

const CANONICAL: &[(usize, usize)] = &[
    (0, 0),
    (0, 1),
    (0, 2),
    (0, 3),
    (1, 1),
    (1, 2),
    (1, 3),
    (2, 2),
    (2, 3),
    (3, 3),
];

fn run_start(r: usize, c: usize, max_len: usize) {
    let start = r * W + c;
    eprintln!("# start ({r},{c}) — enumerating up to length {max_len}");
    let t0 = Instant::now();
    let counts = count_from(start, max_len);
    let elapsed = t0.elapsed();
    eprintln!("#   done in {:.2}s", elapsed.as_secs_f64());

    println!("# start=({r},{c})");
    println!("# len  c_L                              cum_L                           log2(cum_L)");
    let mut cum: u128 = 0;
    for (l, &c_l) in counts.iter().enumerate() {
        cum += c_l;
        let log2_cum = if cum > 0 { (cum as f64).log2() } else { 0.0 };
        println!("  {l:3}  {c_l:>30}  {cum:>30}  {log2_cum:>6.2}");
    }
    println!();
}

fn main() {
    let mut args = env::args().skip(1);
    let max_len: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(22);
    let start_arg = args.next();

    match start_arg.as_deref() {
        None | Some("all") => {
            for &(r, c) in CANONICAL {
                run_start(r, c, max_len);
            }
        }
        Some(s) => {
            let idx: usize = s.parse().expect("start_idx must be an integer or \"all\"");
            let (r, c) = CANONICAL[idx];
            run_start(r, c, max_len);
        }
    }
}
