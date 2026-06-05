//! Single-step pseudo-random number generator (splitmix64).
//!
//! `next(state)` is a pure function returning a `u64` whose bits are mixed
//! well enough for game state. The intended use is as a **hash**, not a
//! stream PRNG: feed it arbitrary state (a board, a position, a seed mixed
//! with something) and get a well-diffused `u64` back. It also works as a
//! stream PRNG if you do `state = next(state)` repeatedly, but the typical
//! call site looks like `let r = next(board_state);` — derive entropy from
//! whatever's already in the game's state.
//!
//! Splitmix64 is chosen over wyrand (what `fastrand` uses) precisely
//! because of this use case. Wyrand's step is `state += CONST; mix(state)`,
//! which assumes you've been advancing from a known seed; calling it on an
//! arbitrary input still diffuses, but it's a touch less principled as a
//! one-shot hash. Splitmix64's output IS the next state — pure
//! diffusion in three multiplies, no linear counter assumption.

/// One step of splitmix64. Pure function.
///
/// ```
/// use bitwise_games::rng::next;
/// assert_eq!(next(0), 0xe220_a839_7b1d_cdaf);
/// // Same input always gives same output:
/// assert_eq!(next(42), next(42));
/// // Adjacent inputs land far apart:
/// assert_ne!(next(0), next(1));
/// ```
pub fn next(state: u64) -> u64 {
    let mut z = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector_for_zero() {
        // Reference output for splitmix64 from seed 0
        // (Vigna's original implementation).
        assert_eq!(next(0), 0xe220_a839_7b1d_cdaf);
    }

    #[test]
    fn deterministic() {
        for x in [0u64, 1, 42, u64::MAX, 0xdead_beef_cafe_babe] {
            assert_eq!(next(x), next(x));
        }
    }

    #[test]
    fn single_bit_inputs_dont_collide() {
        // Single-bit-flip inputs should mix to distinct outputs.
        let mut outputs: Vec<u64> = (0..64).map(|i| next(1u64 << i)).collect();
        outputs.push(next(0));
        outputs.sort();
        outputs.dedup();
        assert_eq!(outputs.len(), 65);
    }

    #[test]
    fn iterating_yields_a_long_cycle() {
        // Walk 100 steps from seed 1 and check no early repeat.
        let mut s = 1u64;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            s = next(s);
            assert!(seen.insert(s));
        }
    }
}
