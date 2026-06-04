//! Bijection between integers and lexicographically-ordered multiset permutations.
//!
//! Given a multiset of symbols — e.g. `b"ppppppppnnbbrrqk"` (8 pawns, 2 knights,
//! 2 bishops, 2 rooks, 1 queen, 1 king) — there are `n! / (c1! * c2! * ... * ck!)`
//! distinct arrangements. Each one has a unique rank in `0..count` under
//! lexicographic ordering, and this module converts in both directions.
//!
//! Counts are computed as a product of binomial coefficients with `u128`
//! intermediates, so the result is exact whenever it fits in `u64` —
//! `n!` never appears directly. Past `u64::MAX` the count saturates and
//! the rank/unrank functions stop being meaningful.

use std::collections::BTreeMap;

/// Lexicographic rank of `pieces` among all distinct arrangements of the same
/// multiset.
///
/// ```
/// use bitwise_games::permutation::from_permutation;
/// assert_eq!(from_permutation(b"aab"), 0);
/// assert_eq!(from_permutation(b"aba"), 1);
/// assert_eq!(from_permutation(b"baa"), 2);
/// ```
pub fn from_permutation<T: Ord + Copy>(pieces: &[T]) -> u64 {
    let mut counts = histogram(pieces);
    let mut rank: u64 = 0;
    for p in pieces {
        let lesser: Vec<T> = counts.range(..*p).map(|(k, _)| *k).collect();
        for s in lesser {
            if counts[&s] == 0 {
                continue;
            }
            *counts.get_mut(&s).unwrap() -= 1;
            rank += multiset_count(&counts);
            *counts.get_mut(&s).unwrap() += 1;
        }
        *counts.get_mut(p).unwrap() -= 1;
    }
    rank
}

/// Inverse of [`from_permutation`]: the `rank`-th lexicographic arrangement of
/// the multiset `symbols`. Only the multiplicities of `symbols` matter — its
/// order is ignored.
///
/// ```
/// use bitwise_games::permutation::to_permutation;
/// assert_eq!(to_permutation(0, b"aab"), b"aab");
/// assert_eq!(to_permutation(1, b"aab"), b"aba");
/// assert_eq!(to_permutation(2, b"aab"), b"baa");
/// ```
pub fn to_permutation<T: Ord + Copy>(mut rank: u64, symbols: &[T]) -> Vec<T> {
    let mut counts = histogram(symbols);
    let n = symbols.len();
    let mut result = Vec::with_capacity(n);
    for _ in 0..n {
        let alphabet: Vec<T> = counts.keys().copied().collect();
        for s in alphabet {
            if counts[&s] == 0 {
                continue;
            }
            *counts.get_mut(&s).unwrap() -= 1;
            let c = multiset_count(&counts);
            if rank < c {
                result.push(s);
                break;
            }
            rank -= c;
            *counts.get_mut(&s).unwrap() += 1;
        }
    }
    result
}

/// Number of distinct arrangements of a multiset with these symbol counts.
///
/// Computed as `C(c1, c1) * C(c1+c2, c2) * C(c1+c2+c3, c3) * ...` using `u128`
/// intermediates. Saturates at `u64::MAX` if the true count overflows.
pub fn multiset_count<T: Ord>(counts: &BTreeMap<T, u32>) -> u64 {
    let mut total: u64 = 0;
    let mut result: u128 = 1;
    for &c in counts.values() {
        if c == 0 {
            continue;
        }
        let c = c as u64;
        total += c;
        // result *= C(total, c), keeping integer arithmetic exact
        let k = c.min(total - c);
        for i in 0..k {
            result = result * (total - i) as u128 / (i + 1) as u128;
        }
    }
    u64::try_from(result).unwrap_or(u64::MAX)
}

fn histogram<T: Ord + Copy>(items: &[T]) -> BTreeMap<T, u32> {
    let mut map = BTreeMap::new();
    for x in items {
        *map.entry(*x).or_insert(0) += 1;
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_chess_pieces() {
        let symbols = b"ppppppppnnbbrrqk";
        let total = multiset_count(&histogram(symbols));
        for &i in &[0u64, 1, 42, 1000, 1_000_000, total - 1] {
            let p = to_permutation(i, symbols);
            assert_eq!(from_permutation(&p), i, "rank {i} did not round-trip");
        }
    }

    #[test]
    fn lex_order_is_monotonic() {
        let symbols = b"aabb";
        let count = multiset_count(&histogram(symbols));
        let mut prev: Vec<u8> = Vec::new();
        for i in 0..count {
            let p = to_permutation(i, symbols);
            if i > 0 {
                assert!(p > prev, "not in lex order at i={i}: {p:?} vs {prev:?}");
            }
            prev = p;
        }
    }

    #[test]
    fn count_matches_multinomial() {
        // 4!/(2! 2!) = 6
        assert_eq!(multiset_count(&histogram(b"aabb")), 6);
        // 5!/(3! 2!) = 10
        assert_eq!(multiset_count(&histogram(b"aaabb")), 10);
        // 16!/(8! 2! 2! 2! 1! 1!) = 64_864_800
        assert_eq!(multiset_count(&histogram(b"ppppppppnnbbrrqk")), 64_864_800);
    }

    #[test]
    fn empty_inputs() {
        assert_eq!(from_permutation::<u8>(&[]), 0);
        assert_eq!(to_permutation::<u8>(0, &[]), Vec::<u8>::new());
    }

    #[test]
    fn single_symbol_one_arrangement() {
        assert_eq!(from_permutation(b"aaaa"), 0);
        assert_eq!(to_permutation(0, b"aaaa"), b"aaaa");
    }

    #[test]
    fn handles_lengths_past_factorial_overflow() {
        // 22 items in 11 pairs — n! overflows u64, but the multinomial fits.
        // 22! / (2!)^11 = 548_828_480_360_160_000 ≈ 5.5e17, fits in u64.
        let symbols: Vec<u8> = (b'a'..=b'k').flat_map(|c| [c, c]).collect();
        let count = multiset_count(&histogram(&symbols));
        assert_eq!(count, 548_828_480_360_160_000);
        for &i in &[0u64, 1, 12345, count - 1] {
            let p = to_permutation(i, &symbols);
            assert_eq!(from_permutation(&p), i);
        }
    }
}
