use minifb::Key;

/// A bitwise game: state packs into a `u64`, evolved by pure transitions.
///
/// Input is restricted to 6 keys: arrows + Z + X. Anything else is filtered
/// out by the framework.
pub trait Game {
    const NAME: &'static str;
    const FPS: usize;
    const WIDTH: usize;
    const HEIGHT: usize;

    /// Initial state and framebuffer. Pure — any nondeterminism enters via `args`.
    fn new(args: Vec<String>) -> (u64, Vec<u32>);

    /// One tick. Pure.
    ///
    /// - `held`: keys held during this tick. Read for continuous action
    ///   (e.g. a paddle slides while the arrow is down).
    /// - `buffered`: an at-most-one-element slice. The framework queues press
    ///   events (rising edges) and dequeues one per tick, so fast multi-key
    ///   inputs don't get lost. Read for discrete actions (puzzle tile slides,
    ///   snake turns) — successive taps queue up and play out one per tick.
    fn update(state: u64, held: &[Key], buffered: &[Key]) -> (u64, Vec<u32>);
}
