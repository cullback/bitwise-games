use crate::frame_buffer::FrameBuffer;

/// The six keys wired through to games: arrows, Z, X.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Z,
    X,
}

/// A bitwise game: state packs into a `u64`, evolved by pure transitions.
///
/// Output is a fixed 128×128 framebuffer over a 16-color palette. Input is
/// restricted to 6 keys: arrows + Z + X.
pub trait Game {
    const NAME: &'static str;
    const FPS: usize;

    /// Initial state and framebuffer. Pure — any nondeterminism enters via `args`.
    fn new(args: Vec<String>) -> (u64, FrameBuffer);

    /// One tick. Pure.
    ///
    /// - `held`: keys held during this tick. Read for continuous action
    ///   (e.g. a paddle slides while the arrow is down).
    /// - `buffered`: at most one queued press event. The framework records
    ///   rising edges and dequeues one per tick, so fast multi-key inputs
    ///   don't get lost. Read for discrete actions (puzzle tile slides,
    ///   snake turns) — successive taps queue up and play out one per tick.
    /// - `mouse`: latest cursor position in framebuffer pixels (0..128),
    ///   or `None` if the cursor is off-canvas. Use for hover / pointing
    ///   games; key-only games can ignore it.
    fn update(
        state: u64,
        held: &[Key],
        buffered: Option<Key>,
        mouse: Option<(u8, u8)>,
    ) -> (u64, FrameBuffer);
}
