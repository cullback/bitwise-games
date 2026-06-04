use minifb::Key;

/// A bitwise game: state packs into a `u64`, evolved by pure transitions.
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
    /// - `pressed`: keys that newly transitioned to held this tick
    ///   (`held & !held_last_tick`). Read for discrete action (e.g. a puzzle
    ///   tile slides exactly once per press).
    fn update(state: u64, held: &[Key], pressed: &[Key]) -> (u64, Vec<u32>);
}
