//! Bijection between integers and variable-length sequences over a fixed
//! alphabet `{0, 1, ..., base-1}`.
//!
//! All sequences of length `0..=max_len` are ordered: first by length, then
//! lexicographically within each length (least-significant digit first).
//! Each one gets a unique integer in `0..count`, where
//! `count = (base^(max_len+1) − 1) / (base − 1)`.
//!
//! Useful when you need to pack a structure whose length itself is part of
//! the state — encoding the length inside the integer (via the "subtract the
//! cumulative count of shorter lengths, then base-N decode the remainder"
//! trick) is denser than a separate length field or a sentinel terminator.
//!
//! Internal arithmetic uses `u128`, so any result that fits in `u64` is
//! computed exactly. Values that would overflow saturate at `u64::MAX`.

/// Lexicographic rank of `digits` among all variable-length sequences over an
/// alphabet of size `base`.
///
/// Each digit must be `< base`. Sequences are ordered first by length, then
/// base-N within each length.
///
/// ```
/// use bitwise_games::varlen::from_varlen;
/// assert_eq!(from_varlen(&[], 3), 0);
/// assert_eq!(from_varlen(&[0], 3), 1);
/// assert_eq!(from_varlen(&[1], 3), 2);
/// assert_eq!(from_varlen(&[2], 3), 3);
/// assert_eq!(from_varlen(&[0, 0], 3), 4);
/// ```
pub fn from_varlen(digits: &[u8], base: u32) -> u64 {
    assert!(base >= 1, "base must be >= 1");
    let n = base as u128;
    let len = digits.len() as u32;

    let cumulative: u128 = cumulative_count(n, len);
    let mut offset: u128 = 0;
    let mut place: u128 = 1;
    for &d in digits {
        debug_assert!((d as u32) < base, "digit {d} >= base {base}");
        offset += d as u128 * place;
        place = place.saturating_mul(n);
    }

    u64::try_from(cumulative + offset).unwrap_or(u64::MAX)
}

/// Inverse of [`from_varlen`]: the `rank`-th variable-length sequence over an
/// alphabet of size `base`.
///
/// ```
/// use bitwise_games::varlen::to_varlen;
/// assert_eq!(to_varlen(0, 3), vec![]);
/// assert_eq!(to_varlen(1, 3), vec![0]);
/// assert_eq!(to_varlen(4, 3), vec![0, 0]);
/// assert_eq!(to_varlen(12, 3), vec![2, 2]);
/// assert_eq!(to_varlen(13, 3), vec![0, 0, 0]);
/// ```
pub fn to_varlen(rank: u64, base: u32) -> Vec<u8> {
    assert!(base >= 1, "base must be >= 1");
    let n = base as u128;
    let mut v = rank as u128;

    // Walk lengths until v falls inside the current length's chunk.
    let mut len: u32 = 0;
    let mut chunk: u128 = 1; // base^len at len = 0
    while chunk <= v {
        v -= chunk;
        len += 1;
        chunk = chunk.saturating_mul(n);
    }

    let mut digits = Vec::with_capacity(len as usize);
    for _ in 0..len {
        digits.push((v % n) as u8);
        v /= n;
    }
    digits
}

/// Number of distinct sequences of length `0..=max_len` over an alphabet of
/// size `base`. Saturates at `u64::MAX`.
pub fn varlen_count(base: u32, max_len: u32) -> u64 {
    assert!(base >= 1, "base must be >= 1");
    let n = base as u128;
    u64::try_from(cumulative_count(n, max_len + 1)).unwrap_or(u64::MAX)
}

/// Sum of `base^0 + base^1 + ... + base^(len-1)`.
fn cumulative_count(base: u128, len: u32) -> u128 {
    if base == 1 {
        return len as u128;
    }
    // Geometric series: (base^len - 1) / (base - 1)
    let mut power: u128 = 1;
    for _ in 0..len {
        power = power.saturating_mul(base);
    }
    (power - 1) / (base - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worked_example_base_3() {
        // From the docstring walk-through.
        let expected: &[(u64, &[u8])] = &[
            (0, &[]),
            (1, &[0]),
            (2, &[1]),
            (3, &[2]),
            (4, &[0, 0]),
            (5, &[1, 0]),
            (6, &[2, 0]),
            (7, &[0, 1]),
            (8, &[1, 1]),
            (9, &[2, 1]),
            (10, &[0, 2]),
            (11, &[1, 2]),
            (12, &[2, 2]),
            (13, &[0, 0, 0]),
        ];
        for (rank, digits) in expected {
            assert_eq!(&to_varlen(*rank, 3), digits, "decode {rank}");
            assert_eq!(from_varlen(digits, 3), *rank, "encode {digits:?}");
        }
    }

    #[test]
    fn round_trip_dense() {
        for base in [2u32, 3, 4, 10] {
            let count = varlen_count(base, 6);
            for rank in 0..count {
                let digits = to_varlen(rank, base);
                assert_eq!(
                    from_varlen(&digits, base),
                    rank,
                    "round-trip failed for base={base} rank={rank}"
                );
            }
        }
    }

    #[test]
    fn count_matches_geometric_series() {
        // base=3, max_len=4: 1 + 3 + 9 + 27 + 81 = 121
        assert_eq!(varlen_count(3, 4), 121);
        // base=2, max_len=10: 2^11 - 1 = 2047
        assert_eq!(varlen_count(2, 10), 2047);
        // base=10, max_len=5: 111111
        assert_eq!(varlen_count(10, 5), 111_111);
    }

    #[test]
    fn base_one_is_just_length() {
        // Alphabet has one symbol (0). The "sequence" is just its length.
        for len in 0..20u32 {
            let digits = vec![0u8; len as usize];
            assert_eq!(from_varlen(&digits, 1), len as u64);
            assert_eq!(to_varlen(len as u64, 1), digits);
        }
        assert_eq!(varlen_count(1, 20), 21);
    }

    #[test]
    fn ranks_grouped_by_length() {
        // The first count(base, L-1) ranks decode to sequences of length <= L,
        // and exactly varlen_count(base, L) - varlen_count(base, L-1) of them
        // have length == L.
        let base = 4u32;
        for len in 1..6u32 {
            let count_at = varlen_count(base, len);
            let count_below = varlen_count(base, len - 1);
            for rank in count_below..count_at {
                assert_eq!(to_varlen(rank, base).len() as u32, len);
            }
        }
    }
}
